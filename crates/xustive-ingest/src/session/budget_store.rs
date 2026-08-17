//! Fail-closed per-identity budget accounting in Redis ([[Session Manager]] §4.5, §7).
//!
//! Budgets are per identity, and the counters are shared so every crawler replica agrees. The rule
//! that matters is the failure mode: **when the budget cannot be confirmed, deny.** After a Redis
//! restart, assuming zero usage would let every identity burn its full daily allowance at once, so
//! uncertainty must fail closed, not open.
//!
//! Two uncertainties are handled:
//!
//! - **Redis unreachable.** A read or write that errors denies the spend outright.
//! - **Redis flushed.** A counter simply being absent is ambiguous — it looks the same whether the
//!   period is fresh or the data was wiped. So the store keeps a durable **sentinel** key, set once
//!   at start-up; if it is ever missing while the process runs, the data was lost, and every spend
//!   is denied until an operator deliberately re-initialises. Recovery is a decision, not an
//!   accident.
//!
//! Counters are per-period keys (`…:h:<hour-bucket>`, `…:d:<day-bucket>`) with a TTL a little past
//! the period, so a new hour or day is a new key that naturally starts at zero — the correct reset,
//! distinct from the flush case above.

/// The decision for one spend attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Within budget; the request may proceed. Carries the hourly allowance still remaining after.
    Allow { remaining_hour: u32 },
    /// The identity has used its hourly or daily allowance.
    DenyOverBudget,
    /// The budget could not be confirmed — Redis is unreachable or was flushed. **Fail closed.**
    DenyUnavailable,
}

impl Decision {
    pub fn is_allowed(self) -> bool {
        matches!(self, Decision::Allow { .. })
    }
}

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 86_400_000;

/// Redis-backed budget counters with the fail-closed guarantees above.
#[derive(Clone)]
pub struct BudgetStore {
    client: redis::Client,
    namespace: String,
}

impl BudgetStore {
    pub fn connect_in(url: &str, namespace: &str) -> Option<Self> {
        Some(Self {
            client: redis::Client::open(url).ok()?,
            namespace: namespace.to_string(),
        })
    }

    async fn conn(&self) -> Option<redis::aio::MultiplexedConnection> {
        self.client.get_multiplexed_async_connection().await.ok()
    }

    fn sentinel_key(&self) -> String {
        format!("{}:budget:alive", self.namespace)
    }

    fn hour_key(&self, id: &str, now_ms: i64) -> String {
        format!("{}:budget:{id}:h:{}", self.namespace, now_ms / HOUR_MS)
    }

    fn day_key(&self, id: &str, now_ms: i64) -> String {
        format!("{}:budget:{id}:d:{}", self.namespace, now_ms / DAY_MS)
    }

    /// Mark the store live — call once at start-up. Sets the durable sentinel whose later absence
    /// means the data was flushed. Returns whether it succeeded; a caller that cannot mark the store
    /// alive should treat budgets as unavailable.
    pub async fn mark_alive(&self) -> bool {
        let Some(mut conn) = self.conn().await else {
            return false;
        };
        redis::cmd("SET")
            .arg(self.sentinel_key())
            .arg("1")
            .query_async::<()>(&mut conn)
            .await
            .is_ok()
    }

    /// Attempt to spend one request against `identity`'s budget at `now_ms`. Increments the hourly
    /// and daily counters only when within both limits. **Denies on any uncertainty** — an
    /// unreachable Redis, a missing sentinel (flush), or a read that fails.
    pub async fn try_spend(
        &self,
        identity: &str,
        hourly: u32,
        daily: u32,
        now_ms: i64,
    ) -> Decision {
        let Some(mut conn) = self.conn().await else {
            return Decision::DenyUnavailable;
        };

        // Flush check: the sentinel we set at start-up must still be there. If it is gone, Redis
        // lost its data and we cannot trust a zero counter — fail closed.
        let alive: Option<String> = match redis::cmd("GET")
            .arg(self.sentinel_key())
            .query_async(&mut conn)
            .await
        {
            Ok(v) => v,
            Err(_) => return Decision::DenyUnavailable,
        };
        if alive.is_none() {
            return Decision::DenyUnavailable;
        }

        let hour_key = self.hour_key(identity, now_ms);
        let day_key = self.day_key(identity, now_ms);
        // Read both counters. A read error is uncertainty → deny.
        let used_hour: u32 = match redis::cmd("GET")
            .arg(&hour_key)
            .query_async::<Option<u32>>(&mut conn)
            .await
        {
            Ok(v) => v.unwrap_or(0),
            Err(_) => return Decision::DenyUnavailable,
        };
        let used_day: u32 = match redis::cmd("GET")
            .arg(&day_key)
            .query_async::<Option<u32>>(&mut conn)
            .await
        {
            Ok(v) => v.unwrap_or(0),
            Err(_) => return Decision::DenyUnavailable,
        };

        if used_hour >= hourly || used_day >= daily {
            return Decision::DenyOverBudget;
        }

        // Within budget: charge it. INCR both, and (re)set an expiry a little past the period so a
        // new bucket starts clean. If the write fails we have not charged — deny, so we never allow
        // a request we could not account for.
        let bumped: Result<(), _> = redis::pipe()
            .cmd("INCR")
            .arg(&hour_key)
            .ignore()
            .cmd("EXPIRE")
            .arg(&hour_key)
            .arg((HOUR_MS / 1000) + 300)
            .ignore()
            .cmd("INCR")
            .arg(&day_key)
            .ignore()
            .cmd("EXPIRE")
            .arg(&day_key)
            .arg((DAY_MS / 1000) + 3_600)
            .ignore()
            .query_async::<()>(&mut conn)
            .await;
        if bumped.is_err() {
            return Decision::DenyUnavailable;
        }
        Decision::Allow {
            remaining_hour: hourly.saturating_sub(used_hour + 1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_denied_decision_is_not_allowed() {
        assert!(Decision::Allow { remaining_hour: 5 }.is_allowed());
        assert!(!Decision::DenyOverBudget.is_allowed());
        assert!(!Decision::DenyUnavailable.is_allowed());
    }

    #[tokio::test]
    async fn an_unreachable_redis_fails_closed() {
        // The load-bearing property: no Redis → deny, never allow.
        let s = BudgetStore::connect_in("redis://127.0.0.1:1", "test").unwrap();
        assert!(!s.mark_alive().await);
        assert_eq!(
            s.try_spend("ig-1", 60, 400, 1_000_000).await,
            Decision::DenyUnavailable
        );
    }
}
