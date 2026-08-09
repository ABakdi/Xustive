//! Shared `robots.txt` cache.
//!
//! The in-process cache on `Politeness` is per worker. With one worker that is the whole story;
//! with fifty it means fifty separate requests for the same `robots.txt`, and the file every
//! polite crawler is supposed to read once a day becomes the most-requested path on the site.
//!
//! That is not merely wasteful. A site watching its logs sees a burst of identical requests from
//! one user-agent and reasonably concludes we are misbehaving — and the file they are looking at
//! is the one that tells us not to.
//!
//! So the parsed rules go in Redis, which every worker already reaches, keyed by host.
//!
//! # Failing open, deliberately, and only here
//!
//! Everywhere else in this crate an unavailable dependency means refuse. Here it means fall
//! through to fetching directly. A Redis outage must not stop the crawl and must never turn into
//! "no rules cached, therefore disallow everything" — that would convert a cache failure into a
//! silent halt, which is the failure mode hardest to diagnose and easiest to mistake for the
//! crawler simply having nothing to do.
//!
//! The safety property is preserved because falling through means *fetching robots.txt properly*,
//! not skipping it. The cache is an optimisation; the rules are not.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::robots::Robots;

/// How long a cached entry stays valid.
///
/// RFC 9309 §2.4 puts the ceiling at 24 hours, and a site that changes its rules is entitled to
/// have that honoured within a day.
pub const TTL: Duration = Duration::from_secs(24 * 3600);

/// Key prefix, versioned.
///
/// A schema change writes to new keys rather than reconciling with entries an older build wrote —
/// deserialising an old shape into a new one is how a cache starts returning rules that parse but
/// mean something else.
const PREFIX: &str = "robots:v1";

pub fn key(host: &str) -> String {
    format!("{PREFIX}:{}", host.to_ascii_lowercase())
}

/// A cached `robots.txt`, as stored.
///
/// The **source text** is stored, not the parsed structure. Parsing is microseconds and the text
/// is what the site actually served, so a parser fix applies to everything already cached instead
/// of needing the cache dropped. It also means a cached entry can be read by a human debugging why
/// a host is being refused, which a serialised rule tree cannot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedRobots {
    pub fetched_at: i64,
    /// The file as served, or empty when the fetch failed.
    pub text: String,
    /// The status that produced this entry. Kept because "404, so unrestricted" and "403, so
    /// disallow" are both cache hits and a human needs to tell them apart.
    pub status: u16,
    /// True when the fetch failed and everything is disallowed.
    pub deny_all: bool,
}

impl CachedRobots {
    pub fn from_text(text: String, status: u16, now: i64) -> Self {
        Self {
            fetched_at: now,
            text,
            status,
            deny_all: false,
        }
    }

    pub fn denied(status: u16, now: i64) -> Self {
        Self {
            fetched_at: now,
            text: String::new(),
            status,
            deny_all: true,
        }
    }

    pub fn to_robots(&self) -> Robots {
        if self.deny_all {
            Robots::deny_all()
        } else {
            Robots::parse(&self.text)
        }
    }

    pub fn age(&self, now: i64) -> Duration {
        Duration::from_secs(now.saturating_sub(self.fetched_at).max(0) as u64)
    }

    pub fn is_stale(&self, now: i64) -> bool {
        self.age(now) > TTL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_786_000_000;

    #[test]
    fn the_stored_text_is_reparsed_rather_than_a_rule_tree_deserialised() {
        // Storing the source means a parser fix applies to everything already cached, instead of
        // needing the cache dropped — and a human debugging a refusal can read the entry.
        let c = CachedRobots::from_text("User-agent: *\nDisallow: /admin\n".into(), 200, NOW);
        let r = c.to_robots();
        assert!(!r.allows("/admin/x"));
        assert!(r.allows("/about"));
    }

    #[test]
    fn a_denied_entry_disallows_everything() {
        // A 403 is a cache hit too. Caching only successes means re-requesting robots.txt on every
        // single fetch for a host that refused us, which is the rudest possible response to being
        // refused.
        let c = CachedRobots::denied(403, NOW);
        assert!(!c.to_robots().allows("/anything"));
        assert!(c.to_robots().is_deny_all());
    }

    #[test]
    fn the_status_survives_so_a_hit_can_be_explained() {
        // "404, therefore unrestricted" and "403, therefore disallowed" are both cache hits, and
        // a human looking at why a host is being skipped needs to tell them apart.
        assert_eq!(CachedRobots::denied(403, NOW).status, 403);
        assert_eq!(CachedRobots::from_text(String::new(), 404, NOW).status, 404);
    }

    #[test]
    fn entries_expire_after_a_day() {
        // RFC 9309 §2.4. A site that changes its rules is entitled to have that honoured within
        // a day.
        let c = CachedRobots::from_text("User-agent: *\n".into(), 200, NOW);
        assert!(!c.is_stale(NOW + 3600));
        assert!(!c.is_stale(NOW + 23 * 3600));
        assert!(c.is_stale(NOW + 25 * 3600));
    }

    #[test]
    fn a_future_timestamp_does_not_wrap_into_an_enormous_age() {
        // Clock skew between workers is routine, and a negative age wrapped into a u64 would read
        // as roughly 585 billion years — marking every entry permanently fresh, which is the
        // wrong direction to be wrong in.
        let c = CachedRobots::from_text(String::new(), 200, NOW + 600);
        assert_eq!(c.age(NOW), Duration::ZERO);
        assert!(!c.is_stale(NOW));
    }

    #[test]
    fn keys_are_versioned_and_case_insensitive() {
        assert_eq!(key("Example.DZ"), key("example.dz"));
        assert!(key("example.dz").starts_with("robots:v1:"));
    }

    #[test]
    fn the_ttl_matches_what_the_bot_page_promises() {
        // `/bot` tells operators a change takes effect within 24 hours. A TTL longer than that
        // makes the page a lie, and the person it misleads is the one who blocked us.
        assert!(TTL <= Duration::from_secs(24 * 3600));
    }
}

/// A Redis-backed shared cache.
///
/// Every method swallows its errors and reports absence instead. That is the correct shape here
/// and nowhere else in this crate: a caller that cannot read the cache must fetch `robots.txt`
/// itself, which is slower but exactly as polite. A cache that could return an error would give
/// callers a third case to handle, and the tempting way to handle it is to skip the rules.
#[derive(Clone)]
pub struct RobotsCache {
    client: redis::Client,
}

impl RobotsCache {
    pub fn connect(url: &str) -> Option<Self> {
        match redis::Client::open(url) {
            Ok(client) => Some(Self { client }),
            Err(e) => {
                tracing::warn!(error = %e, "no shared robots cache; each worker will fetch its own");
                None
            }
        }
    }

    /// A cached entry for this host, if one is present and fresh.
    pub async fn get(&self, host: &str, now: i64) -> Option<CachedRobots> {
        let mut conn = self.client.get_multiplexed_async_connection().await.ok()?;
        let raw: Option<String> = redis::cmd("GET")
            .arg(key(host))
            .query_async(&mut conn)
            .await
            .ok()?;
        // An entry that will not deserialise is treated as absent rather than as an error. It is
        // almost certainly from an older build, and refetching is the right response either way.
        let entry: CachedRobots = serde_json::from_str(&raw?).ok()?;
        if entry.is_stale(now) {
            return None;
        }
        Some(entry)
    }

    /// Drop a host's cached rules so the next fetch re-reads them.
    ///
    /// Operationally useful — "this site says it changed its robots.txt, re-read it now" — and
    /// necessary for tests, which would otherwise inherit an entry from an earlier run and pass
    /// for the wrong reason. That is not hypothetical: it happened.
    pub async fn forget(&self, host: &str) {
        let Ok(mut conn) = self.client.get_multiplexed_async_connection().await else {
            return;
        };
        let _: Result<(), _> = redis::cmd("DEL")
            .arg(key(host))
            .query_async::<()>(&mut conn)
            .await;
    }

    pub async fn put(&self, host: &str, entry: &CachedRobots) {
        let Ok(mut conn) = self.client.get_multiplexed_async_connection().await else {
            return;
        };
        let Ok(payload) = serde_json::to_string(entry) else {
            return;
        };
        // Redis expiry is a backstop, not the freshness rule — `is_stale` decides that. Set well
        // past the TTL so a slightly stale entry is still readable and can be distinguished from
        // one that was never written, which matters when debugging why a host is being refetched.
        let ttl = TTL.as_secs() * 2;
        let _: Result<(), _> = redis::cmd("SET")
            .arg(key(host))
            .arg(payload)
            .arg("EX")
            .arg(ttl)
            .query_async::<()>(&mut conn)
            .await;
    }
}
