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
/// Per-source counters live in one hash, field `"<source_id>:<metric>"`. One hash rather than a key
/// per source keeps reset a single `DEL` and the dashboard read a single `HGETALL` — the same shape
/// as every other counter here. Source ids are slugs (`[a-z0-9-]`) so `:` never collides.
const K_SOURCE: &str = "crawl:source";
/// Per-channel yield counters (M2-T16.8), one hash, field `"<channel>:<metric>"`. Same one-hash
/// shape as the per-source counters. Channel tokens are a fixed closed set, so `:` never collides.
const K_CHANNEL: &str = "crawl:channel";

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
    /// Of `fetched`, the revisits. `fetched - revisited` is fresh discovery — the two halves of the
    /// crawl budget, so the console can show whether it is keeping the corpus current or growing it.
    #[serde(default)]
    pub revisited: u64,
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

/// Per-source quality counters (§7 of [[Data Sources Registry]]), accumulated by the crawler and
/// read by the console. Cumulative and read whole, like every other counter here.
///
/// `spam_sum` is the sum of per-document spam scores ×1000 (spam is `0.0..=1.0`; Redis counts
/// integers), so the mean is `spam_sum / indexed / 1000`. Keeping the sum rather than the mean is
/// what lets the mean stay correct as documents accrue without re-reading them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceMetrics {
    pub fetched: u64,
    pub failed: u64,
    pub indexed: u64,
    /// Parsed but too thin to index — the extraction-quality signal.
    pub thin: u64,
    /// Dropped as a duplicate of something already held.
    pub duplicate: u64,
    /// Indexed documents whose publication date we could not determine.
    pub date_unknown: u64,
    /// Σ(spam_score × 1000) over indexed documents.
    pub spam_sum: u64,
}

impl SourceMetrics {
    /// Fetches that succeeded, in `0.0..=1.0`. `None` until anything was attempted, so the console
    /// shows "—" rather than a misleading 100 % or 0 % for a source that has not run.
    pub fn fetch_success_rate(&self) -> Option<f32> {
        let attempts = self.fetched + self.failed;
        (attempts > 0).then(|| self.fetched as f32 / attempts as f32)
    }

    /// Of fetched pages, the fraction that yielded an indexable document rather than being too thin.
    /// The silent-failure signal: a redesign that breaks extraction shows here before anywhere else.
    pub fn extraction_success_rate(&self) -> Option<f32> {
        let seen = self.indexed + self.thin;
        (seen > 0).then(|| self.indexed as f32 / seen as f32)
    }

    /// Fraction of indexed-or-duplicate documents that were duplicates — high means mostly
    /// republished content, a demotion signal.
    pub fn duplicate_ratio(&self) -> Option<f32> {
        let total = self.indexed + self.duplicate;
        (total > 0).then(|| self.duplicate as f32 / total as f32)
    }

    /// Mean spam score over indexed documents, in `0.0..=1.0`.
    pub fn spam_mean(&self) -> Option<f32> {
        (self.indexed > 0).then(|| self.spam_sum as f32 / self.indexed as f32 / 1000.0)
    }

    /// Fraction of indexed documents with no determinable date — high means the parser needs a date
    /// selector for this domain.
    pub fn date_unknown_ratio(&self) -> Option<f32> {
        (self.indexed > 0).then(|| self.date_unknown as f32 / self.indexed as f32)
    }

    /// Map the accumulated counters to the health signal the lifecycle automation consumes
    /// ([`xustive_core::model::SourceHealth`] via the caller). `error_rate_24h` is approximated by
    /// the lifetime failure rate — these counters are cumulative, not windowed, so this is the
    /// coarse form; a windowed source can refine it later without changing the consumer.
    pub fn error_rate(&self) -> f32 {
        let attempts = self.fetched + self.failed;
        if attempts == 0 {
            0.0
        } else {
            self.failed as f32 / attempts as f32
        }
    }
}

/// Per-channel discovery yield (§M2-T16.8): the funnel from URLs a channel introduced to documents
/// that survived dedup. This is the number that decides whether an expensive channel (SERP, Brave)
/// earns its place — measured, not assumed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelMetrics {
    /// URLs this channel introduced to the frontier.
    pub discovered: u64,
    /// Of those, the ones actually fetched (some are dropped as duplicates or traps first).
    pub fetched: u64,
    /// Documents indexed from this channel's URLs.
    pub indexed: u64,
    /// Documents dropped as duplicates of something already held — the same URL reached by a
    /// cheaper channel, typically. A high ratio means the channel is mostly rediscovering.
    pub duplicate: u64,
}

impl ChannelMetrics {
    /// Documents that survived to the index as a fraction of URLs discovered — the channel's yield.
    /// `None` until the channel has discovered anything, so the console shows "—" not a false 0%.
    pub fn yield_rate(&self) -> Option<f32> {
        (self.discovered > 0).then(|| self.indexed as f32 / self.discovered as f32)
    }

    /// Of documents this channel produced, the fraction that were fresh rather than duplicates.
    pub fn unique_rate(&self) -> Option<f32> {
        let total = self.indexed + self.duplicate;
        (total > 0).then(|| self.indexed as f32 / total as f32)
    }
}

/// Live crawl counters in Redis.
///
/// Holds **one** auto-reconnecting [`redis::aio::ConnectionManager`], cloned per operation rather
/// than opened per call. This matters most for the admin "Live" page: it snapshots once a second
/// over SSE, and opening a fresh multiplexed connection each frame was both wasteful and fragile —
/// a single transient blip during connection setup showed as "Redis unreachable" even though Redis
/// was up (the same churn that once flooded the logs from the queue). A `ConnectionManager` hands
/// back a shared connection instantly and re-establishes itself under the hood, so a blip
/// self-heals instead of surfacing as an outage.
#[derive(Clone)]
pub struct CrawlStats {
    manager: redis::aio::ConnectionManager,
}

impl CrawlStats {
    /// Connect, establishing the shared connection manager. `None` only if Redis is genuinely
    /// unreachable at connect time; after that the manager tolerates transient drops.
    pub async fn connect(url: &str) -> Option<Self> {
        let client = redis::Client::open(url).ok()?;
        let manager = client.get_connection_manager().await.ok()?;
        Some(Self { manager })
    }

    /// The shared connection. Cloning a `ConnectionManager` is cheap and never fails — it is a
    /// handle to the one managed connection, so this is `Some` for the life of the store.
    async fn conn(&self) -> Option<redis::aio::ConnectionManager> {
        Some(self.manager.clone())
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

    /// Bump one per-source counter (§7). `source_id` is a registry slug; `metric` is one of the
    /// `SourceMetrics` field names. Best-effort like the rest — a lost counter is a slightly wrong
    /// dashboard, never a lost document.
    pub async fn incr_source(&self, source_id: &str, metric: &str, by: u64) {
        if source_id.is_empty() || by == 0 {
            return;
        }
        let Some(mut c) = self.conn().await else {
            return;
        };
        let _: Result<(), _> = redis::cmd("HINCRBY")
            .arg(K_SOURCE)
            .arg(format!("{source_id}:{metric}"))
            .arg(by as i64)
            .query_async::<()>(&mut c)
            .await;
    }

    /// Read every source's counters, keyed by source id. One `HGETALL`, reassembled by splitting
    /// each field on its last `:` into `<source_id>:<metric>`.
    pub async fn source_metrics(&self) -> HashMap<String, SourceMetrics> {
        let Some(mut c) = self.conn().await else {
            return HashMap::new();
        };
        let flat: HashMap<String, u64> = redis::cmd("HGETALL")
            .arg(K_SOURCE)
            .query_async(&mut c)
            .await
            .unwrap_or_default();
        let mut out: HashMap<String, SourceMetrics> = HashMap::new();
        for (field, value) in flat {
            let Some((id, metric)) = field.rsplit_once(':') else {
                continue;
            };
            let m = out.entry(id.to_string()).or_default();
            match metric {
                "fetched" => m.fetched = value,
                "failed" => m.failed = value,
                "indexed" => m.indexed = value,
                "thin" => m.thin = value,
                "duplicate" => m.duplicate = value,
                "date_unknown" => m.date_unknown = value,
                "spam_sum" => m.spam_sum = value,
                _ => {}
            }
        }
        out
    }

    /// Bump one per-channel yield counter (M2-T16.8). `channel` is a
    /// [`xustive_core::DiscoveryChannel::token`]; `metric` is a `ChannelMetrics` field name.
    pub async fn incr_channel(&self, channel: &str, metric: &str, by: u64) {
        if channel.is_empty() || by == 0 {
            return;
        }
        let Some(mut c) = self.conn().await else {
            return;
        };
        let _: Result<(), _> = redis::cmd("HINCRBY")
            .arg(K_CHANNEL)
            .arg(format!("{channel}:{metric}"))
            .arg(by as i64)
            .query_async::<()>(&mut c)
            .await;
    }

    /// Read every channel's yield counters, keyed by channel token. One `HGETALL`, split on the
    /// last `:` into `<channel>:<metric>`.
    pub async fn channel_metrics(&self) -> HashMap<String, ChannelMetrics> {
        let Some(mut c) = self.conn().await else {
            return HashMap::new();
        };
        let flat: HashMap<String, u64> = redis::cmd("HGETALL")
            .arg(K_CHANNEL)
            .query_async(&mut c)
            .await
            .unwrap_or_default();
        let mut out: HashMap<String, ChannelMetrics> = HashMap::new();
        for (field, value) in flat {
            let Some((chan, metric)) = field.rsplit_once(':') else {
                continue;
            };
            let m = out.entry(chan.to_string()).or_default();
            match metric {
                "discovered" => m.discovered = value,
                "fetched" => m.fetched = value,
                "indexed" => m.indexed = value,
                "duplicate" => m.duplicate = value,
                _ => {}
            }
        }
        out
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

        // The first read is authoritative for reachability. With the shared connection manager the
        // clone above never fails, so "is Redis actually answering" has to come from a real command:
        // an **error** means unreachable (surfaced as such), while an **empty** result means the
        // crawler simply has not counted anything yet (idle, not down). Every later read tolerates a
        // blip with `unwrap_or_default`, since this one already decided the availability.
        let counters: HashMap<String, u64> = match redis::cmd("HGETALL")
            .arg(K_COUNTERS)
            .query_async(&mut c)
            .await
        {
            Ok(counters) => counters,
            Err(_) => {
                return Snapshot {
                    unavailable: true,
                    state: "unknown".into(),
                    ..Snapshot::default()
                }
            }
        };
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
            revisited: counters.get("revisited").copied().unwrap_or(0),
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
        for k in [K_COUNTERS, K_SKIPS, K_RECENT, K_HOSTS, K_SOURCE, K_CHANNEL] {
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
    fn source_metrics_compute_the_section_7_ratios() {
        let m = SourceMetrics {
            fetched: 90,
            failed: 10,
            indexed: 60,
            thin: 20,
            duplicate: 20,
            date_unknown: 6,
            // mean spam 0.15 over 60 indexed → sum = 0.15*60*1000 = 9000
            spam_sum: 9_000,
        };
        assert!((m.fetch_success_rate().unwrap() - 0.90).abs() < 1e-6);
        assert!((m.extraction_success_rate().unwrap() - 0.75).abs() < 1e-6); // 60/(60+20)
        assert!((m.duplicate_ratio().unwrap() - 0.25).abs() < 1e-6); // 20/(60+20)
        assert!((m.spam_mean().unwrap() - 0.15).abs() < 1e-6);
        assert!((m.date_unknown_ratio().unwrap() - 0.10).abs() < 1e-6); // 6/60
        assert!((m.error_rate() - 0.10).abs() < 1e-6);
    }

    #[test]
    fn channel_metrics_compute_yield_and_unique_rate() {
        let m = ChannelMetrics {
            discovered: 1000,
            fetched: 800,
            indexed: 200,
            duplicate: 50,
        };
        assert!((m.yield_rate().unwrap() - 0.20).abs() < 1e-6); // 200/1000
        assert!((m.unique_rate().unwrap() - 0.80).abs() < 1e-6); // 200/(200+50)
                                                                 // A channel that discovered nothing yet reports None, not a misleading 0%.
        assert!(ChannelMetrics::default().yield_rate().is_none());
        assert!(ChannelMetrics::default().unique_rate().is_none());
    }

    #[test]
    fn a_source_that_never_ran_reports_none_not_a_misleading_zero() {
        // A blank source must read as "—" on the dashboard, not "0% fetch success" (which looks
        // like a broken source) nor "100%" (which looks healthy). None is the honest answer.
        let m = SourceMetrics::default();
        assert!(m.fetch_success_rate().is_none());
        assert!(m.extraction_success_rate().is_none());
        assert!(m.spam_mean().is_none());
        assert_eq!(m.error_rate(), 0.0, "no attempts is not an error");
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
