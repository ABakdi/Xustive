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
    /// How much we rate the source, 0–100. Inherited by whatever this page links to.
    pub trust: u8,
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

/// Priority for a page we are scheduling to see again.
///
/// M2-T15.7. Discovery priority answers "which unseen URL first"; this answers a different
/// question — "which page we already hold is most worth re-reading" — and the inputs differ
/// accordingly. Depth and trust still set the base, and two revisit-only signals adjust it:
///
/// - **How often it changes**, read off the interval the scheduler has converged on. A page held
///   near its floor is one we have *measured* as changing, which is better evidence than any
///   guess from its URL.
/// - **How overdue it is.** Being late is the part that actually costs freshness, and without it a
///   trusted page could sit behind a stream of new discoveries indefinitely.
///
/// Both credits are capped, deliberately. Uncapped, a volatile page would outrank everything on
/// its host forever — and a page that changes on every visit is precisely the one the Cho result
/// says not to chase. The caps keep the two signals as adjustments to an ordering rather than as
/// an ordering of their own, which is also what keeps this function predictable enough to debug
/// from a document count that stopped rising.
pub fn priority_for_revisit(depth: u32, trust: u8, interval_secs: u64, overdue_secs: i64) -> i64 {
    let base = priority_for(depth, trust, false);

    // Bands rather than a curve. A continuous function of the interval reads better and is far
    // harder to reason about at 2am; the bands are hours, a day, a week, and beyond.
    let change_credit = match interval_secs {
        0..=7_200 => 400,
        7_201..=86_400 => 200,
        86_401..=604_800 => 50,
        _ => 0,
    };

    // An hour late is worth little, a week late is worth a lot, and beyond twelve days more
    // lateness buys nothing — past that the page is not competing with anything, it is simply
    // waiting for its host.
    let overdue_credit = (overdue_secs / 3_600).clamp(0, 300);

    base - change_credit - overdue_credit
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
    fn host_case_and_default_ports_are_normalised() {
        // Host and scheme are case-insensitive, and :443/:80 are the defaults — three spellings of
        // one resource that would otherwise be crawled three times. url::Url folds these on parse;
        // this pins that we rely on it, so a parser swap cannot silently reintroduce the duplicates.
        assert_eq!(
            canonical(&u("https://Example.DZ/Article")),
            canonical(&u("https://example.dz/Article"))
        );
        assert_eq!(
            canonical(&u("https://e.dz:443/x")),
            canonical(&u("https://e.dz/x"))
        );
        assert_eq!(
            canonical(&u("http://e.dz:80/x")),
            canonical(&u("http://e.dz/x"))
        );
        // Path case is preserved — servers may treat /Article and /article as different resources.
        assert_ne!(
            canonical(&u("https://e.dz/Article")),
            canonical(&u("https://e.dz/article"))
        );
    }

    #[test]
    fn query_order_does_not_make_a_new_url() {
        assert_eq!(
            canonical(&u("https://e.dz/a?b=2&a=1")),
            canonical(&u("https://e.dz/a?a=1&b=2"))
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

// ── The Redis-backed frontier ───────────────────────────────────────────────────────────────

/// Default key namespace.
///
/// Namespaced rather than fixed, so two crawls can share one Redis without colliding — a staging
/// run against a copy of production, or several test binaries in parallel. The tests needed it
/// first: sharing one namespace, they wiped each other's state and failed in ways that looked like
/// concurrency bugs in the frontier itself.
pub const DEFAULT_NAMESPACE: &str = "frontier";

/// Claim the next due URL, atomically.
///
/// One script rather than several commands, because the sequence — find a due host, pop its
/// cheapest URL, push its due-time forward, record the claim — must not interleave with another
/// worker doing the same. Done as separate round trips, two workers routinely pick the same host
/// between the read and the write, and both fetch it. That is the exact failure the frontier
/// exists to prevent, and it appears only under concurrency, which is where it is hardest to see.
///
/// Returns the claimed URL, or nothing when no host is due.
const CLAIM_SCRIPT: &str = r#"
local now = tonumber(ARGV[1])
local delay_ms = tonumber(ARGV[2])
local claim_until = tonumber(ARGV[3])

-- The earliest host whose due-time has passed.
local due = redis.call('ZRANGEBYSCORE', KEYS[1], '-inf', now, 'LIMIT', 0, 1)
if #due == 0 then return nil end
local host = due[1]

local q = ARGV[4] .. ':q:' .. host
local item = redis.call('ZPOPMIN', q, 1)
if #item == 0 then
  -- The host is due but has nothing left. Drop it so it stops being considered.
  redis.call('ZREM', KEYS[1], host)
  return nil
end
local url = item[1]

-- Push the host's next slot forward *before* returning, so a second worker arriving in the same
-- millisecond sees it as not due rather than claiming a second URL from it.
redis.call('ZADD', KEYS[1], now + delay_ms, host)
redis.call('HSET', KEYS[3], url, claim_until)
-- Read the metadata inside the script rather than as a follow-up GET: a separate round trip could
-- observe a `complete` from another worker in between and hand back a URL with no depth, which
-- silently resets that branch of the crawl to the root.
-- Lua truncates a table at the first nil, so an absent entry must become a string here.
local meta = redis.call('HGET', KEYS[2], url)
return {host, url, meta or ''}
"#;

/// Return claims whose worker never came back.
///
/// A worker that dies mid-fetch cannot release anything, so claims expire and are swept. Without
/// this the URL is lost and, worse, nothing indicates it: the frontier simply gets quietly smaller.
const RECLAIM_SCRIPT: &str = r#"
local now = tonumber(ARGV[1])
local reclaimed = 0
local claims = redis.call('HGETALL', KEYS[1])
for i = 1, #claims, 2 do
  local url = claims[i]
  local expiry = tonumber(claims[i + 1])
  if expiry and expiry < now then
    redis.call('HDEL', KEYS[1], url)
    reclaimed = reclaimed + 1
  end
end
return reclaimed
"#;

#[derive(Clone)]
pub struct Frontier {
    client: redis::Client,
    namespace: String,
}

/// A claimed URL. Dropping it does **not** release the claim — the claim expires instead, so a
/// crashed worker behaves the same as a slow one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub host: String,
    pub url: String,
    /// Links followed from a seed to reach this URL.
    ///
    /// Carried through the frontier because it cannot be recovered from the URL itself. Without
    /// it the orchestrator has no parent depth to increment, every discovery looks like depth 1,
    /// and `max_depth` can never fire.
    pub depth: u32,
    /// The seed this descends from, so budgets and provenance survive link-following.
    pub source_id: String,
    /// Inherited from the parent. A link from a source we rate highly is worth reaching sooner
    /// than one from a source we do not.
    pub trust: u8,
}

/// Per-URL state the queue itself cannot hold.
///
/// The host queue is a sorted set of URLs scored by priority, so there is nowhere in it to put
/// depth. Encoded as one field rather than three keys because it is written on every discovery and
/// read on every claim, and at five million URLs that ratio decides the cost.
///
/// `source_id` is last so it may contain anything; the numbers before it are parsed positionally.
fn encode_meta(depth: u32, trust: u8, source_id: &str) -> String {
    format!("{depth}\t{trust}\t{source_id}")
}

/// Decode what `encode_meta` wrote, falling back to a root-level default.
///
/// A missing or unparsable entry yields depth 0, which is the **conservative** direction: it makes
/// the crawler treat the URL as a seed and keep going. Defaulting to `max_depth` instead would
/// make one bad write look like a crawl that mysteriously stops following links.
fn decode_meta(raw: &str) -> (u32, u8, String) {
    let mut parts = raw.splitn(3, '\t');
    let depth = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let trust = parts.next().and_then(|s| s.parse().ok()).unwrap_or(50);
    let source_id = parts.next().unwrap_or_default().to_string();
    (depth, trust, source_id)
}

impl Frontier {
    pub fn connect(url: &str) -> Result<Self, redis::RedisError> {
        Self::connect_in(url, DEFAULT_NAMESPACE)
    }

    pub fn connect_in(url: &str, namespace: &str) -> Result<Self, redis::RedisError> {
        Ok(Self {
            client: redis::Client::open(url)?,
            namespace: namespace.to_string(),
        })
    }

    fn k_hosts(&self) -> String {
        format!("{}:hosts", self.namespace)
    }
    fn k_seen(&self) -> String {
        format!("{}:seen", self.namespace)
    }
    fn k_inflight(&self) -> String {
        format!("{}:inflight", self.namespace)
    }
    fn k_meta(&self) -> String {
        format!("{}:meta", self.namespace)
    }
    fn k_due(&self) -> String {
        format!("{}:due", self.namespace)
    }
    fn host_queue(&self, host: &str) -> String {
        format!("{}:q:{host}", self.namespace)
    }

    async fn conn(&self) -> Option<redis::aio::MultiplexedConnection> {
        self.client.get_multiplexed_async_connection().await.ok()
    }

    /// Add a URL, unless it is already known.
    ///
    /// Returns whether it was added. `seen` is checked and set in the same call so two workers
    /// discovering the same link do not both queue it.
    pub async fn add(&self, pending: &Pending) -> Result<bool, Rejected> {
        let Some(mut conn) = self.conn().await else {
            return Err(Rejected::Full);
        };

        let fresh: i64 = redis::cmd("SADD")
            .arg(self.k_seen())
            .arg(&pending.url)
            .query_async(&mut conn)
            .await
            .unwrap_or(0);
        if fresh == 0 {
            return Ok(false);
        }

        // Named for what it is. This is how many URLs the host already has queued, not the crawl
        // depth of anything — a distinction that now matters, since `Pending::depth` is real.
        let queued_for_host: usize = redis::cmd("ZCARD")
            .arg(self.host_queue(&pending.host))
            .query_async(&mut conn)
            .await
            .unwrap_or(0);
        if queued_for_host >= MAX_PER_HOST {
            return Err(Rejected::Full);
        }

        // Errors here are **not** swallowed, unlike most of this file.
        //
        // `seen` was already written, so a silent failure leaves the URL marked as known with
        // nothing queued — and it is then never retried, because every future discovery sees it in
        // `seen` and stops. One dropped write loses a URL permanently and reports success.
        //
        // Observed: a source added from the admin console landed in `seen` with no host queue and
        // no due-time, and the endpoint returned `queued: true`.
        let queued: Result<(), _> = redis::cmd("ZADD")
            .arg(self.host_queue(&pending.host))
            .arg(pending.priority)
            .arg(&pending.url)
            .query_async::<()>(&mut conn)
            .await;
        // Depth travels with the URL, not with the worker that found it. Written before the host
        // becomes due, so a claim can never arrive ahead of the metadata it needs.
        let meta: Result<(), _> = redis::cmd("HSET")
            .arg(self.k_meta())
            .arg(&pending.url)
            .arg(encode_meta(
                pending.depth,
                pending.trust,
                &pending.source_id,
            ))
            .query_async::<()>(&mut conn)
            .await;
        // `NX` so an existing due-time is never pushed backwards by a new discovery — a host that
        // is due now must stay due, or a steady trickle of links would starve it forever.
        let host_added: Result<(), _> = redis::cmd("ZADD")
            .arg(self.k_hosts())
            .arg("NX")
            .arg(0)
            .arg(&pending.host)
            .query_async::<()>(&mut conn)
            .await;

        if let Err(e) = queued.and(meta).and(host_added) {
            // Undo the `seen` write so the URL can be discovered again. Leaving it would make one
            // transient Redis error a permanent hole in the crawl.
            let _: Result<(), _> = redis::cmd("SREM")
                .arg(self.k_seen())
                .arg(&pending.url)
                .query_async::<()>(&mut conn)
                .await;
            // And the metadata, or a failed add leaves an orphan that nothing will ever claim or
            // clear. `seen` is what gates rediscovery, so this is tidiness rather than correctness
            // — but the hash is unbounded and nobody sweeps it.
            let _: Result<(), _> = redis::cmd("HDEL")
                .arg(self.k_meta())
                .arg(&pending.url)
                .query_async::<()>(&mut conn)
                .await;
            tracing::warn!(url = %pending.url, error = %e, "could not queue; will be retried");
            return Err(Rejected::Full);
        }
        Ok(true)
    }

    /// Schedule a URL to be crawled again at a given time.
    ///
    /// # Why this is not just `add`
    ///
    /// `add` refuses anything already in `seen`, which is exactly right for discovery — a link
    /// found on forty listing pages must be queued once. A revisit is the opposite case: the URL is
    /// in `seen` *because* we crawled it, and that is the precondition rather than the objection.
    ///
    /// # Why a separate set rather than a due-time on the queue entry
    ///
    /// The host queue is scored by priority, and priority is an ordering, not a time; overloading
    /// it would mean a page due next month sorting ahead of one due now whenever its trust was
    /// higher. Deferred URLs therefore wait in their own sorted set keyed by due time, and
    /// [`Frontier::promote_due`] moves them into the host queue when they come round.
    ///
    /// That keeps the claim path unchanged and O(1)-ish: no per-URL time check on the hot path,
    /// which at five million URLs is the difference between a range query and a scan.
    pub async fn defer(&self, pending: &Pending, due_ms: i64) {
        let Some(mut conn) = self.conn().await else {
            return;
        };
        // Metadata is rewritten, not assumed to survive: `complete` clears it when the previous
        // visit finished, so a deferred URL with no metadata would come back at depth 0 and be
        // treated as a seed.
        let _: Result<(), _> = redis::cmd("HSET")
            .arg(self.k_meta())
            .arg(&pending.url)
            .arg(encode_meta(
                pending.depth,
                pending.trust,
                &pending.source_id,
            ))
            .query_async::<()>(&mut conn)
            .await;
        let _: Result<(), _> = redis::cmd("ZADD")
            .arg(self.k_due())
            .arg(due_ms)
            .arg(format!(
                "{}\t{}\t{}",
                pending.host, pending.priority, pending.url
            ))
            .query_async::<()>(&mut conn)
            .await;
    }

    /// Move every URL whose due time has passed into its host queue.
    ///
    /// Called on the same cadence as claim reclamation rather than per step. A page due a second
    /// ago is not urgent, and putting a range query on the hot path to find out would cost more
    /// than the second it saves.
    ///
    /// Returns how many were promoted.
    pub async fn promote_due(&self, now_ms: i64, limit: usize) -> usize {
        let Some(mut conn) = self.conn().await else {
            return 0;
        };
        let entries: Vec<String> = redis::cmd("ZRANGEBYSCORE")
            .arg(self.k_due())
            .arg("-inf")
            .arg(now_ms)
            .arg("LIMIT")
            .arg(0)
            .arg(limit)
            .query_async(&mut conn)
            .await
            .unwrap_or_default();

        let mut promoted = 0usize;
        for entry in entries {
            let mut parts = entry.splitn(3, '\t');
            let (Some(host), Some(priority), Some(url)) =
                (parts.next(), parts.next(), parts.next())
            else {
                // Unparsable, so it can never be promoted. Drop it rather than leave it to be
                // re-read on every sweep forever.
                let _: Result<(), _> = redis::cmd("ZREM")
                    .arg(self.k_due())
                    .arg(&entry)
                    .query_async::<()>(&mut conn)
                    .await;
                continue;
            };
            let priority: i64 = priority.parse().unwrap_or(0);

            let queued: Result<(), _> = redis::cmd("ZADD")
                .arg(self.host_queue(host))
                .arg(priority)
                .arg(url)
                .query_async::<()>(&mut conn)
                .await;
            // `NX` so a host that is already due stays due. Without it, every promotion would push
            // the host's next slot forward and a busy host would starve itself.
            let host_added: Result<(), _> = redis::cmd("ZADD")
                .arg(self.k_hosts())
                .arg("NX")
                .arg(0)
                .arg(host)
                .query_async::<()>(&mut conn)
                .await;

            // Removed only once it is queued. A crash between the two costs a duplicate fetch,
            // which is a request; the other order costs the URL, which is a document forever.
            if queued.and(host_added).is_ok() {
                let _: Result<(), _> = redis::cmd("ZREM")
                    .arg(self.k_due())
                    .arg(&entry)
                    .query_async::<()>(&mut conn)
                    .await;
                promoted += 1;
            }
        }
        promoted
    }

    /// How many URLs are waiting for their due time.
    pub async fn deferred(&self) -> usize {
        let Some(mut conn) = self.conn().await else {
            return 0;
        };
        redis::cmd("ZCARD")
            .arg(self.k_due())
            .query_async(&mut conn)
            .await
            .unwrap_or(0)
    }

    /// Move a URL to the head of its host's queue.
    ///
    /// Ordering only. The URL still passes every check on the way in, and the host's crawl-delay
    /// still applies — an operator being impatient does not make a site answer faster.
    pub async fn promote(&self, host: &str, url: &str) {
        let Some(mut conn) = self.conn().await else {
            return;
        };
        let _: Result<(), _> = redis::cmd("ZADD")
            .arg(self.host_queue(host))
            .arg("XX")
            .arg(i64::MIN)
            .arg(url)
            .query_async::<()>(&mut conn)
            .await;
    }

    /// Claim the next due URL, or nothing.
    pub async fn claim(&self, now_ms: i64, host_delay: Duration) -> Option<Claim> {
        let mut conn = self.conn().await?;
        let result: Option<Vec<String>> = redis::Script::new(CLAIM_SCRIPT)
            .key(self.k_hosts())
            .key(self.k_meta())
            .key(self.k_inflight())
            .arg(now_ms)
            .arg(host_delay.as_millis() as i64)
            .arg(now_ms + CLAIM_TTL.as_millis() as i64)
            .arg(&self.namespace)
            .invoke_async(&mut conn)
            .await
            .ok()?;
        let parts = result?;
        let (depth, trust, source_id) = decode_meta(parts.get(2).map(String::as_str).unwrap_or(""));
        Some(Claim {
            host: parts.first()?.clone(),
            url: parts.get(1)?.clone(),
            depth,
            trust,
            source_id,
        })
    }

    /// Mark a claimed URL finished. Idempotent.
    pub async fn complete(&self, url: &str) {
        let Some(mut conn) = self.conn().await else {
            return;
        };
        let _: Result<(), _> = redis::cmd("HDEL")
            .arg(self.k_inflight())
            .arg(url)
            .query_async::<()>(&mut conn)
            .await;
        // The metadata dies with the claim. It exists to carry depth from discovery to fetch, and
        // once fetched the URL is in `seen` and will not be queued again — so keeping it would grow
        // a hash the size of every URL ever crawled.
        let _: Result<(), _> = redis::cmd("HDEL")
            .arg(self.k_meta())
            .arg(url)
            .query_async::<()>(&mut conn)
            .await;
    }

    /// Sweep expired claims. Returns how many were released.
    pub async fn reclaim(&self, now_ms: i64) -> usize {
        let Some(mut conn) = self.conn().await else {
            return 0;
        };
        redis::Script::new(RECLAIM_SCRIPT)
            .key(self.k_inflight())
            .arg(now_ms)
            .invoke_async(&mut conn)
            .await
            .unwrap_or(0)
    }

    /// How many URLs are waiting, and how many are in flight.
    pub async fn depth(&self) -> (usize, usize) {
        let Some(mut conn) = self.conn().await else {
            return (0, 0);
        };
        let hosts: Vec<String> = redis::cmd("ZRANGE")
            .arg(self.k_hosts())
            .arg(0)
            .arg(-1)
            .query_async(&mut conn)
            .await
            .unwrap_or_default();
        let mut waiting = 0usize;
        for h in hosts {
            let n: usize = redis::cmd("ZCARD")
                .arg(self.host_queue(&h))
                .query_async(&mut conn)
                .await
                .unwrap_or(0);
            waiting += n;
        }
        let inflight: usize = redis::cmd("HLEN")
            .arg(self.k_inflight())
            .query_async(&mut conn)
            .await
            .unwrap_or(0);
        (waiting, inflight)
    }

    /// Remove everything. Tests and a deliberate operator reset only.
    pub async fn clear(&self) {
        let Some(mut conn) = self.conn().await else {
            return;
        };
        let hosts: Vec<String> = redis::cmd("ZRANGE")
            .arg(self.k_hosts())
            .arg(0)
            .arg(-1)
            .query_async(&mut conn)
            .await
            .unwrap_or_default();
        for h in hosts {
            let _: Result<(), _> = redis::cmd("DEL")
                .arg(self.host_queue(&h))
                .query_async::<()>(&mut conn)
                .await;
        }
        for k in [
            self.k_hosts(),
            self.k_seen(),
            self.k_inflight(),
            self.k_meta(),
            self.k_due(),
        ] {
            let _: Result<(), _> = redis::cmd("DEL").arg(k).query_async::<()>(&mut conn).await;
        }
    }
}

#[cfg(test)]
mod revisit_priority {
    use super::*;

    const HOUR: i64 = 3_600;

    #[test]
    fn a_page_that_changes_often_is_re_read_sooner() {
        let volatile = priority_for_revisit(1, 50, 3_600, 0);
        let stable = priority_for_revisit(1, 50, 30 * 24 * 3_600, 0);
        assert!(
            volatile < stable,
            "a page on a one-hour interval should sort ahead of one on a monthly interval"
        );
    }

    #[test]
    fn being_overdue_moves_a_page_up() {
        let punctual = priority_for_revisit(1, 50, 86_400, 0);
        let late = priority_for_revisit(1, 50, 86_400, 48 * HOUR);
        assert!(late < punctual, "lateness is what actually costs freshness");
    }

    #[test]
    fn trust_still_matters_between_pages_that_change_alike() {
        let ministry = priority_for_revisit(1, 95, 86_400, 0);
        let stray = priority_for_revisit(1, 10, 86_400, 0);
        assert!(ministry < stray);
    }

    /// The credits are adjustments, not an ordering of their own.
    ///
    /// Uncapped, a page changing every hour would outrank everything on its host forever — and
    /// that is exactly the page the Cho result says not to chase. Depth must still dominate.
    #[test]
    fn a_volatile_deep_page_does_not_outrank_a_shallow_stable_one() {
        let deep_volatile = priority_for_revisit(4, 50, 3_600, 365 * 24 * HOUR);
        let shallow_stable = priority_for_revisit(0, 50, 30 * 24 * 3_600, 0);
        assert!(
            shallow_stable < deep_volatile,
            "depth must still dominate, or one churning archive page swallows the crawl"
        );
    }

    /// Lateness beyond the cap buys nothing, so a page abandoned for a year cannot accumulate
    /// unbounded priority and jump the queue when it is finally promoted.
    #[test]
    fn overdue_credit_is_bounded() {
        let a = priority_for_revisit(1, 50, 86_400, 20 * 24 * HOUR);
        let b = priority_for_revisit(1, 50, 86_400, 3650 * 24 * HOUR);
        assert_eq!(a, b);
    }
}
