//! Shared circuit breakers ([[Proxy Manager]] §4.7).
//!
//! When a host, platform, or ASN is failing, every crawler replica must back off it — not each on
//! its own clock, or between them they keep hammering a target one of them already knows is down.
//! So the breaker state lives in Redis, keyed by scope, and all replicas read the same open-until
//! time.
//!
//! The cooldown **doubles** on each successive trip, to a ceiling: a host that flaps back to failing
//! the moment the breaker closes gets progressively longer rests rather than a fixed one. Host
//! breakers start at 60 s; platform breakers start at 15 min, because a platform-level block clears
//! far more slowly than one flaky host, and probing it early just burns identities.

use std::time::Duration;

/// What a breaker protects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Host(String),
    Platform(String),
    Asn(String),
}

impl Scope {
    fn key(&self) -> String {
        match self {
            Scope::Host(h) => format!("breaker:{h}"),
            Scope::Platform(p) => format!("breaker:platform:{p}"),
            Scope::Asn(n) => format!("breaker:asn:{n}"),
        }
    }

    /// The first cooldown for this scope. Platform starts high; host and ASN start at the base.
    fn base_cooldown(&self) -> Duration {
        match self {
            Scope::Platform(_) => PLATFORM_BASE,
            _ => HOST_BASE,
        }
    }
}

const HOST_BASE: Duration = Duration::from_secs(60);
const PLATFORM_BASE: Duration = Duration::from_secs(900);
const CEILING: Duration = Duration::from_secs(1_800);

/// The cooldown for a given trip level (0-based), doubling from `base` to `CEILING`. Pure, so the
/// backoff schedule is testable without a clock or Redis.
pub fn cooldown_for(base: Duration, level: u32) -> Duration {
    // Saturating shift so a large level cannot overflow; capped at the ceiling regardless.
    let secs = base
        .as_secs()
        .saturating_mul(1u64.checked_shl(level).unwrap_or(u64::MAX));
    Duration::from_secs(secs.min(CEILING.as_secs()))
}

/// Redis-backed breaker store. Each scope is a hash: `until_ms` (when it reopens) and `level` (how
/// many times it has tripped, for the doubling).
#[derive(Clone)]
pub struct Breakers {
    client: redis::Client,
    namespace: String,
}

impl Breakers {
    pub fn connect_in(url: &str, namespace: &str) -> Option<Self> {
        Some(Self {
            client: redis::Client::open(url).ok()?,
            namespace: namespace.to_string(),
        })
    }

    async fn conn(&self) -> Option<redis::aio::MultiplexedConnection> {
        self.client.get_multiplexed_async_connection().await.ok()
    }

    fn key(&self, scope: &Scope) -> String {
        format!("{}:{}", self.namespace, scope.key())
    }

    /// Trip the breaker for `scope` at time `now_ms`. Increments the level and sets the reopen time
    /// to `now + cooldown(level)`. Returns the cooldown applied, so the caller can log/alert. A dead
    /// Redis degrades to per-replica behaviour (documented, §7) — reported as a zero cooldown.
    pub async fn trip(&self, scope: &Scope, now_ms: i64) -> Duration {
        let Some(mut conn) = self.conn().await else {
            return Duration::ZERO;
        };
        let key = self.key(scope);
        // The level *after* this trip: HINCRBY returns the new value, so a first trip yields 1 and
        // the cooldown for level 0.
        let new_level: i64 = redis::cmd("HINCRBY")
            .arg(&key)
            .arg("level")
            .arg(1)
            .query_async(&mut conn)
            .await
            .unwrap_or(1);
        let level = (new_level - 1).max(0) as u32;
        let cooldown = cooldown_for(scope.base_cooldown(), level);
        let until = now_ms + cooldown.as_millis() as i64;
        // Expire the key a little after it reopens, so a scope that stops failing forgets its level
        // and starts fresh next time rather than carrying a long cooldown forever.
        let ttl = cooldown.as_secs().max(1) * 2;
        let _: Result<(), _> = redis::pipe()
            .cmd("HSET")
            .arg(&key)
            .arg("until_ms")
            .arg(until)
            .ignore()
            .cmd("EXPIRE")
            .arg(&key)
            .arg(ttl)
            .ignore()
            .query_async::<()>(&mut conn)
            .await;
        cooldown
    }

    /// Whether the breaker for `scope` is currently open (backing off) at `now_ms`.
    pub async fn is_open(&self, scope: &Scope, now_ms: i64) -> bool {
        let Some(mut conn) = self.conn().await else {
            return false; // Fail open: a breaker we cannot read does not block a fetch.
        };
        let until: Option<i64> = redis::cmd("HGET")
            .arg(self.key(scope))
            .arg("until_ms")
            .query_async(&mut conn)
            .await
            .ok()
            .flatten();
        until.is_some_and(|u| u > now_ms)
    }

    /// Close the breaker on a clean success, forgetting its level so the next trip starts from the
    /// base cooldown again.
    pub async fn reset(&self, scope: &Scope) {
        let Some(mut conn) = self.conn().await else {
            return;
        };
        let _: Result<(), _> = redis::cmd("DEL")
            .arg(self.key(scope))
            .query_async::<()>(&mut conn)
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooldown_doubles_from_the_base_to_the_ceiling() {
        assert_eq!(cooldown_for(HOST_BASE, 0), Duration::from_secs(60));
        assert_eq!(cooldown_for(HOST_BASE, 1), Duration::from_secs(120));
        assert_eq!(cooldown_for(HOST_BASE, 2), Duration::from_secs(240));
        // Capped at 30 minutes however high the level.
        assert_eq!(cooldown_for(HOST_BASE, 10), CEILING);
        assert_eq!(cooldown_for(HOST_BASE, 1000), CEILING);
    }

    #[test]
    fn platform_breakers_start_higher() {
        assert_eq!(cooldown_for(PLATFORM_BASE, 0), Duration::from_secs(900));
        // 15min → 30min → capped.
        assert_eq!(cooldown_for(PLATFORM_BASE, 1), CEILING);
    }

    #[test]
    fn scope_keys_are_distinct_per_kind() {
        assert_eq!(Scope::Host("a.dz".into()).key(), "breaker:a.dz");
        assert_eq!(
            Scope::Platform("instagram".into()).key(),
            "breaker:platform:instagram"
        );
        assert_eq!(Scope::Asn("AS36947".into()).key(), "breaker:asn:AS36947");
    }

    #[tokio::test]
    async fn a_dead_redis_fails_open() {
        let b = Breakers::connect_in("redis://127.0.0.1:1", "test").unwrap();
        // Cannot read → not open (a fetch is allowed), and tripping is a silent no-op.
        assert!(!b.is_open(&Scope::Host("x.dz".into()), 0).await);
        assert_eq!(b.trip(&Scope::Host("x.dz".into()), 0).await, Duration::ZERO);
    }
}
