//! The crawl frontier.
//!
//! The set of URLs known and not yet fetched, plus when each host may next be touched. It lives in
//! Redis rather than in a process, for one reason that decides the whole shape: **politeness is a
//! property of a host, not of a worker.** Two workers each holding their own idea of when
//! `elkhabar.com` may next be fetched will between them hit it twice as often as either intends,
//! and neither will be able to tell that it happened.
//!
//! So the due-time is shared state, and claiming work is atomic.
//!
//! # Structure
//!
//! | Key | Type | Holds |
//! |:---|:---|:---|
//! | `frontier:hosts` | sorted set | host → unix millis when it may next be fetched |
//! | `frontier:q:{host}` | sorted set | url → priority score, low first |
//! | `frontier:seen` | set | url hashes, so a link found twice is queued once |
//! | `frontier:inflight` | hash | url → claim expiry, so a dead worker's work returns |
//!
//! Hosts are ordered by due-time, so "what may I fetch now" is a range query rather than a scan.
//! At a million URLs the difference is the whole design.
//!
//! # Why claims expire rather than being released
//!
//! A worker that dies mid-fetch cannot release anything. If a claim were held until released, that
//! URL would be lost and its host would stay blocked behind it. Claims therefore carry an expiry
//! and are reclaimed — the same reasoning as the queue's `XAUTOCLAIM`, and the same consequence:
//! **at-least-once, so fetching must be idempotent.** Fetching a page twice costs a request;
//! losing it costs a document forever.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// How long a claim is held before another worker may take it.
///
/// Comfortably longer than the fetch timeout, so a slow-but-alive fetch is never stolen — a stolen
/// claim means two workers fetching the same URL, which is precisely the impoliteness the frontier
/// exists to prevent.
pub const CLAIM_TTL: Duration = Duration::from_secs(120);

/// Cap on a single host's queue.
///
/// A site with a calendar or a faceted search can generate unbounded URLs, and without a cap one
/// host crowds out every other. Past this, new discoveries for that host are dropped: the frontier
/// is a working set, not an archive.
pub const MAX_PER_HOST: usize = 10_000;

/// Cap on total known URLs, as a backstop against the same failure across many hosts at once.
pub const MAX_TOTAL: usize = 5_000_000;

/// A URL waiting to be fetched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    pub url: String,
    pub host: String,
    /// Which seed or source this came from, for budget accounting.
    pub source_id: String,
    /// Links followed from the seed. Used by the trap detectors and by priority.
    pub depth: u32,
    /// Lower is sooner. Not a timestamp — an ordering.
    pub priority: i64,
}

/// Priority for a newly discovered URL.
///
/// Lower sorts first. The components are deliberately few, because a priority function nobody can
/// predict is one nobody can debug when the crawler spends a day somewhere useless.
///
/// - **Depth dominates.** A link from the homepage is more likely to be current than one four hops
///   in, and shallow-first keeps the crawl broad rather than falling down a single site.
/// - **Trust adjusts.** A source we rate highly is worth reaching sooner.
/// - **Article-shaped URLs win ties**, since a listing page mostly exists to point at them.
pub fn priority_for(depth: u32, trust: u8, looks_like_article: bool) -> i64 {
    let depth_cost = i64::from(depth) * 1_000;
    // Trust is 0–100; higher trust must lower the score.
    let trust_credit = i64::from(trust) * 5;
    let article_credit = if looks_like_article { 250 } else { 0 };
    depth_cost - trust_credit - article_credit
}

/// Whether a path looks like an article rather than a listing or a control page.
///
/// A heuristic, and only ever used to break ties. Being wrong costs ordering, not correctness.
pub fn looks_like_article(path: &str) -> bool {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() < 2 {
        return false;
    }
    let last = segments.last().copied().unwrap_or_default();
    // A date in the path, or a long hyphenated slug, is what article URLs look like almost
    // everywhere — including every Algerian news site in the seed list.
    let has_date = segments
        .iter()
        .any(|s| s.len() == 4 && s.starts_with("20") && s.chars().all(|c| c.is_ascii_digit()));
    let slug_like = last.len() > 20 && last.contains('-');
    has_date || slug_like
}

/// Why a discovered URL was not queued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejected {
    /// Already known. The common case by far.
    Seen,
    /// Off-site, when the crawl is scoped to a host.
    OffSite,
    /// Failed `SafeUrl`, a blocklist, or robots.
    NotPermitted,
    /// Tripped a trap detector.
    Trap(&'static str),
    /// The host's queue or the frontier as a whole is full.
    Full,
    /// Past the configured depth.
    TooDeep,
}

impl Rejected {
    /// Stable label for metrics.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Seen => "seen",
            Self::OffSite => "off_site",
            Self::NotPermitted => "not_permitted",
            Self::Trap(_) => "trap",
            Self::Full => "full",
            Self::TooDeep => "too_deep",
        }
    }
}

/// Maximum path segments before a URL is treated as a trap.
pub const MAX_DEPTH_SEGMENTS: usize = 12;
/// Maximum query parameters.
pub const MAX_QUERY_PARAMS: usize = 8;
/// How many times one path segment may repeat before it is a loop.
pub const MAX_SEGMENT_REPEATS: usize = 3;

/// Crawler-trap detection.
///
/// Traps are rarely malicious. A calendar with a "next month" link generates URLs forever; a
/// faceted shop multiplies filters; a misconfigured relative link produces `/a/b/a/b/a/b/…`. The
/// crawler cannot tell any of them from a large site, so it needs rules — and the cost of not
/// having them is a worker that never leaves one host while appearing to make progress.
pub fn detect_trap(url: &url::Url) -> Option<Rejected> {
    let segments: Vec<&str> = url.path().split('/').filter(|s| !s.is_empty()).collect();

    if segments.len() > MAX_DEPTH_SEGMENTS {
        return Some(Rejected::Trap("deep_path"));
    }

    if url.query_pairs().count() > MAX_QUERY_PARAMS {
        return Some(Rejected::Trap("many_params"));
    }

    // A repeating segment is the signature of a relative-link loop. Counted rather than looked for
    // adjacently, because `/a/b/a/b/a/` never repeats adjacently.
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for s in &segments {
        let n = counts.entry(*s).or_insert(0);
        *n += 1;
        if *n > MAX_SEGMENT_REPEATS {
            return Some(Rejected::Trap("repeating_segment"));
        }
    }

    // Session identifiers in the path or query make every visit a new URL, so the frontier grows
    // without ever covering anything new.
    const SESSION_KEYS: &[&str] = &["sessionid", "jsessionid", "phpsessid", "sid", "session_id"];
    for (k, _) in url.query_pairs() {
        if SESSION_KEYS.contains(&k.to_ascii_lowercase().as_str()) {
            return Some(Rejected::Trap("session_id"));
        }
    }
    if segments.iter().any(|s| {
        SESSION_KEYS
            .iter()
            .any(|k| s.to_ascii_lowercase().starts_with(k))
    }) {
        return Some(Rejected::Trap("session_id"));
    }

    None
}

/// Normalise a URL for the `seen` set.
///
/// Two URLs that fetch the same page must hash the same, or the frontier queues the same document
/// repeatedly and the politeness budget is spent re-reading what we already have.
///
/// Deliberately conservative: only things that provably do not change the response are removed. A
/// query parameter that looks like tracking usually is, but `?page=2` is not, and stripping the
/// wrong one silently loses pages.
pub fn canonical(url: &url::Url) -> String {
    let mut u = url.clone();
    u.set_fragment(None);

    // Tracking parameters. These are appended by the referrer, never read by the server, and
    // leaving them in means one article queued once per place it was linked from.
    const TRACKING: &[&str] = &[
        "utm_source",
        "utm_medium",
        "utm_campaign",
        "utm_term",
        "utm_content",
        "fbclid",
        "gclid",
        "mc_cid",
        "mc_eid",
        "ref",
        "_ga",
    ];
    let kept: Vec<(String, String)> = u
        .query_pairs()
        .filter(|(k, _)| !TRACKING.contains(&k.to_ascii_lowercase().as_str()))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if kept.is_empty() {
        u.set_query(None);
    } else {
        // Sorted, so `?a=1&b=2` and `?b=2&a=1` are one URL.
        let mut sorted = kept;
        sorted.sort();
        let query = sorted
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        u.set_query(Some(&query));
    }

    // A trailing slash on a directory is the same resource; on a file it is not, so only the
    // bare-root case is normalised.
    let s = u.to_string();
    s.strip_suffix('/')
        .filter(|t| !t.ends_with("//"))
        .map(str::to_string)
        .unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    fn u(s: &str) -> Url {
        Url::parse(s).expect("test url")
    }

    #[test]
    fn shallow_links_are_crawled_before_deep_ones() {
        // Keeps the crawl broad. Depth-first on one site means a week inside one archive while
        // nineteen other sources go untouched.
        assert!(priority_for(0, 50, false) < priority_for(1, 50, false));
        assert!(priority_for(1, 50, false) < priority_for(4, 50, false));
    }

    #[test]
    fn trust_and_article_shape_only_break_ties() {
        // Trust must not outweigh depth, or a single trusted site swallows the crawl.
        assert!(priority_for(0, 0, false) < priority_for(1, 100, true));
        // Within a depth, trust and shape order things.
        assert!(priority_for(2, 90, false) < priority_for(2, 10, false));
        assert!(priority_for(2, 50, true) < priority_for(2, 50, false));
    }

    #[test]
    fn article_shaped_paths_are_recognised() {
        assert!(looks_like_article("/2026/08/president-visits-oran"));
        assert!(looks_like_article(
            "/actualite/une-longue-histoire-de-la-ville"
        ));
        // Listings and control pages are not.
        assert!(!looks_like_article("/"));
        assert!(!looks_like_article("/sports"));
        assert!(!looks_like_article("/page/2"));
    }

    #[test]
    fn a_calendar_that_generates_urls_forever_is_a_trap() {
        // Not malicious, and the crawler cannot tell it from a large site without a rule.
        assert!(detect_trap(&u("https://e.dz/a/b/c/d/e/f/g/h/i/j/k/l/m/n")).is_some());
    }

    #[test]
    fn a_relative_link_loop_is_caught_even_when_not_adjacent() {
        // `/a/b/a/b/a/b/` never repeats adjacently, which is what a naive check looks for.
        assert!(detect_trap(&u("https://e.dz/a/b/a/b/a/b/a/b")).is_some());
        // A path that merely reuses a word twice is fine.
        assert!(detect_trap(&u("https://e.dz/news/news-of-the-day")).is_none());
    }

    #[test]
    fn faceted_search_is_a_trap() {
        assert!(detect_trap(&u("https://e.dz/shop?a=1&b=2&c=3&d=4&e=5&f=6&g=7&h=8&i=9")).is_some());
    }

    #[test]
    fn session_identifiers_make_every_visit_a_new_url() {
        assert!(detect_trap(&u("https://e.dz/a?sessionid=abc")).is_some());
        assert!(detect_trap(&u("https://e.dz/a?PHPSESSID=abc")).is_some());
        assert!(detect_trap(&u("https://e.dz/jsessionid=abc/page")).is_some());
    }

    #[test]
    fn ordinary_urls_are_not_traps() {
        // The other direction. A trap detector that refuses real pages is worse than none: the
        // symptom is a thin index with nothing to explain it.
        for s in [
            "https://www.aps.dz/",
            "https://www.elkhabar.com/press/article/12345-titre-de-larticle/",
            "https://horizons.dz/2026/08/09/une-nouvelle/",
            "https://e.dz/search?q=algerie&page=2",
        ] {
            assert!(detect_trap(&u(s)).is_none(), "{s} was called a trap");
        }
    }

    #[test]
    fn tracking_parameters_do_not_make_a_new_url() {
        // One article linked from five places would otherwise be queued five times, and the
        // politeness budget spent re-reading what we already hold.
        let a = canonical(&u("https://e.dz/article?utm_source=fb&utm_campaign=x"));
        let b = canonical(&u("https://e.dz/article"));
        assert_eq!(a, b);
        assert_eq!(
            canonical(&u("https://e.dz/a#section")),
            canonical(&u("https://e.dz/a"))
        );
    }

    #[test]
    fn meaningful_parameters_are_kept() {
        // `?page=2` is a different page. Stripping it silently loses everything after the first.
        assert_ne!(
            canonical(&u("https://e.dz/list?page=2")),
            canonical(&u("https://e.dz/list"))
        );
    }

    #[test]
    fn parameter_order_does_not_make_a_new_url() {
        assert_eq!(
            canonical(&u("https://e.dz/a?b=2&a=1")),
            canonical(&u("https://e.dz/a?a=1&b=2"))
        );
    }

    #[test]
    fn rejections_have_stable_labels() {
        for r in [
            Rejected::Seen,
            Rejected::OffSite,
            Rejected::NotPermitted,
            Rejected::Trap("deep_path"),
            Rejected::Full,
            Rejected::TooDeep,
        ] {
            assert!(!r.as_str().is_empty() && !r.as_str().contains(' '));
        }
    }

    #[test]
    fn a_claim_outlives_a_slow_fetch() {
        // A claim that expired mid-fetch would be taken by a second worker, and two workers
        // fetching one URL is exactly the impoliteness the frontier exists to prevent.
        assert!(CLAIM_TTL > Duration::from_secs(30));
    }
}
