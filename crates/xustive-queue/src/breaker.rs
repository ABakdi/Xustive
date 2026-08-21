//! A **shared** circuit breaker, coordinated through Redis ([[Error Handling and Resilience]],
//! M4-T02.2). Where [`xustive_core::circuit::SharedBreaker`] trips one process, this trips every
//! process that points at the same dependency — so a fleet of API instances all fail fast together
//! when Meilisearch (or any shared dependency) is down, instead of each discovering it separately
//! and each paying the timeout.
//!
//! # Why the transitions are Lua
//!
//! `allow` and `on_failure` are read-modify-write across several keys. Between instances that races,
//! and a raced breaker under-counts failures (never trips) or hands out several half-open probes at
//! once (defeats the point). Each transition therefore runs as one atomic server-side script.
//!
//! The state machine matches the in-process breaker: Closed → Open at `failure_threshold` failures
//! within a window → Half-open after the (exponentially backed-off, capped) cooldown → Closed on a
//! successful probe, or Open-longer on a failed one. Only one instance's probe is let through, via a
//! `SET NX` lock.
//!
//! Time comes from the caller (`SystemTime`), so hosts must be roughly NTP-synced — trivially true
//! in practice, and the skew that matters (sub-second) is negligible against cooldowns in seconds.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use redis::Script;

use crate::QueueError;

/// Tuning, mirroring [`xustive_core::circuit::Config`] but in the units the scripts use.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub failure_threshold: u32,
    pub cooldown: Duration,
    pub max_cooldown: Duration,
    /// How long a run of failures accumulates before the count resets (the "consecutive within a
    /// window" window).
    pub window: Duration,
    /// How long a single half-open probe holds its exclusive lock.
    pub probe_ttl: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            cooldown: Duration::from_secs(2),
            max_cooldown: Duration::from_secs(60),
            window: Duration::from_secs(30),
            probe_ttl: Duration::from_secs(5),
        }
    }
}

/// A named breaker backed by shared Redis state.
#[derive(Clone)]
pub struct RedisBreaker {
    manager: redis::aio::ConnectionManager,
    namespace: String,
    config: Config,
}

impl RedisBreaker {
    /// Connect within a namespace. `None` if Redis is unreachable — a breaker whose store is down is
    /// useless, and the caller should treat "no breaker" as "always allow" rather than fail closed.
    pub async fn connect_in(url: &str, namespace: &str, config: Config) -> Option<Self> {
        let client = redis::Client::open(url).ok()?;
        let manager = client.get_connection_manager().await.ok()?;
        Some(Self {
            manager,
            namespace: namespace.to_string(),
            config,
        })
    }

    fn hash(&self, name: &str) -> String {
        format!("{}:cb:{name}", self.namespace)
    }
    fn fails(&self, name: &str) -> String {
        format!("{}:cb:{name}:fails", self.namespace)
    }
    fn probe(&self, name: &str) -> String {
        format!("{}:cb:{name}:probe", self.namespace)
    }

    fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    /// Whether a request may proceed. Advances Open→Half-open when the cooldown has elapsed and hands
    /// exactly one probe (fleet-wide) through. On a Redis error, returns `true` (fail open): a broken
    /// breaker must not become the outage.
    pub async fn allow(&self, name: &str) -> bool {
        let mut conn = self.manager.clone();
        let out: Result<i64, _> = ALLOW
            .key(self.hash(name))
            .key(self.probe(name))
            .arg(Self::now_ms())
            .arg(self.config.probe_ttl.as_millis() as i64)
            .invoke_async(&mut conn)
            .await;
        out.map(|v| v == 1).unwrap_or(true)
    }

    /// Record a success: close the breaker and clear the backoff.
    pub async fn on_success(&self, name: &str) {
        let mut conn = self.manager.clone();
        let _: Result<(), _> = redis::cmd("DEL")
            .arg(self.hash(name))
            .arg(self.fails(name))
            .arg(self.probe(name))
            .query_async::<()>(&mut conn)
            .await;
    }

    /// Record a failure: count it (Closed), or reopen with a longer cooldown (a failed probe).
    pub async fn on_failure(&self, name: &str) {
        let mut conn = self.manager.clone();
        let _: Result<i64, _> = ON_FAILURE
            .key(self.hash(name))
            .key(self.fails(name))
            .key(self.probe(name))
            .arg(Self::now_ms())
            .arg(self.config.failure_threshold as i64)
            .arg(self.config.cooldown.as_millis() as i64)
            .arg(self.config.max_cooldown.as_millis() as i64)
            .arg(self.config.window.as_secs() as i64)
            .invoke_async(&mut conn)
            .await;
    }

    /// `"closed"`, `"open"`, or `"half-open"`, for the admin console. A read-only best-effort view.
    pub async fn state(&self, name: &str) -> Result<&'static str, QueueError> {
        let mut conn = self.manager.clone();
        let open_until: Option<i64> = redis::cmd("HGET")
            .arg(self.hash(name))
            .arg("open_until_ms")
            .query_async(&mut conn)
            .await?;
        Ok(match open_until {
            None | Some(0) => "closed",
            Some(t) if Self::now_ms() < t => "open",
            Some(_) => "half-open",
        })
    }
}

// Closed → 1 (allow). Open before cooldown → 0. At/after cooldown → one probe via SET NX.
static ALLOW: std::sync::LazyLock<Script> = std::sync::LazyLock::new(|| {
    Script::new(
        r#"
        local open_until = tonumber(redis.call('HGET', KEYS[1], 'open_until_ms') or '0')
        if open_until == 0 then return 1 end
        if tonumber(ARGV[1]) < open_until then return 0 end
        if redis.call('SET', KEYS[2], '1', 'NX', 'PX', tonumber(ARGV[2])) then return 1 else return 0 end
        "#,
    )
});

// Count failures (Closed) and trip at the threshold; a failure at/after cooldown (failed probe)
// reopens with a doubled, capped cooldown.
static ON_FAILURE: std::sync::LazyLock<Script> = std::sync::LazyLock::new(|| {
    Script::new(
        r#"
        local h, failsk, probek = KEYS[1], KEYS[2], KEYS[3]
        local now = tonumber(ARGV[1])
        local threshold = tonumber(ARGV[2])
        local base = tonumber(ARGV[3])
        local maxcd = tonumber(ARGV[4])
        local window = tonumber(ARGV[5])
        local function reopen()
          local opens = tonumber(redis.call('HGET', h, 'opens') or '0') + 1
          local shift = math.min(opens - 1, 20)
          local cd = math.min(base * (2 ^ shift), maxcd)
          redis.call('HSET', h, 'opens', opens, 'open_until_ms', now + cd)
        end
        local open_until = tonumber(redis.call('HGET', h, 'open_until_ms') or '0')
        if open_until > 0 then
          if now >= open_until then reopen(); redis.call('DEL', probek) end
          return 1
        end
        local fails = redis.call('INCR', failsk)
        redis.call('EXPIRE', failsk, window)
        if fails >= threshold then
          reopen()
          redis.call('DEL', failsk)
        end
        return 1
        "#,
    )
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_is_sane() {
        let c = Config::default();
        assert!(c.failure_threshold >= 1);
        assert!(c.cooldown <= c.max_cooldown);
    }
}
