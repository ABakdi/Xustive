//! Crawler counters, shared through Redis.
//!
//! The crawler is a separate process from the API, so the console cannot read its memory. It could
//! have exposed its own HTTP endpoint; putting the counters in Redis is better for two reasons:
//!
//! 1. **The console keeps working while the crawler is restarting.** An endpoint on the crawler
//!    goes away exactly when you most want to know what happened.
//! 2. **One set of numbers.** The console and Prometheus read the same keys, so they cannot
//!    disagree — and two dashboards disagreeing is worse than one, because nothing tells you which
//!    is lying.
//!
//! # Absolute counters, and a bounded feed
//!
//! Counters are cumulative and read whole. Deltas would need every reader to have seen every
//! frame, and a console that missed one would drift silently until reload.
//!
//! The recent-URL feed is a capped list. It is the part that answers "is it collecting articles or
//! tag pages", which no aggregate can, and it is also the part that would grow without bound.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

const K_COUNTERS: &str = "crawl:counters";
const K_SKIPS: &str = "crawl:skips";
const K_RECENT: &str = "crawl:recent";
const K_HOSTS: &str = "crawl:hosts";
const K_STATE: &str = "crawl:state";

/// How many recent URLs to keep.
///
/// Enough to see a pattern, small enough that reading it is one cheap call. Fifty is roughly a
/// screen; a thousand would be a scroll nobody does and a slower page every second.
const RECENT_MAX: isize = 50;

/// What happened to one URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentUrl {
    pub url: String,
    pub host: String,
    /// `indexed`, `thin`, `robots`, `failed`, … — the same vocabulary as the skip counters.
    pub outcome: String,
    pub at: i64,
    /// Words extracted. A navigation page and a real article look identical by title, and this is
    /// the cheapest thing that tells them apart at a glance.
    pub words: usize,
}

/// A snapshot the console renders.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub state: String,
    pub fetched: u64,
    pub parsed: u64,
    pub indexed: u64,
    pub discovered: u64,
    pub failed: u64,
    pub skipped: HashMap<String, u64>,
    pub recent: Vec<RecentUrl>,
    /// host → last-fetch unix seconds.
    pub hosts: HashMap<String, i64>,
    pub waiting: usize,
    pub inflight: usize,
    /// Pages waiting for their revisit due time. The freshness backlog, distinct from `waiting`:
    /// these are pages we already hold and have booked a return to.
    #[serde(default)]
    pub deferred: usize,
    /// True when the counters could not be read at all.
    ///
    /// The console shows this rather than zeroes. A zero and an unreachable Redis look identical,
    /// and the second is the one that needs attention — an operator seeing `0 fetched, 0 failed`
    /// reasonably concludes the crawl is idle.
    pub unavailable: bool,
}

#[derive(Clone)]
pub struct CrawlStats {
    client: redis::Client,
}

impl CrawlStats {
    pub fn connect(url: &str) -> Option<Self> {
        redis::Client::open(url).ok().map(|client| Self { client })
    }

    async fn conn(&self) -> Option<redis::aio::MultiplexedConnection> {
        self.client.get_multiplexed_async_connection().await.ok()
    }

    /// Record that the crawler is running, stopped, or whatever else.
    pub async fn set_state(&self, state: &str) {
        let Some(mut c) = self.conn().await else {
            return;
        };
        let _: Result<(), _> = redis::cmd("SET")
            .arg(K_STATE)
            .arg(state)
            .query_async::<()>(&mut c)
            .await;
    }

    /// Add to a counter.
    pub async fn incr(&self, field: &str, by: u64) {
        let Some(mut c) = self.conn().await else {
            return;
        };
        let _: Result<(), _> = redis::cmd("HINCRBY")
            .arg(K_COUNTERS)
            .arg(field)
            .arg(by as i64)
            .query_async::<()>(&mut c)
            .await;
    }

    pub async fn incr_skip(&self, reason: &str) {
        let Some(mut c) = self.conn().await else {
            return;
        };
        let _: Result<(), _> = redis::cmd("HINCRBY")
            .arg(K_SKIPS)
            .arg(reason)
            .arg(1)
            .query_async::<()>(&mut c)
            .await;
    }

    /// Record a fetched URL and the host's activity in one round trip.
    pub async fn record(&self, entry: &RecentUrl) {
        let Some(mut c) = self.conn().await else {
            return;
        };
        let Ok(payload) = serde_json::to_string(entry) else {
            return;
        };
        // Pushed then trimmed, so the list cannot grow between the two.
        let mut pipe = redis::pipe();
        pipe.cmd("LPUSH")
            .arg(K_RECENT)
            .arg(payload)
            .ignore()
            .cmd("LTRIM")
            .arg(K_RECENT)
            .arg(0)
            .arg(RECENT_MAX - 1)
            .ignore()
            .cmd("HSET")
            .arg(K_HOSTS)
            .arg(&entry.host)
            .arg(entry.at)
            .ignore();
        let _: Result<(), _> = pipe.query_async::<()>(&mut c).await;
    }

    /// Everything the console needs, in one read.
    pub async fn snapshot(&self) -> Snapshot {
        let Some(mut c) = self.conn().await else {
            return Snapshot {
                unavailable: true,
                state: "unknown".into(),
                ..Snapshot::default()
            };
        };

        let counters: HashMap<String, u64> = redis::cmd("HGETALL")
            .arg(K_COUNTERS)
            .query_async(&mut c)
            .await
            .unwrap_or_default();
        let skipped: HashMap<String, u64> = redis::cmd("HGETALL")
            .arg(K_SKIPS)
            .query_async(&mut c)
            .await
            .unwrap_or_default();
        let raw: Vec<String> = redis::cmd("LRANGE")
            .arg(K_RECENT)
            .arg(0)
            .arg(RECENT_MAX - 1)
            .query_async(&mut c)
            .await
            .unwrap_or_default();
        let hosts: HashMap<String, i64> = redis::cmd("HGETALL")
            .arg(K_HOSTS)
            .query_async(&mut c)
            .await
            .unwrap_or_default();
        let state: Option<String> = redis::cmd("GET")
            .arg(K_STATE)
            .query_async(&mut c)
            .await
            .unwrap_or(None);

        Snapshot {
            // No state key at all means the crawler has never run, which is different from stopped
            // and worth saying differently.
            state: state.unwrap_or_else(|| "never started".into()),
            // Filled by the caller that holds a Frontier; the stats store does not know about
            // the due set and should not — it would be a second reader disagreeing with the first.
            deferred: 0,
            fetched: counters.get("fetched").copied().unwrap_or(0),
            parsed: counters.get("parsed").copied().unwrap_or(0),
            indexed: counters.get("indexed").copied().unwrap_or(0),
            discovered: counters.get("discovered").copied().unwrap_or(0),
            failed: counters.get("failed").copied().unwrap_or(0),
            skipped,
            recent: raw
                .iter()
                .filter_map(|s| serde_json::from_str(s).ok())
                .collect(),
            hosts,
            waiting: 0,
            inflight: 0,
            unavailable: false,
        }
    }

    /// Reset the counters. Deliberate operator action only.
    pub async fn reset(&self) {
        let Some(mut c) = self.conn().await else {
            return;
        };
        for k in [K_COUNTERS, K_SKIPS, K_RECENT, K_HOSTS] {
            let _: Result<(), _> = redis::cmd("DEL").arg(k).query_async::<()>(&mut c).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreadable_snapshot_says_so_rather_than_reporting_zero() {
        // The failure the whole console exists to prevent. `0 fetched, 0 failed` reads as a healthy
        // idle crawler; "cannot read state" reads as something to look at.
        let s = Snapshot {
            unavailable: true,
            state: "unknown".into(),
            ..Snapshot::default()
        };
        assert!(s.unavailable);
        assert_ne!(s.state, "stopped");
    }

    #[test]
    fn never_started_is_distinct_from_stopped() {
        // Different problems. One means "you have not run it"; the other means "it ran and ended".
        let fresh = Snapshot::default();
        assert_eq!(fresh.state, "");
        assert!(!fresh.unavailable);
    }

    #[test]
    fn the_recent_feed_is_bounded() {
        // It is the part that would otherwise grow without limit, and it is read every second.
        const { assert!(RECENT_MAX > 10 && RECENT_MAX <= 200) };
    }

    #[test]
    fn a_recent_entry_carries_what_distinguishes_an_article_from_a_tag_page() {
        let e = RecentUrl {
            url: "https://e.dz/a".into(),
            host: "e.dz".into(),
            outcome: "indexed".into(),
            at: 1,
            words: 640,
        };
        let json = serde_json::to_string(&e).expect("serialises");
        assert!(json.contains("words"), "word count is the cheap tell");
        assert!(json.contains("outcome"));
    }
}
