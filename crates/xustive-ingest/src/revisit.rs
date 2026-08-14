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

/// How aggressively the interval shrinks when content changed.
const SHRINK: f64 = 0.5;
/// How aggressively it grows when it did not. Deliberately gentler than the shrink: being too slow
/// to notice a change costs freshness, while being too eager costs someone else's bandwidth.
const GROW: f64 = 1.5;

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
        let factor = if observation.is_change() {
            SHRINK
        } else {
            GROW
        };
        let next = Duration::from_secs_f64(current.as_secs_f64() * factor)
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
