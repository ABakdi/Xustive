//! Query-driven discovery: which searches the corpus cannot answer (M2-T16.4).
//!
//! A search that returns little or nothing is a free, precise signal of weak coverage — the exact
//! queries worth finding more sources for. But a query is personal data, and
//! [[ADR-0008 - No Query Logging]] forbids retaining it. This module is the narrow, structural
//! reconciliation of the two:
//!
//! - **Off unless explicitly enabled** ([`xustive_core::config::DiscoveryConfig`]). Disabled, this
//!   type is never even constructed, so a search leaves no trace anywhere.
//! - **k-anonymous on read.** A term is stored as a bare counter, and [`WeakCoverage::weak_terms`]
//!   never returns one below the `k` threshold (≥ 20 per the ADR). A term surfaced here has been
//!   searched by at least twenty searches — common, not personal.
//! - **Windowed.** Each term's counter carries a sliding TTL, so a term that is not searched again
//!   within the window decays and is forgotten. A rare query cannot slowly accrete to the floor
//!   over years, and retention is bounded by construction.
//!
//! What it deliberately does **not** do yet is resolve a weak term to URLs — that needs an external
//! discovery source (Brave, SERP, Common Crawl) which is a later task. Until then this surfaces the
//! gaps for the console (M2-T16.5) and for a human to act on.

use std::time::Duration;

/// Whether a term's count is high enough to surface without breaching k-anonymity. The single
/// privacy-critical predicate, pulled out so it is trivially testable: a term below `k` is never
/// returned to any caller, whatever else changes.
pub fn surfaceable(count: u32, k: u32) -> bool {
    count >= k.max(20)
}

/// A weak-coverage term and how many searches hit it, once past the k-anonymity floor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeakTerm {
    pub term: String,
    pub count: u32,
}

/// The recorder. Holds no query text itself — everything lives in Redis under a sliding TTL.
#[derive(Clone)]
pub struct WeakCoverage {
    client: redis::Client,
    namespace: String,
    k: u32,
    window: Duration,
}

impl WeakCoverage {
    /// Connect. `k` is clamped up to the ADR's floor of 20 here too, so a misconfiguration cannot
    /// lower it. Lazy — no connection until first use.
    pub fn connect_in(url: &str, namespace: &str, k: u32, window: Duration) -> Option<Self> {
        Some(Self {
            client: redis::Client::open(url).ok()?,
            namespace: namespace.to_string(),
            k: k.max(20),
            window,
        })
    }

    async fn conn(&self) -> Option<redis::aio::MultiplexedConnection> {
        self.client.get_multiplexed_async_connection().await.ok()
    }

    fn key(&self, term: &str) -> String {
        format!("{}:weak:{term}", self.namespace)
    }

    fn prefix(&self) -> String {
        format!("{}:weak:", self.namespace)
    }

    /// Record one weak-coverage search for `term` (a normalised query). Increments the counter and
    /// refreshes its window. Best-effort: a lost increment is a slightly undercounted gap, never a
    /// correctness problem. The caller must already have checked that recording is enabled and that
    /// the search was actually weak — this type does not police policy it cannot see.
    pub async fn record(&self, term: &str) {
        let term = term.trim();
        if term.is_empty() {
            return;
        }
        let Some(mut conn) = self.conn().await else {
            return;
        };
        let key = self.key(term);
        // INCR then a sliding EXPIRE: the window restarts on every hit, so an actively-searched
        // term persists and a one-off decays. Pipelined so the two cannot be split by a crash into
        // a counter with no expiry (which would be unbounded retention of the term text).
        let _: Result<(), _> = redis::pipe()
            .cmd("INCR")
            .arg(&key)
            .ignore()
            .cmd("EXPIRE")
            .arg(&key)
            .arg(self.window.as_secs().max(1))
            .ignore()
            .query_async::<()>(&mut conn)
            .await;
    }

    /// The weak-coverage terms worth acting on: those at or above the k-anonymity floor, most-
    /// searched first, capped at `limit`. Terms below the floor are never returned — that is the
    /// structural k-anonymity guarantee, not a courtesy.
    pub async fn weak_terms(&self, limit: usize) -> Vec<WeakTerm> {
        let Some(mut conn) = self.conn().await else {
            return Vec::new();
        };
        let prefix = self.prefix();
        let pattern = format!("{prefix}*");

        // SCAN rather than KEYS: KEYS blocks the server, and this runs behind a console page, not a
        // benchmark. Cursor-paged so a large keyspace does not arrive in one reply.
        let mut cursor: u64 = 0;
        let mut out = Vec::new();
        loop {
            let (next, keys): (u64, Vec<String>) = match redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(200)
                .query_async(&mut conn)
                .await
            {
                Ok(v) => v,
                Err(_) => return out,
            };
            for key in keys {
                let count: u32 = redis::cmd("GET")
                    .arg(&key)
                    .query_async(&mut conn)
                    .await
                    .unwrap_or(Some(0))
                    .unwrap_or(0);
                // The k-anonymity gate. A term below the floor is dropped here and never seen.
                if surfaceable(count, self.k) {
                    if let Some(term) = key.strip_prefix(&prefix) {
                        out.push(WeakTerm {
                            term: term.to_string(),
                            count,
                        });
                    }
                }
            }
            cursor = next;
            if cursor == 0 {
                break;
            }
        }
        out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.term.cmp(&b.term)));
        out.truncate(limit);
        out
    }

    /// Forget a term — after its coverage gap has been resolved, so it stops being surfaced.
    pub async fn forget(&self, term: &str) {
        let Some(mut conn) = self.conn().await else {
            return;
        };
        let _: Result<(), _> = redis::cmd("DEL")
            .arg(self.key(term))
            .query_async::<()>(&mut conn)
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_term_below_k_is_never_surfaceable_even_if_k_is_misconfigured_low() {
        // The ADR floor of 20 holds whatever the config says.
        assert!(!surfaceable(19, 20));
        assert!(surfaceable(20, 20));
        // A config that tried to set k=5 is clamped: 19 still not surfaceable.
        assert!(!surfaceable(19, 5));
        assert!(surfaceable(20, 5));
        assert!(surfaceable(1000, 20));
    }

    #[tokio::test]
    async fn a_dead_redis_is_silent_and_surfaces_nothing() {
        let w =
            WeakCoverage::connect_in("redis://127.0.0.1:1", "test", 20, Duration::from_secs(60))
                .unwrap();
        // Record and read against a port nothing listens on: no panic, nothing surfaced.
        w.record("الجزائر تعليم").await;
        assert!(w.weak_terms(10).await.is_empty());
    }
}
