//! The tool data plane.
//!
//! The serving plane has **no route to the internet** — that is enforced, not aspirational, and
//! it is what guarantees a prompt-injected summariser or a compromised dependency cannot
//! exfiltrate queries. Weather and exchange rates come from outside.
//!
//! Rather than giving the serving plane egress, this process fetches on a schedule into a Redis
//! cache both planes can reach. The consequence is worth stating plainly: **the serving plane can
//! only ever answer from cache.** A tool needing data nobody has fetched has no answer, and that
//! is correct rather than unfortunate.
//!
//! It never sees a query. It cannot — it does not share a network with anything that has one. A
//! weather request from a user therefore reveals nothing to any publisher, because the fetch
//! pattern is identical whether one person searched or a million did.

pub mod store;
pub mod validate;
pub mod weather;

use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("http: {0}")]
    Http(String),
    #[error("payload did not parse: {0}")]
    Parse(String),
    #[error("rejected: {0}")]
    Rejected(String),
    #[error("cache: {0}")]
    Cache(String),
}

/// A cached dataset, with its provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cached<T> {
    /// When we asked.
    pub fetched_at: i64,
    /// When the **publisher** measured it.
    ///
    /// Distinct from `fetched_at` on purpose. A rate fetched a minute ago that the publisher
    /// measured yesterday is a day old, and showing the fetch time would be exactly the lie the
    /// instant-answer rules forbid.
    pub observed_at: i64,
    pub source: String,
    pub licence: String,
    pub payload: T,
}

impl<T> Cached<T> {
    /// Age against the publisher's measurement, not our fetch.
    pub fn age(&self, now: i64) -> Duration {
        Duration::from_secs(now.saturating_sub(self.observed_at).max(0) as u64)
    }

    pub fn is_stale(&self, now: i64, limit: Duration) -> bool {
        self.age(now) > limit
    }
}

/// A dataset this process knows how to fetch.
pub trait Dataset {
    /// Cache key prefix. Versioned, so a schema change does not have to reconcile with entries
    /// written by an older build — it simply writes elsewhere and the old ones age out.
    fn key_prefix(&self) -> &'static str;

    /// How often to fetch. Fixed, never per-request.
    fn cadence(&self) -> Duration;

    /// Beyond this, the serving plane withholds the value rather than showing it stale.
    fn staleness_limit(&self) -> Duration;
}

/// Shared client.
///
/// A single user agent that identifies the project, because a publisher seeing unexplained
/// traffic should be able to find out who we are and ask us to stop.
pub fn client() -> Result<reqwest::Client, FetchError> {
    reqwest::Client::builder()
        .user_agent("XustiveToolFetcher/0.1 (+https://xustive.dz; contact via repository)")
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| FetchError::Http(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_is_measured_from_the_publishers_observation() {
        // The distinction the whole struct exists for. Fetched a minute ago, measured yesterday:
        // the value is a day old, and reporting a minute would be false.
        let cached = Cached {
            fetched_at: 1_786_000_000,
            observed_at: 1_785_913_600, // 24 hours earlier
            source: "test".into(),
            licence: "CC-BY".into(),
            payload: (),
        };
        assert_eq!(cached.age(1_786_000_000).as_secs(), 86_400);
    }

    #[test]
    fn a_future_observation_does_not_wrap_to_an_enormous_age() {
        // Clock skew between us and a publisher is routine. A negative age wrapped into a `u64`
        // would report roughly 585 billion years and mark everything permanently stale.
        let cached = Cached {
            fetched_at: 1_786_000_000,
            observed_at: 1_786_003_600,
            source: "test".into(),
            licence: "CC-BY".into(),
            payload: (),
        };
        assert_eq!(cached.age(1_786_000_000), Duration::ZERO);
        assert!(!cached.is_stale(1_786_000_000, Duration::from_secs(60)));
    }

    #[test]
    fn staleness_is_a_comparison_against_the_limit() {
        let cached = Cached {
            fetched_at: 0,
            observed_at: 0,
            source: "t".into(),
            licence: "t".into(),
            payload: (),
        };
        assert!(!cached.is_stale(3_600, Duration::from_secs(7_200)));
        assert!(cached.is_stale(10_000, Duration::from_secs(7_200)));
    }
}
