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

/// Whether a term's count is high enough to surface. `k` is the anonymity floor the caller derived
/// from config ([`xustive_core::config::DiscoveryConfig::effective_k`]) — 20 by default, lower only
/// on a single-user deployment where there is no one to anonymise against. This predicate does not
/// re-clamp: it trusts the floor the config already decided, so the policy lives in exactly one
/// place. A `k` of 0 is treated as 1 — a term must have been searched at least once.
pub fn surfaceable(count: u32, k: u32) -> bool {
    count >= k.max(1)
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
    /// Connect. `k` is the anonymity floor the caller derived from config (default 20; lower only for
    /// a single-user deployment). Treated as at least 1. Lazy — no connection until first use.
    pub fn connect_in(url: &str, namespace: &str, k: u32, window: Duration) -> Option<Self> {
        Some(Self {
            client: redis::Client::open(url).ok()?,
            namespace: namespace.to_string(),
            k: k.max(1),
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
        // INCR with a **banded** sliding expiry (BUG-039, mirroring `Interactions::bump`): the
        // window is armed when the key is new or its TTL has fallen below half the window, never on
        // every hit — refreshing every time made `window − TTL` a per-term last-searched timestamp
        // readable to ~1s. An actively-searched term still persists and a one-off still decays
        // (worst case window/2 sooner); a counter still never exists without an expiry.
        let window = self.window.as_secs().max(1) as i64;
        // The increment runs on its own, so the TTL refinement below can never cost the count.
        let _: Result<i64, _> = redis::cmd("INCR").arg(&key).query_async(&mut conn).await;
        // On a failed PTTL read, treat the key as expiry-less (-1) and arm it: the INCR may have
        // landed, and a term key with no expiry is unbounded retention of the term text.
        let pttl = redis::cmd("PTTL")
            .arg(&key)
            .query_async::<i64>(&mut conn)
            .await
            .unwrap_or(-1);
        if pttl < window * 500 {
            let _: Result<(), _> = redis::cmd("EXPIRE")
                .arg(&key)
                .arg(window)
                .query_async::<()>(&mut conn)
                .await;
        }
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
    fn surfaceable_honours_the_configured_floor() {
        // The public default of 20: nothing below it surfaces.
        assert!(!surfaceable(19, 20));
        assert!(surfaceable(20, 20));
        // A single-user deployment lowering the floor: a term surfaces at its configured count.
        assert!(
            surfaceable(1, 1),
            "a personal engine can act on a single search"
        );
        assert!(
            !surfaceable(0, 1),
            "but a term must have been searched at least once"
        );
        assert!(surfaceable(3, 3));
        assert!(!surfaceable(2, 3));
        // k of 0 is treated as 1.
        assert!(surfaceable(1, 0));
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
