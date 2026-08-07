//! Retrieval quality metrics.
//!
//! Ranking changes are easy to make and hard to judge. Every weight in [`crate::rank`] looks
//! defensible in isolation, and a change that improves the three queries someone happened to try
//! can quietly ruin the rest. These metrics exist so that "better" is a number rather than an
//! impression.
//!
//! # Choosing the metrics
//!
//! - **nDCG@10** is the headline. It is the only one here that uses graded relevance, so it can
//!   tell "the best result moved to position 3" from "a mediocre result moved to position 3" —
//!   which is exactly the distinction ranking work turns on.
//! - **MRR@10** answers a different question: how far down does a user have to read before
//!   finding anything useful. A change can improve nDCG while making MRR worse by promoting
//!   several decent results above one excellent one.
//! - **recall@50** separates retrieval failures from ranking failures. If recall is low, no
//!   amount of re-ranking helps, because the right document is not in the pool at all — and for
//!   an Arabic engine this is where normalisation and transliteration bugs show up.
//! - **zero-result rate** is the one users feel most sharply, and the one an average hides: a
//!   corpus can score well on nDCG while failing outright on a fifth of queries.
//!
//! # Judgements
//!
//! Grades are 0–3: not relevant, marginal, relevant, ideal. Anything not judged is treated as
//! **not relevant**, which is the standard assumption and is also wrong in a specific direction —
//! it penalises a system that surfaces a good document nobody thought to judge. That bias is
//! acceptable when comparing two systems against the same judgements and unacceptable as an
//! absolute score, so the reports here are only ever compared against each other.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// The best grade a judge can give.
pub const MAX_GRADE: u8 = 3;

/// One query and its judged documents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenQuery {
    pub id: String,
    pub query: String,
    /// Expected language, for reporting per-language breakdowns.
    pub lang: String,
    /// Document id to relevance grade, 0–3.
    #[serde(default)]
    pub judgements: HashMap<String, u8>,
    /// Free-text note on what the query is testing. Not used in scoring.
    #[serde(default)]
    pub note: String,
    /// How the judgements were produced. See [`Provenance`].
    #[serde(default)]
    pub judged_by: Provenance,
}

/// Where a judgement came from.
///
/// Recorded per query rather than per file so a set can be upgraded a query at a time as real
/// judges review it, and so a report can say what fraction of its score rests on machine labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Provenance {
    /// Generated automatically. Usable for regression detection, **not** as ground truth: it
    /// measures agreement with whatever heuristic produced it, not with an Algerian reader.
    #[default]
    Machine,
    /// Judged by a native speaker.
    Human,
}

impl GoldenQuery {
    pub fn grade(&self, doc_id: &str) -> u8 {
        self.judgements.get(doc_id).copied().unwrap_or(0)
    }

    /// Documents graded at least marginally relevant.
    pub fn relevant_count(&self) -> usize {
        self.judgements.values().filter(|g| **g > 0).count()
    }
}

/// Scores for a single query.
#[derive(Debug, Clone, Serialize)]
pub struct QueryScore {
    pub id: String,
    pub lang: String,
    pub ndcg_at_10: f64,
    pub mrr_at_10: f64,
    pub recall_at_50: f64,
    pub returned: usize,
    pub judged: usize,
}

/// Discounted cumulative gain at `k`.
///
/// Uses the `2^grade - 1` gain form rather than the linear one. With linear gain, an ideal result
/// is only three times a marginal one; with exponential, it is seven — which matches how users
/// actually value the difference between "answers my question" and "mentions the topic".
fn dcg(grades: &[u8], k: usize) -> f64 {
    grades
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, &g)| {
            let gain = (2f64.powi(g as i32)) - 1.0;
            // log2(i + 2) so the first position has divisor 1 rather than 0.
            gain / ((i + 2) as f64).log2()
        })
        .sum()
}

/// Normalised DCG at `k`.
///
/// Returns `None` when the query has no relevant documents at all. Scoring that as 0 would drag
/// the mean down for a query the system could not possibly have got right; scoring it as 1 would
/// reward systems for queries nobody judged. Excluding it is the only honest option, and the
/// count of excluded queries is reported alongside.
pub fn ndcg(golden: &GoldenQuery, results: &[String], k: usize) -> Option<f64> {
    if golden.relevant_count() == 0 {
        return None;
    }

    let actual: Vec<u8> = results.iter().map(|id| golden.grade(id)).collect();

    // The ideal ranking is every judged document sorted by grade, regardless of whether the
    // system returned it. Building it from the returned set instead would make a system that
    // retrieves nothing score 1.0.
    let mut ideal: Vec<u8> = golden.judgements.values().copied().collect();
    ideal.sort_unstable_by(|a, b| b.cmp(a));

    let ideal_dcg = dcg(&ideal, k);
    if ideal_dcg == 0.0 {
        return None;
    }
    Some((dcg(&actual, k) / ideal_dcg).clamp(0.0, 1.0))
}

/// Reciprocal rank of the first relevant result within `k`.
///
/// "Relevant" here means grade ≥ 2, not ≥ 1. A marginal result at position 1 is not a success —
/// counting it as one would let a system look good for burying the answer under near-misses.
pub fn mrr(golden: &GoldenQuery, results: &[String], k: usize) -> f64 {
    results
        .iter()
        .take(k)
        .position(|id| golden.grade(id) >= 2)
        .map(|i| 1.0 / (i + 1) as f64)
        .unwrap_or(0.0)
}

/// Fraction of relevant documents retrieved within `k`.
///
/// Returns `None` when nothing is judged relevant, for the same reason as [`ndcg`].
pub fn recall(golden: &GoldenQuery, results: &[String], k: usize) -> Option<f64> {
    let total = golden.relevant_count();
    if total == 0 {
        return None;
    }
    let found = results
        .iter()
        .take(k)
        .filter(|id| golden.grade(id) > 0)
        .count();
    Some(found as f64 / total as f64)
}

/// Aggregate scores over a run.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// Set when the caller stamps it; scoring itself has no clock.
    pub generated_at: String,
    pub queries: usize,
    /// Queries excluded from nDCG and recall because nothing was judged relevant.
    pub unjudged: usize,
    /// Queries whose judgements are machine-generated. A report where this is high is a
    /// regression detector, not a quality measurement.
    pub machine_judged: usize,
    pub ndcg_at_10: f64,
    pub mrr_at_10: f64,
    pub recall_at_50: f64,
    pub zero_result_rate: f64,
    /// Per-language nDCG@10. An Algeria-first engine that scores well overall while failing on
    /// Darija has not succeeded at the thing it exists for, and the mean hides that.
    pub by_language: HashMap<String, LanguageScore>,
    /// Age of returned documents, in days, bucketed. Not a quality score — a distribution to
    /// look at, because a ranking change that quietly favours old pages shows up here first.
    pub freshness: HashMap<String, usize>,
    pub per_query: Vec<QueryScore>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LanguageScore {
    pub queries: usize,
    pub ndcg_at_10: f64,
    pub zero_result_rate: f64,
}

/// One query's outcome, as observed from the search API.
pub struct Observed {
    pub golden: GoldenQuery,
    /// Result ids in rank order.
    pub results: Vec<String>,
    /// `published_at` for each result, for the freshness distribution.
    pub published: Vec<i64>,
}

/// Score a whole run.
pub fn score(observations: &[Observed], now: i64) -> Report {
    let mut per_query = Vec::with_capacity(observations.len());
    let mut ndcg_sum = 0.0;
    let mut ndcg_n = 0usize;
    let mut recall_sum = 0.0;
    let mut recall_n = 0usize;
    let mut mrr_sum = 0.0;
    let mut zero = 0usize;
    let mut unjudged = 0usize;
    let mut machine = 0usize;
    let mut freshness: HashMap<String, usize> = HashMap::new();
    let mut by_lang: HashMap<String, (usize, f64, usize, usize)> = HashMap::new();

    for obs in observations {
        let n = ndcg(&obs.golden, &obs.results, 10);
        let r = recall(&obs.golden, &obs.results, 50);
        let m = mrr(&obs.golden, &obs.results, 10);

        if n.is_none() {
            unjudged += 1;
        }
        if obs.golden.judged_by == Provenance::Machine {
            machine += 1;
        }
        if obs.results.is_empty() {
            zero += 1;
        }

        if let Some(v) = n {
            ndcg_sum += v;
            ndcg_n += 1;
        }
        if let Some(v) = r {
            recall_sum += v;
            recall_n += 1;
        }
        mrr_sum += m;

        for ts in &obs.published {
            *freshness.entry(age_bucket(*ts, now).into()).or_insert(0) += 1;
        }

        let entry = by_lang
            .entry(obs.golden.lang.clone())
            .or_insert((0, 0.0, 0, 0));
        entry.0 += 1;
        if let Some(v) = n {
            entry.1 += v;
            entry.2 += 1;
        }
        if obs.results.is_empty() {
            entry.3 += 1;
        }

        per_query.push(QueryScore {
            id: obs.golden.id.clone(),
            lang: obs.golden.lang.clone(),
            ndcg_at_10: n.unwrap_or(f64::NAN),
            mrr_at_10: m,
            recall_at_50: r.unwrap_or(f64::NAN),
            returned: obs.results.len(),
            judged: obs.golden.relevant_count(),
        });
    }

    let total = observations.len().max(1) as f64;
    Report {
        generated_at: String::new(),
        queries: observations.len(),
        unjudged,
        machine_judged: machine,
        ndcg_at_10: mean(ndcg_sum, ndcg_n),
        mrr_at_10: mrr_sum / total,
        recall_at_50: mean(recall_sum, recall_n),
        zero_result_rate: zero as f64 / total,
        by_language: by_lang
            .into_iter()
            .map(|(lang, (queries, sum, n, zeros))| {
                (
                    lang,
                    LanguageScore {
                        queries,
                        ndcg_at_10: mean(sum, n),
                        zero_result_rate: zeros as f64 / queries.max(1) as f64,
                    },
                )
            })
            .collect(),
        freshness,
        per_query,
    }
}

fn mean(sum: f64, n: usize) -> f64 {
    if n == 0 {
        0.0
    } else {
        sum / n as f64
    }
}

/// Bucket a document's age. Boundaries chosen to match the freshness decay in [`crate::rank`],
/// so a change to one is visible in the other.
fn age_bucket(published_at: i64, now: i64) -> &'static str {
    if published_at <= 0 {
        return "unknown";
    }
    match (now - published_at) / 86_400 {
        d if d < 0 => "future",
        0..=1 => "0-1d",
        2..=7 => "2-7d",
        8..=30 => "8-30d",
        31..=365 => "31-365d",
        _ => "365d+",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn golden(judgements: &[(&str, u8)]) -> GoldenQuery {
        GoldenQuery {
            id: "q1".into(),
            query: "test".into(),
            lang: "ar".into(),
            judgements: judgements
                .iter()
                .map(|(id, g)| ((*id).to_string(), *g))
                .collect(),
            note: String::new(),
            judged_by: Provenance::Machine,
        }
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn a_perfect_ranking_scores_one() {
        let g = golden(&[("a", 3), ("b", 2), ("c", 1)]);
        let score = ndcg(&g, &ids(&["a", "b", "c"]), 10).unwrap();
        assert!((score - 1.0).abs() < 1e-9, "got {score}");
    }

    #[test]
    fn a_reversed_ranking_scores_below_a_perfect_one() {
        let g = golden(&[("a", 3), ("b", 2), ("c", 1)]);
        let good = ndcg(&g, &ids(&["a", "b", "c"]), 10).unwrap();
        let bad = ndcg(&g, &ids(&["c", "b", "a"]), 10).unwrap();
        assert!(bad < good, "{bad} should be worse than {good}");
    }

    #[test]
    fn retrieving_nothing_scores_zero_not_one() {
        // The ideal ranking is built from the judgements, not from what was returned. Building
        // it from the returned set would make a system that retrieves nothing look perfect,
        // because its (empty) ranking is trivially optimal for its (empty) result set.
        let g = golden(&[("a", 3)]);
        assert_eq!(ndcg(&g, &[], 10), Some(0.0));
    }

    #[test]
    fn an_unjudged_query_is_excluded_rather_than_scored_zero() {
        // Scoring it 0 would punish the system for a query nobody judged; scoring it 1 would
        // reward it. Neither is a measurement, so it does not enter the mean at all.
        let g = golden(&[]);
        assert_eq!(ndcg(&g, &ids(&["a"]), 10), None);
        assert_eq!(recall(&g, &ids(&["a"]), 50), None);
    }

    #[test]
    fn grade_differences_matter_more_than_linearly() {
        // Exponential gain: an ideal result is worth seven marginal ones, not three. That
        // matches how a user values "answers my question" over "mentions the topic".
        let g = golden(&[("ideal", 3), ("marginal", 1)]);
        let ideal_first = ndcg(&g, &ids(&["ideal", "marginal"]), 10).unwrap();
        let marginal_first = ndcg(&g, &ids(&["marginal", "ideal"]), 10).unwrap();
        assert!(
            ideal_first - marginal_first > 0.1,
            "the ordering barely mattered: {ideal_first} vs {marginal_first}"
        );
    }

    #[test]
    fn position_one_is_worth_more_than_position_two() {
        let g = golden(&[("a", 3), ("b", 3)]);
        let first = ndcg(&golden(&[("a", 3)]), &ids(&["a", "x"]), 10).unwrap();
        let second = ndcg(&golden(&[("a", 3)]), &ids(&["x", "a"]), 10).unwrap();
        assert!(first > second);
        // Two ideal results in either order is still perfect.
        assert!((ndcg(&g, &ids(&["a", "b"]), 10).unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn mrr_ignores_merely_marginal_results() {
        // A near-miss at position 1 is not a success. Counting it as one lets a system look good
        // for burying the answer under things that merely mention the topic.
        let g = golden(&[("marginal", 1), ("real", 2)]);
        assert_eq!(mrr(&g, &ids(&["marginal", "real"]), 10), 0.5);
        assert_eq!(mrr(&g, &ids(&["real", "marginal"]), 10), 1.0);
    }

    #[test]
    fn mrr_is_zero_when_nothing_relevant_is_found() {
        let g = golden(&[("a", 3)]);
        assert_eq!(mrr(&g, &ids(&["x", "y"]), 10), 0.0);
    }

    #[test]
    fn recall_counts_marginal_documents() {
        // Deliberately different from MRR. Recall asks whether retrieval found the material at
        // all, and a marginal document being missing is still a retrieval failure — it is where
        // Arabic normalisation bugs surface.
        let g = golden(&[("a", 1), ("b", 3)]);
        assert_eq!(recall(&g, &ids(&["a"]), 50), Some(0.5));
        assert_eq!(recall(&g, &ids(&["a", "b"]), 50), Some(1.0));
    }

    #[test]
    fn cutoffs_are_respected() {
        let g = golden(&[("a", 3)]);
        let mut results = ids(&["x"; 20]);
        results.push("a".into());
        assert_eq!(mrr(&g, &results, 10), 0.0, "position 21 is beyond @10");
        assert_eq!(recall(&g, &results, 50), Some(1.0), "but within @50");
    }

    #[test]
    fn a_report_separates_machine_judgements_from_human_ones() {
        // A report built on machine labels measures agreement with a heuristic, not with an
        // Algerian reader. It has to say so on its face, or it will be quoted as if it did.
        let obs = vec![Observed {
            golden: golden(&[("a", 3)]),
            results: ids(&["a"]),
            published: vec![0],
        }];
        let report = score(&obs, 1_700_000_000);
        assert_eq!(report.machine_judged, 1);
        assert_eq!(report.queries, 1);
    }

    #[test]
    fn the_zero_result_rate_is_not_hidden_by_the_mean() {
        // Half these queries return nothing, and nDCG over the surviving half looks excellent.
        // This is the metric that keeps that from reading as success.
        let obs = vec![
            Observed {
                golden: golden(&[("a", 3)]),
                results: ids(&["a"]),
                published: vec![],
            },
            Observed {
                golden: golden(&[("b", 3)]),
                results: vec![],
                published: vec![],
            },
        ];
        let report = score(&obs, 1_700_000_000);
        assert_eq!(report.zero_result_rate, 0.5);
        assert!(report.ndcg_at_10 > 0.4, "got {}", report.ndcg_at_10);
    }

    #[test]
    fn per_language_scores_are_reported_separately() {
        // An Algeria-first engine that scores well overall while failing on Darija has not done
        // the thing it exists to do, and an average will not show it.
        let mut ary = golden(&[("a", 3)]);
        ary.lang = "ary".into();
        let obs = vec![
            Observed {
                golden: golden(&[("a", 3)]),
                results: ids(&["a"]),
                published: vec![],
            },
            Observed {
                golden: ary,
                results: vec![],
                published: vec![],
            },
        ];
        let report = score(&obs, 1_700_000_000);
        assert_eq!(report.by_language["ary"].zero_result_rate, 1.0);
        assert_eq!(report.by_language["ar"].zero_result_rate, 0.0);
    }

    #[test]
    fn freshness_buckets_line_up_with_the_ranking_decay() {
        let now = 1_700_000_000;
        assert_eq!(age_bucket(now, now), "0-1d");
        assert_eq!(age_bucket(now - 3 * 86_400, now), "2-7d");
        assert_eq!(age_bucket(now - 60 * 86_400, now), "31-365d");
        assert_eq!(age_bucket(0, now), "unknown");
        assert_eq!(age_bucket(now + 86_400, now), "future");
    }
}
