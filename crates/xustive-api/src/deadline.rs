//! Request deadlines.
//!
//! # Why an absolute instant, not a duration
//!
//! Passing "you have 1500 ms" down a call chain gives every stage the *full* budget. Four stages
//! each politely allowing 1500 ms produce a request that takes six seconds while every one of
//! them believes it was within budget.
//!
//! An absolute `Instant` cannot be misread. Each stage asks how long is left, and the answer
//! accounts for everything already spent — including the time the request queued before any of
//! this ran.
//!
//! # The degradation ladder
//!
//! When the budget runs short, stages are dropped in a fixed order rather than the request
//! failing. The order encodes what the product is: results are the product; everything else is an
//! improvement on them.
//!
//! ```text
//!   summary  →  expansion leg  →  facets  →  re-ranking  →  retrieval
//!   dropped first                                          never dropped
//! ```
//!
//! Retrieval is never dropped, because a search that returns nothing is not a degraded search —
//! it is a broken one, and the user cannot tell the difference from an outage.

use std::time::{Duration, Instant};

/// An absolute point by which a request must be finished.
#[derive(Debug, Clone, Copy)]
pub struct Deadline {
    at: Instant,
    total: Duration,
}

/// Stages, in the order they are given up.
///
/// Ordered by what a user loses. Dropping the summary costs a convenience; dropping re-ranking
/// costs result quality; dropping retrieval costs the product.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// The AI summary. Already a separate request, so this is nearly free to abandon.
    Summary,
    /// The second, expanded retrieval leg. Costs Arabizi queries their results, so it is given up
    /// only after the summary.
    Expansion,
    /// Facet counts. The filter chips disappear; the results do not.
    Facets,
    /// Re-ranking. Results are returned in the engine's own order, which is worse but is still
    /// results.
    Rerank,
    /// Retrieval. Never dropped.
    Retrieval,
}

impl Stage {
    /// The share of the total budget below which this stage is skipped.
    ///
    /// Fractions rather than fixed milliseconds, so the ladder still holds when the budget is
    /// changed in configuration — a hardcoded 200 ms floor is wrong the moment someone sets a
    /// 300 ms budget.
    fn floor(self) -> f32 {
        match self {
            Self::Summary => 0.55,
            Self::Expansion => 0.35,
            Self::Facets => 0.20,
            Self::Rerank => 0.08,
            // Always attempted, whatever is left.
            Self::Retrieval => 0.0,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Expansion => "expansion",
            Self::Facets => "facets",
            Self::Rerank => "rerank",
            Self::Retrieval => "retrieval",
        }
    }
}

impl Deadline {
    /// Start a budget now.
    pub fn new(budget: Duration) -> Self {
        Self {
            at: Instant::now() + budget,
            total: budget,
        }
    }

    /// Time left, saturating at zero. Never negative — a stage asking "how long do I have" after
    /// the deadline gets nothing, not a wrapped duration.
    pub fn remaining(&self) -> Duration {
        self.at.saturating_duration_since(Instant::now())
    }

    pub fn expired(&self) -> bool {
        self.remaining().is_zero()
    }

    /// Whether there is enough budget left to attempt `stage`.
    ///
    /// Asked *before* starting, not after failing. A stage that begins with 20 ms left and blocks
    /// for 200 has already broken the budget by the time anyone notices.
    pub fn allows(&self, stage: Stage) -> bool {
        if stage == Stage::Retrieval {
            return true;
        }
        let left = self.remaining().as_secs_f32();
        let total = self.total.as_secs_f32().max(f32::EPSILON);
        left / total >= stage.floor()
    }

    /// A per-call timeout: whatever is left, capped by the caller's own limit.
    ///
    /// The cap matters. Giving a single backend call the entire remaining budget means one slow
    /// dependency consumes everything and the stages after it are skipped for lack of time
    /// rather than because they were not worth doing.
    pub fn budget_for(&self, cap: Duration) -> Duration {
        self.remaining().min(cap)
    }

    pub fn total(&self) -> Duration {
        self.total
    }

    /// How much has been spent, for reporting.
    pub fn elapsed(&self) -> Duration {
        self.total.saturating_sub(self.remaining())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_deadline_allows_everything() {
        let d = Deadline::new(Duration::from_millis(1500));
        for stage in [
            Stage::Summary,
            Stage::Expansion,
            Stage::Facets,
            Stage::Rerank,
            Stage::Retrieval,
        ] {
            assert!(
                d.allows(stage),
                "{stage:?} should be allowed on a fresh budget"
            );
        }
    }

    #[test]
    fn stages_are_dropped_in_order_as_the_budget_shrinks() {
        // Constructed rather than slept: a test that waits out a real budget is slow and flaky.
        let ladder = [
            (
                0.60,
                vec![
                    Stage::Summary,
                    Stage::Expansion,
                    Stage::Facets,
                    Stage::Rerank,
                ],
            ),
            (0.40, vec![Stage::Expansion, Stage::Facets, Stage::Rerank]),
            (0.25, vec![Stage::Facets, Stage::Rerank]),
            (0.10, vec![Stage::Rerank]),
            (0.01, vec![]),
        ];

        for (fraction, expected) in ladder {
            let total = Duration::from_millis(1000);
            let d = Deadline {
                at: Instant::now() + Duration::from_millis((1000.0 * fraction) as u64),
                total,
            };
            let allowed: Vec<Stage> = [
                Stage::Summary,
                Stage::Expansion,
                Stage::Facets,
                Stage::Rerank,
            ]
            .into_iter()
            .filter(|s| d.allows(*s))
            .collect();
            assert_eq!(allowed, expected, "at {fraction} of the budget");
        }
    }

    #[test]
    fn retrieval_is_never_dropped() {
        // A search that returns nothing is not a degraded search, it is a broken one — and the
        // user cannot tell the difference from an outage.
        let expired = Deadline {
            at: Instant::now() - Duration::from_secs(10),
            total: Duration::from_millis(1500),
        };
        assert!(expired.expired());
        assert!(expired.allows(Stage::Retrieval));
        assert!(!expired.allows(Stage::Summary));
    }

    #[test]
    fn remaining_never_goes_negative() {
        let past = Deadline {
            at: Instant::now() - Duration::from_secs(5),
            total: Duration::from_millis(100),
        };
        assert_eq!(past.remaining(), Duration::ZERO);
    }

    #[test]
    fn a_per_call_budget_is_capped() {
        // Without the cap, one slow dependency eats the whole budget and later stages are skipped
        // for lack of time rather than on merit.
        let d = Deadline::new(Duration::from_millis(1500));
        assert!(d.budget_for(Duration::from_millis(200)) <= Duration::from_millis(200));
    }

    #[test]
    fn a_per_call_budget_never_exceeds_what_is_left() {
        let nearly_done = Deadline {
            at: Instant::now() + Duration::from_millis(30),
            total: Duration::from_millis(1500),
        };
        assert!(nearly_done.budget_for(Duration::from_secs(10)) <= Duration::from_millis(31));
    }

    #[test]
    fn the_ladder_is_ordered_by_what_the_user_loses() {
        // The floors must decrease down the ladder, or a stage would be dropped before something
        // more valuable than it.
        let ordered = [
            Stage::Summary,
            Stage::Expansion,
            Stage::Facets,
            Stage::Rerank,
        ];
        for pair in ordered.windows(2) {
            assert!(
                pair[0].floor() > pair[1].floor(),
                "{:?} must be given up before {:?}",
                pair[0],
                pair[1]
            );
        }
    }
}
