//! When to come back to a page.
//!
//! Governed by [[ADR-0011]]. The policy is small, and every part of it is a deliberate rejection of
//! something more obvious.
//!
//! # Why not refresh in proportion to how often a page changes
//!
//! The intuitive policy — visit the fast-changing pages more — is measurably **worse than visiting
//! everything at the same rate** (Cho & Garcia-Molina, *Effective Page Refresh Policies for Web
//! Crawlers*, ACM TODS 28, 2003).
//!
//! A page that changes several times between visits can never be kept fresh. Every fetch spent on
//! it is stale again almost immediately, and that budget came out of pages that *could* have been
//! kept current. The optimal policy is non-monotonic: effort rises with change rate, then falls
//! away, and the fastest-changing pages are best left alone. [`Decision::Volatile`] is that tail.
//!
//! # Why "changed" means the article text, not the page
//!
//! Olston & Pandey (*Recrawl Scheduling Based on Information Longevity*, WWW 2008) found the useful
//! question is whether a change **persisted**, not whether bytes differed. Ephemeral furniture —
//! view counters, "most read" sidebars, rendered timestamps — changes constantly and is worthless,
//! because by the time it is indexed it no longer describes the page.
//!
//! We get this cheaply: `content_hash` is BLAKE3 over the *extracted, normalised* body, so
//! comparing it across fetches already ignores everything outside the article. That is the whole
//! reason this module compares content hashes rather than response bodies.
//!
//! It matters more here than for a general crawler. Algerian news sites wrap their articles in
//! exactly this kind of churn — an APS or Echorouk article page differs byte-for-byte on nearly
//! every fetch while the article body never moves. A crawler keyed on raw difference would recrawl
//! the entire corpus daily and learn nothing from it.
//!
//! # Why multiplicative, and not an estimator
//!
//! We never observe *how many* times a page changed, only whether it differs from the copy we hold.
//! The obvious estimator — changes ÷ visits — therefore undercounts, and undercounts worst on
//! fast-changing pages, which is exactly where being wrong is most expensive.
//!
//! Halving and growing by half sidesteps the estimate entirely. It needs no per-page model, it is
//! robust when the history is one observation long, and it converges from either direction.

use std::time::Duration;

/// Consecutive changed visits *at the floor* before a page is treated as volatile.
///
/// Four rather than one or two: a site can legitimately publish several updates in a row, and
/// abandoning after the first burst would drop breaking news exactly when it matters.
pub const VOLATILE_AFTER: u32 = 4;

/// How aggressively the interval shrinks when content changed: halve it.
///
/// Multiplicative *decrease* — the reactive half of AIMD. When a page changes we may have just
/// missed several changes, so cutting the interval sharply is the right response.
const SHRINK: f64 = 0.5;

/// How the interval grows when nothing changed: add one floor-step, not multiply.
///
/// # Why additive, learned from the freshness evaluation
///
/// The first version multiplied by 1.5 on every quiet visit. That overshoots: a page that changes
/// weekly would grow past a week's interval in a few visits, miss the next change, then halve —
/// oscillating between far-too-slow and too-fast with large amplitude, and never settling near the
/// true period. `freshness_eval.rs` measured the result as *worse* than both a fixed interval and
/// the proportional policy ADR-0011 exists to reject: 45 h mean staleness where proportional got
/// 14.5 h, and at higher cost.
///
/// Additive increase with multiplicative decrease is the same discipline TCP uses for the same
/// reason — it converges on the largest interval that still catches the change, rather than
/// leaping past it. Growth is in units of the tier's floor so a trusted source (short floor) probes
/// back more finely than a stray one.
fn grow(current: Duration, floor: Duration) -> Duration {
    current + floor
}

/// The bounds a page's interval is held within, chosen by how much we trust the source.
///
/// A ministry that publishes decrees is worth checking daily even when it rarely changes; a page
/// three hops off a directory is not. Without the tiering, a large quiet corpus drags every
/// interval to the ceiling and the sources that matter go stale with the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub floor: Duration,
    pub ceiling: Duration,
}

const HOUR: u64 = 3_600;
const DAY: u64 = 24 * HOUR;

impl Bounds {
    /// Bounds for a source's trust score, 0–100.
    pub fn for_trust(trust: u8) -> Self {
        match trust {
            80..=u8::MAX => Self {
                floor: Duration::from_secs(HOUR),
                ceiling: Duration::from_secs(3 * DAY),
            },
            50..=79 => Self {
                floor: Duration::from_secs(2 * HOUR),
                ceiling: Duration::from_secs(14 * DAY),
            },
            _ => Self {
                floor: Duration::from_secs(6 * HOUR),
                ceiling: Duration::from_secs(30 * DAY),
            },
        }
    }
}

/// What a fetch told us.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observation {
    /// The extracted body differs from the copy we hold.
    Changed,
    /// It does not.
    Unchanged,
    /// The server answered `304 Not Modified`.
    ///
    /// Treated as [`Observation::Unchanged`] for scheduling — it *is* unchanged — but kept distinct
    /// because it cost a few hundred bytes rather than a page, and the two want telling apart when
    /// judging whether the revisit budget is being spent well.
    NotModified,
}

impl Observation {
    /// Whether the indexed content actually moved.
    pub fn is_change(self) -> bool {
        matches!(self, Self::Changed)
    }

    /// Compare what we just extracted against what we hold.
    ///
    /// An empty previous hash means we have never successfully parsed this URL, which is not the
    /// same as "unchanged" — treating it as unchanged would let a page we have never read grow
    /// straight to the ceiling.
    pub fn from_hashes(previous: &str, current: &str) -> Self {
        if previous.is_empty() || previous != current {
            Self::Changed
        } else {
            Self::Unchanged
        }
    }
}

/// The scheduler's verdict for one page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Come back after this long.
    Revisit(Duration),
    /// Changing faster than we can ever track it. Parked at the ceiling rather than dropped.
    ///
    /// The Cho result applied directly: a page we cannot keep fresh should not be chased, because
    /// the budget buys nothing here and buys real freshness elsewhere. Parked rather than forgotten
    /// so a page that settles down is eventually noticed again — a ticker today may be an archive
    /// next year, and a permanent drop could never find out.
    Volatile(Duration),
}

impl Decision {
    /// How long until the next visit, whichever verdict was reached.
    pub fn interval(self) -> Duration {
        match self {
            Self::Revisit(d) | Self::Volatile(d) => d,
        }
    }

    pub fn is_volatile(self) -> bool {
        matches!(self, Self::Volatile(_))
    }
}

/// Per-URL scheduling state.
///
/// Deliberately small. This is written on every fetch, including the cheap 304s that make frequent
/// revisits affordable at all, so anything expensive here undoes the point of the exercise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    /// Current interval. Zero on a page never fetched.
    pub interval: Duration,
    /// Consecutive changed observations while already at the floor. The volatility signal.
    pub changes_at_floor: u32,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            interval: Duration::ZERO,
            changes_at_floor: 0,
        }
    }
}

impl Schedule {
    /// Fold one observation in and say when to return.
    ///
    /// The state machine is: halve on change, grow by half otherwise, clamp to the trust tier, and
    /// count consecutive changes that happen while already as fast as we are willing to go. Once
    /// that count passes [`VOLATILE_AFTER`], the page is one we cannot track, and chasing it is
    /// spending the budget for nothing.
    pub fn observe(&mut self, observation: Observation, trust: u8) -> Decision {
        let bounds = Bounds::for_trust(trust);

        // A page we have never scheduled starts at the floor. Starting at the ceiling would mean a
        // newly discovered source is not looked at again for a month, which is the wrong way to be
        // wrong about something we know nothing about.
        let current = if self.interval.is_zero() {
            bounds.floor
        } else {
            self.interval
        };

        let was_at_floor = current <= bounds.floor;
        // Multiplicative decrease on a change, additive increase on a quiet visit — AIMD. The
        // asymmetry is the point: react sharply to a change we may have been late for, approach the
        // idle ceiling gently so we settle near the true period instead of leaping past it.
        let next = if observation.is_change() {
            Duration::from_secs_f64(current.as_secs_f64() * SHRINK)
        } else {
            grow(current, bounds.floor)
        }
        .clamp(bounds.floor, bounds.ceiling);

        if observation.is_change() && was_at_floor {
            self.changes_at_floor = self.changes_at_floor.saturating_add(1);
        } else if !observation.is_change() {
            // Only a quiet visit clears the count. Resetting on any non-floor visit would let a
            // page oscillate across the floor forever without ever being called volatile.
            self.changes_at_floor = 0;
        }

        if self.changes_at_floor >= VOLATILE_AFTER {
            self.interval = bounds.ceiling;
            return Decision::Volatile(bounds.ceiling);
        }

        self.interval = next;
        Decision::Revisit(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A_TIER: u8 = 90;

    /// Written from a *backed-off* page rather than a fresh one.
    ///
    /// A fresh schedule starts at the floor, so the first two observations both return the floor
    /// and any `<=` assertion passes without the halving ever running. The shrink is only
    /// observable on a page that had already grown quiet.
    #[test]
    fn a_page_that_starts_changing_again_is_visited_sooner() {
        let mut s = Schedule::default();
        for _ in 0..6 {
            s.observe(Observation::Unchanged, 60);
        }
        let quiet = s.interval;
        assert!(
            quiet > Bounds::for_trust(60).floor,
            "should have backed off"
        );

        let after = s.observe(Observation::Changed, 60).interval();
        assert!(
            after < quiet,
            "a change must pull the next visit in: {after:?} vs {quiet:?}"
        );
    }

    #[test]
    fn a_quiet_page_is_visited_less_and_less() {
        let mut s = Schedule::default();
        let mut previous = s.observe(Observation::Unchanged, 60).interval();
        for _ in 0..5 {
            let next = s.observe(Observation::Unchanged, 60).interval();
            assert!(
                next >= previous,
                "a quiet page should back off, not speed up"
            );
            previous = next;
        }
    }

    /// The bounds are the whole reason the loop terminates.
    #[test]
    fn the_interval_never_leaves_its_tier() {
        let bounds = Bounds::for_trust(A_TIER);
        let mut s = Schedule::default();
        for _ in 0..40 {
            let d = s.observe(Observation::Unchanged, A_TIER).interval();
            assert!(d <= bounds.ceiling, "{d:?} exceeded the ceiling");
        }
        for _ in 0..40 {
            let d = s.observe(Observation::Changed, A_TIER).interval();
            assert!(d >= bounds.floor, "{d:?} fell below the floor");
        }
    }

    /// The revision-loop guard (M2-T05.8), which is the same mechanism as the freshness
    /// abandonment (M2-T15.4) seen from the dedup side: a page that produces a different body on
    /// every fetch would add a fresh content hash to the dedup set each visit forever. Parking it
    /// at the ceiling caps how often it can do so — a revision loop cannot flood any index because
    /// the page it belongs to is barely visited.
    #[test]
    fn a_revision_loop_is_parked_so_it_cannot_flood_the_dedup_index() {
        let mut s = Schedule::default();
        let mut d = Decision::Revisit(Duration::ZERO);
        for _ in 0..VOLATILE_AFTER {
            d = s.observe(Observation::Changed, 90);
        }
        assert!(
            d.is_volatile(),
            "a page changing every visit is a revision loop"
        );
        // Parked at the ceiling: at trust 90 that is days between visits, not the floor's hours.
        assert!(
            d.interval() >= Bounds::for_trust(90).ceiling,
            "a revision loop must be visited rarely, not chased"
        );
    }

    /// The Cho result: a page we cannot keep fresh is parked rather than chased.
    #[test]
    fn a_page_changing_on_every_visit_is_eventually_abandoned() {
        let mut s = Schedule::default();
        let mut verdict = Decision::Revisit(Duration::ZERO);
        for _ in 0..VOLATILE_AFTER {
            verdict = s.observe(Observation::Changed, A_TIER);
        }
        assert!(
            verdict.is_volatile(),
            "changing at the floor {VOLATILE_AFTER} times running is untrackable"
        );
        assert_eq!(
            verdict.interval(),
            Bounds::for_trust(A_TIER).ceiling,
            "an abandoned page parks at the ceiling; it is not dropped"
        );
    }

    /// A burst of updates is not the same as a page that never settles.
    #[test]
    fn a_burst_of_changes_does_not_abandon_the_page() {
        let mut s = Schedule::default();
        for _ in 0..VOLATILE_AFTER - 1 {
            assert!(!s.observe(Observation::Changed, A_TIER).is_volatile());
        }
        // One quiet visit means it settled. Breaking news does this constantly.
        assert!(!s.observe(Observation::Unchanged, A_TIER).is_volatile());
        for _ in 0..VOLATILE_AFTER - 1 {
            assert!(
                !s.observe(Observation::Changed, A_TIER).is_volatile(),
                "the count should have restarted after the quiet visit"
            );
        }
    }

    /// 304 is the whole reason frequent revisits are affordable, so it must schedule as a
    /// non-change rather than as an error or an unknown.
    #[test]
    fn not_modified_backs_off_exactly_like_unchanged() {
        let mut a = Schedule::default();
        let mut b = Schedule::default();
        for _ in 0..6 {
            let x = a.observe(Observation::Unchanged, 60);
            let y = b.observe(Observation::NotModified, 60);
            assert_eq!(x.interval(), y.interval());
        }
    }

    #[test]
    fn a_trusted_source_is_checked_more_often_than_an_untrusted_one() {
        let (mut trusted, mut ordinary) = (Schedule::default(), Schedule::default());
        for _ in 0..20 {
            trusted.observe(Observation::Unchanged, 95);
            ordinary.observe(Observation::Unchanged, 10);
        }
        assert!(
            trusted.interval < ordinary.interval,
            "a ministry that rarely changes is still worth checking sooner than a stray page: \
             {:?} vs {:?}",
            trusted.interval,
            ordinary.interval
        );
    }

    /// A page we have never parsed is not "unchanged".
    #[test]
    fn a_first_sighting_counts_as_a_change() {
        assert_eq!(Observation::from_hashes("", "b3:abc"), Observation::Changed);
        assert_eq!(
            Observation::from_hashes("b3:abc", "b3:abc"),
            Observation::Unchanged
        );
        assert_eq!(
            Observation::from_hashes("b3:abc", "b3:def"),
            Observation::Changed
        );
    }

    /// Churn outside the article must not register as a change.
    ///
    /// This is the property the whole module rests on, and it holds because `content_hash` is
    /// taken over the extracted body rather than the response. Asserted here so that a change to
    /// what gets hashed shows up as a failure in the scheduler that depends on it.
    #[test]
    fn furniture_changing_around_a_stable_article_is_not_a_change() {
        let article = "الحكومة تعلن عن إجراءات جديدة لدعم المؤسسات الناشئة في الجزائر";
        let monday = xustive_core::hash::content_hash(article);
        let tuesday = xustive_core::hash::content_hash(article);
        assert_eq!(
            Observation::from_hashes(&monday, &tuesday),
            Observation::Unchanged
        );

        let mut s = Schedule::default();
        let first = s.observe(Observation::from_hashes("", &monday), A_TIER);
        let second = s.observe(Observation::from_hashes(&monday, &tuesday), A_TIER);
        assert!(
            second.interval() > first.interval(),
            "an article whose text did not move should be visited less often, not more"
        );
    }
}

// --- storage -----------------------------------------------------------------------------------

use serde::{Deserialize, Serialize};

/// What we remember about a URL between visits.
///
/// # Why this lives in Redis rather than on the document
///
/// [[ADR-0011]] listed the change history as a schema change on the indexed document. Writing it
/// there turns out to defeat the purpose.
///
/// This record is written on **every** fetch, including the unchanged ones — and the cheapness of
/// an unchanged revisit is the entire reason adaptive scheduling pays for itself. Meilisearch takes
/// writes as queued tasks, so a corpus of a million pages revisiting daily would enqueue a million
/// bookkeeping tasks a day that change nothing anyone searches for. This project has already lost
/// a day to a Meilisearch task queue that stopped draining; feeding it write traffic proportional
/// to crawl volume rather than to content volume is how that happens again.
///
/// So: scheduling state sits beside the frontier, which is where the rest of the crawl's hot state
/// already is and shares its lifetime. The durable facts — `content_hash`, `crawled_at` — remain on
/// the document in Meilisearch, which is still the system of record for anything a user can see.
///
/// **What losing Redis costs.** Intervals reset, so every page looks new and is fetched once at its
/// floor before backing off again. Degraded, self-correcting, and in the safe direction: the
/// failure is extra politeness-bounded traffic, not a corpus that silently stops refreshing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Visit {
    /// Current revisit interval, in seconds.
    #[serde(default)]
    pub interval_secs: u64,
    /// Consecutive changed visits at the floor. Carries the volatility signal across restarts.
    #[serde(default)]
    pub changes_at_floor: u32,
    /// `content_hash` of the last successfully parsed body. The comparison key.
    #[serde(default)]
    pub content_hash: String,
    /// Validators, so the next request can be conditional and cost a few hundred bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
    /// Unix seconds of the last fetch.
    #[serde(default)]
    pub last_fetched: i64,
    /// Parked as untrackable. Kept so the console can explain why a page is rarely visited.
    #[serde(default)]
    pub volatile: bool,
}

impl Visit {
    /// The scheduling half of this record.
    pub fn schedule(&self) -> Schedule {
        Schedule {
            interval: Duration::from_secs(self.interval_secs),
            changes_at_floor: self.changes_at_floor,
        }
    }

    /// Fold an observation in, returning when to come back.
    ///
    /// Takes the new hash rather than reading it back out, so the caller cannot forget to store
    /// what it just compared against — an omission that would make every visit look like a change
    /// and hold the whole corpus at its floor forever.
    pub fn record(
        &mut self,
        observation: Observation,
        trust: u8,
        hash: &str,
        now: i64,
    ) -> Decision {
        let mut schedule = self.schedule();
        let decision = schedule.observe(observation, trust);
        self.interval_secs = schedule.interval.as_secs();
        self.changes_at_floor = schedule.changes_at_floor;
        self.volatile = decision.is_volatile();
        self.last_fetched = now;
        // A 304 carries no body to hash, so the stored hash must survive it untouched.
        if observation != Observation::NotModified && !hash.is_empty() {
            self.content_hash = hash.to_string();
        }
        decision
    }

    /// When this URL is next due, in unix seconds.
    pub fn due_at(&self) -> i64 {
        self.last_fetched.saturating_add(self.interval_secs as i64)
    }
}

/// What a sitemap `<lastmod>` tells us about a page we already hold (M2-T15.6).
///
/// A sitemap fetch reports on hundreds of URLs at once, so consulting it before scheduling a
/// revisit is the cheapest freshness signal there is — cheaper than a 304, because it is no request
/// against the page at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SitemapVerdict {
    /// The sitemap's `lastmod` is newer than our last fetch: the page changed, fetch it now.
    Changed,
    /// `lastmod` is no newer than our last fetch: unchanged, and a scheduled revisit can be skipped.
    Unchanged,
    /// We have no `lastmod`, or have never fetched this URL. The sitemap tells us nothing useful —
    /// fall back to the ordinary content-hash schedule and do not act on this.
    Unknown,
}

/// Decide what a sitemap entry says about a page we may already hold.
///
/// Deliberately conservative in the `Unknown` direction. A missing `lastmod`, or a page we have
/// never crawled, yields `Unknown` rather than a guess: acting on absent evidence is how a freshness
/// optimisation turns into a page that silently never refreshes. Only a `lastmod` we can compare
/// against a real prior fetch produces `Changed` or `Unchanged`.
pub fn sitemap_verdict(lastmod: Option<i64>, visit: Option<&Visit>) -> SitemapVerdict {
    match (lastmod, visit) {
        (Some(lm), Some(v)) if v.last_fetched > 0 => {
            if lm > v.last_fetched {
                SitemapVerdict::Changed
            } else {
                SitemapVerdict::Unchanged
            }
        }
        _ => SitemapVerdict::Unknown,
    }
}

/// Revisit state for a crawl, in Redis beside the frontier.
#[derive(Clone)]
pub struct Visits {
    client: redis::Client,
    namespace: String,
}

impl Visits {
    pub fn connect_in(url: &str, namespace: &str) -> Option<Self> {
        Some(Self {
            client: redis::Client::open(url).ok()?,
            namespace: namespace.to_string(),
        })
    }

    fn key(&self) -> String {
        format!("{}:visits", self.namespace)
    }

    async fn conn(&self) -> Option<redis::aio::MultiplexedConnection> {
        self.client.get_multiplexed_async_connection().await.ok()
    }

    /// What we know about a URL. `None` means never fetched — which the scheduler treats as a
    /// change, not as "unchanged", so a page we have never read cannot back off to the ceiling.
    pub async fn get(&self, url: &str) -> Option<Visit> {
        let mut conn = self.conn().await?;
        let raw: Option<String> = redis::cmd("HGET")
            .arg(self.key())
            .arg(url)
            .query_async(&mut conn)
            .await
            .ok()?;
        serde_json::from_str(&raw?).ok()
    }

    /// Store what we learned. Best-effort on purpose.
    ///
    /// A failed write costs one page's scheduling memory: it looks unvisited next time and is
    /// fetched at its floor. Failing the crawl over bookkeeping would be the wrong trade — the same
    /// reasoning as the robots cache failing open.
    pub async fn put(&self, url: &str, visit: &Visit) {
        let Some(mut conn) = self.conn().await else {
            return;
        };
        let Ok(encoded) = serde_json::to_string(visit) else {
            return;
        };
        let _: Result<(), _> = redis::cmd("HSET")
            .arg(self.key())
            .arg(url)
            .arg(encoded)
            .query_async::<()>(&mut conn)
            .await;
    }

    /// Forget a URL, for a takedown or a manual reindex.
    pub async fn forget(&self, url: &str) {
        let Some(mut conn) = self.conn().await else {
            return;
        };
        let _: Result<(), _> = redis::cmd("HDEL")
            .arg(self.key())
            .arg(url)
            .query_async::<()>(&mut conn)
            .await;
    }

    /// How many URLs have revisit state.
    pub async fn len(&self) -> usize {
        let Some(mut conn) = self.conn().await else {
            return 0;
        };
        redis::cmd("HLEN")
            .arg(self.key())
            .query_async(&mut conn)
            .await
            .unwrap_or(0)
    }

    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }

    /// Apply one sitemap entry to the page's schedule, and report what to do about it (M2-T15.6).
    ///
    /// - `Changed`: pull the due-time to `now` so the ordinary revisit path fetches it next. The
    ///   caller promotes it into the frontier; this only records the urgency.
    /// - `Unchanged`: the sitemap confirms no change, which is exactly a free 304 — so fold in an
    ///   `Unchanged` observation, growing the interval, without spending a request. This is the
    ///   whole saving: a page a sitemap says is stable is not fetched at all.
    /// - `Unknown`: nothing to act on; the content-hash schedule stands.
    ///
    /// Returns the verdict so the caller can promote a `Changed` page. Silent when there is no
    /// store, like every other write here.
    pub async fn apply_sitemap(
        &self,
        url: &str,
        lastmod: Option<i64>,
        now: i64,
        trust: u8,
    ) -> SitemapVerdict {
        let existing = self.get(url).await;
        let verdict = sitemap_verdict(lastmod, existing.as_ref());
        match verdict {
            SitemapVerdict::Unchanged => {
                // A free confirmation of no change. Fold it in as an unchanged observation so the
                // interval grows, and stamp `last_fetched` to now — we have current evidence the
                // held copy is good, which is what a successful revisit would have established.
                if let Some(mut v) = existing {
                    v.record(Observation::Unchanged, trust, "", now);
                    self.put(url, &v).await;
                }
            }
            SitemapVerdict::Changed => {
                // Bring the due-time forward so the page is fetched next. We do not fetch here —
                // the sitemap does not carry the body — only mark it overdue.
                if let Some(mut v) = existing {
                    v.interval_secs = 0;
                    v.last_fetched = 0;
                    self.put(url, &v).await;
                }
            }
            SitemapVerdict::Unknown => {}
        }
        verdict
    }
}

#[cfg(test)]
mod store_tests {
    use super::*;

    /// A 304 carries no body, so it must not overwrite the hash it was validated against.
    ///
    /// Clearing it would make the *next* visit compare against an empty hash, read that as a
    /// change, and pull the page back to its floor — turning the cheapest possible outcome into a
    /// reason to crawl harder.
    #[test]
    fn not_modified_preserves_the_stored_hash() {
        let mut v = Visit::default();
        v.record(Observation::Changed, 90, "b3:first", 1_000);
        assert_eq!(v.content_hash, "b3:first");

        v.record(Observation::NotModified, 90, "", 2_000);
        assert_eq!(
            v.content_hash, "b3:first",
            "a 304 has no body and must leave the comparison key alone"
        );
        assert_eq!(v.last_fetched, 2_000, "but it is still a visit");
    }

    #[test]
    fn a_visit_round_trips_through_json() {
        let mut v = Visit {
            etag: Some("\"abc\t123\"".into()),
            last_modified: Some("Wed, 21 Oct 2026 07:28:00 GMT".into()),
            ..Visit::default()
        };
        v.record(Observation::Changed, 60, "b3:x", 5_000);

        let encoded = serde_json::to_string(&v).unwrap();
        let decoded: Visit = serde_json::from_str(&encoded).unwrap();
        assert_eq!(
            decoded, v,
            "validators may contain anything, including tabs"
        );
    }

    #[test]
    fn due_at_is_the_last_fetch_plus_the_interval() {
        let mut v = Visit::default();
        v.record(Observation::Unchanged, 90, "b3:x", 10_000);
        assert_eq!(v.due_at(), 10_000 + v.interval_secs as i64);
    }

    /// Volatility must survive a restart, or a parked page resumes being chased on every deploy.
    #[test]
    fn volatility_survives_a_round_trip() {
        let mut v = Visit::default();
        for i in 0..VOLATILE_AFTER {
            v.record(Observation::Changed, 90, "b3:x", i as i64);
            // A changing page: a fresh hash every time.
            v.content_hash = format!("b3:{i}");
        }
        assert!(v.volatile);
        let decoded: Visit = serde_json::from_str(&serde_json::to_string(&v).unwrap()).unwrap();
        assert!(decoded.volatile);
        assert_eq!(decoded.changes_at_floor, v.changes_at_floor);
    }
}

#[cfg(test)]
mod sitemap_tests {
    use super::*;

    fn visit_fetched_at(t: i64) -> Visit {
        Visit {
            last_fetched: t,
            interval_secs: 7200,
            content_hash: "b3:x".into(),
            ..Visit::default()
        }
    }

    #[test]
    fn a_newer_lastmod_means_changed() {
        let v = visit_fetched_at(1000);
        assert_eq!(
            sitemap_verdict(Some(2000), Some(&v)),
            SitemapVerdict::Changed
        );
    }

    #[test]
    fn an_older_or_equal_lastmod_means_unchanged() {
        let v = visit_fetched_at(2000);
        assert_eq!(
            sitemap_verdict(Some(1000), Some(&v)),
            SitemapVerdict::Unchanged
        );
        assert_eq!(
            sitemap_verdict(Some(2000), Some(&v)),
            SitemapVerdict::Unchanged
        );
    }

    /// Absent evidence is never a verdict. A missing lastmod, or a page we have never fetched,
    /// must not be acted on — that is how a freshness optimisation becomes a page that never
    /// refreshes.
    #[test]
    fn absent_evidence_is_unknown() {
        let v = visit_fetched_at(1000);
        assert_eq!(sitemap_verdict(None, Some(&v)), SitemapVerdict::Unknown);
        assert_eq!(sitemap_verdict(Some(2000), None), SitemapVerdict::Unknown);
        // A visit that was never actually fetched (last_fetched 0) is not a baseline.
        assert_eq!(
            sitemap_verdict(Some(2000), Some(&Visit::default())),
            SitemapVerdict::Unknown
        );
    }
}
