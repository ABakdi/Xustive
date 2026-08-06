//! Stage-2 re-ranking.
//!
//! Meilisearch returns candidates ordered by textual relevance. This reorders them using signals
//! the engine does not have: how fresh the document is *relative to what the query is asking
//! for*, how accountable its source is, and whether the page has anything to say.
//!
//! It runs in-process rather than as index settings because these weights change often, and
//! reindexing to tune a weight would make tuning impractical.
//!
//! # The rule that governs everything here
//!
//! Textual relevance dominates. Every other signal is a tie-breaker among documents that already
//! match. A freshness or quality boost that can lift an irrelevant document above a relevant one
//! is a bug, not a feature, and the weights are bounded to prevent it.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use xustive_core::{DatePrecision, TrustTier};

/// What the query seems to be asking for, which sets how fast relevance decays with age.
///
/// Someone asking about today's news wants today's news; someone asking how to renew a passport
/// wants the correct answer regardless of when it was written. Applying one decay curve to both
/// makes one of them wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// Breaking or dated. Half-life measured in days.
    News,
    /// Procedures, definitions, how-to. Age barely matters.
    Evergreen,
    /// No strong signal either way.
    Default,
}

impl Intent {
    /// Freshness time constant, in days.
    pub fn tau_days(self) -> f32 {
        match self {
            Self::News => 3.0,
            Self::Evergreen => 90.0,
            Self::Default => 30.0,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::News => "news",
            Self::Evergreen => "evergreen",
            Self::Default => "default",
        }
    }
}

/// Terms that mark a query as time-sensitive.
const NEWS_MARKERS: &[&str] = &[
    "اليوم",
    "الان",
    "الآن",
    "عاجل",
    "أمس",
    "امس",
    "مباشر",
    "جديد",
    "آخر",
    "اخر",
    "لحظة",
    "aujourd",
    "maintenant",
    "urgent",
    "hier",
    "direct",
    "dernier",
    "derniere",
    "actualite",
    "today",
    "now",
    "breaking",
    "latest",
    "live",
];

/// Terms that mark a query as looking for a stable answer.
const EVERGREEN_MARKERS: &[&str] = &[
    "كيفاش",
    "كيف",
    "طريقة",
    "شروط",
    "وثائق",
    "كيفية",
    "دليل",
    "شرح",
    "ما هو",
    "تعريف",
    "kifach",
    "comment",
    "procedure",
    "conditions",
    "documents",
    "guide",
    "demarche",
    "how",
    "what is",
    "guide",
    "steps",
    "requirements",
];

/// Positions over which engine relevance decays by `1/e`. See the comment at its use site.
const RELEVANCE_DECAY: f32 = 10.0;

/// Weights. Hot-reloadable from `config/ranking.toml` without a restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Weights {
    /// Textual relevance. Deliberately the largest by a wide margin.
    pub relevance: f32,
    pub freshness: f32,
    pub trust: f32,
    pub quality: f32,
    /// Subtracted, not added.
    pub spam_penalty: f32,
    /// Multiplier applied to freshness when the publication date was guessed.
    pub unknown_date_factor: f32,
    /// Maximum results from one domain in the first page.
    pub per_domain_cap: usize,
    /// Hamming distance at or below which two documents are treated as the same story.
    pub simhash_collapse_distance: u32,
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            // The side weights sum to 0.32, deliberately below the relevance gap across
            // twenty positions (0.48). That bound is what makes "relevance dominates" true by
            // construction rather than by hoping the numbers work out.
            relevance: 0.55,
            freshness: 0.15,
            trust: 0.10,
            quality: 0.07,
            spam_penalty: 0.15,
            unknown_date_factor: 0.5,
            per_domain_cap: 3,
            simhash_collapse_distance: 3,
        }
    }
}

/// A per-signal breakdown, so `--explain` can answer "why is this result third?".
#[derive(Debug, Clone, Default, Serialize)]
pub struct Explain {
    pub relevance: f32,
    pub freshness: f32,
    pub trust: f32,
    pub quality: f32,
    pub spam: f32,
    pub total: f32,
    pub age_days: f32,
    pub date_trusted: bool,
    pub collapsed: usize,
}

#[derive(Debug, Clone)]
pub struct Ranked {
    pub hit: Value,
    pub score: f32,
    pub explain: Explain,
    /// Near-duplicates folded into this result.
    pub collapsed: Vec<Value>,
}

/// Infer intent from the normalised query plus what came back.
///
/// The candidate ages are used as a fallback signal: if most of what matched is from the last
/// week, the query is almost certainly about something current even without a temporal word.
pub fn infer_intent(normalized_query: &str, candidate_ages_days: &[f32]) -> Intent {
    let q = normalized_query;
    if NEWS_MARKERS.iter().any(|m| q.contains(m)) {
        return Intent::News;
    }
    if EVERGREEN_MARKERS.iter().any(|m| q.contains(m)) {
        return Intent::Evergreen;
    }
    if !candidate_ages_days.is_empty() {
        let recent = candidate_ages_days.iter().filter(|a| **a < 7.0).count();
        if recent as f32 / candidate_ages_days.len() as f32 >= 0.4 {
            return Intent::News;
        }
    }
    Intent::Default
}

/// Re-rank candidates.
///
/// `now` is passed in rather than read from the clock so ranking is deterministic and testable.
pub fn rerank(
    hits: &[Value],
    normalized_query: &str,
    now: i64,
    trust_of: &HashMap<String, TrustTier>,
    weights: &Weights,
) -> Vec<Ranked> {
    if hits.is_empty() {
        return Vec::new();
    }

    let ages: Vec<f32> = hits.iter().map(|h| age_days(h, now)).collect();
    let intent = infer_intent(normalized_query, &ages);
    let tau = intent.tau_days();

    let mut scored: Vec<Ranked> = hits
        .iter()
        .enumerate()
        .map(|(pos, hit)| {
            // Engine rank, normalised.
            //
            // The shape of this curve is the whole tuning question. Adjacent candidates are
            // near-equally relevant and the engine's ordering between them is close to
            // arbitrary, so side signals *should* be able to reorder them. Candidates fifty
            // apart genuinely differ, and side signals should *not* be able to bridge that.
            //
            // A logarithmic curve gets this exactly backwards: `1/log2(pos+2)` drops 0.37
            // between positions 0 and 1, which is more than every other signal combined can
            // produce, so freshness and trust become decorative. An exponential with a decay
            // constant of ten gives a 0.05 gap between neighbours and a 0.48 gap across twenty
            // positions — small enough locally for freshness to matter, large enough globally
            // that nothing climbs the list on side signals alone.
            let relevance = (-(pos as f32) / RELEVANCE_DECAY).exp();

            let age = ages[pos];
            let date_trusted = precision_of(hit) != DatePrecision::Unknown;
            let mut freshness = (-age / tau).exp();
            if !date_trusted {
                // We refuse to reward a date we guessed.
                freshness *= weights.unknown_date_factor;
            }

            let trust = source_of(hit)
                .and_then(|s| trust_of.get(&s))
                .copied()
                .unwrap_or(TrustTier::B)
                .weight();

            let quality = f32_field(hit, "quality_score").unwrap_or(0.4);
            let spam = f32_field(hit, "spam_score").unwrap_or(0.0);

            let total = weights.relevance * relevance
                + weights.freshness * freshness
                + weights.trust * trust
                + weights.quality * quality
                - weights.spam_penalty * spam;

            Ranked {
                explain: Explain {
                    relevance: weights.relevance * relevance,
                    freshness: weights.freshness * freshness,
                    trust: weights.trust * trust,
                    quality: weights.quality * quality,
                    spam: -weights.spam_penalty * spam,
                    total,
                    age_days: age,
                    date_trusted,
                    collapsed: 0,
                },
                hit: hit.clone(),
                score: total,
                collapsed: Vec::new(),
            }
        })
        .collect();

    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let scored = collapse_near_duplicates(scored, weights.simhash_collapse_distance);
    cap_per_domain(scored, weights.per_domain_cap)
}

/// Fold near-identical documents into the best-scoring copy.
///
/// The same press release republished by six sites should occupy one slot, not six. The survivor
/// is the highest-scoring copy, which given the trust weight is usually the most accountable
/// publisher.
fn collapse_near_duplicates(ranked: Vec<Ranked>, max_distance: u32) -> Vec<Ranked> {
    let mut out: Vec<Ranked> = Vec::with_capacity(ranked.len());

    'outer: for item in ranked {
        let Some(sig) = simhash_of(&item.hit) else {
            out.push(item);
            continue;
        };
        for kept in out.iter_mut() {
            if let Some(other) = simhash_of(&kept.hit) {
                if (sig ^ other).count_ones() <= max_distance {
                    kept.collapsed.push(item.hit);
                    kept.explain.collapsed = kept.collapsed.len();
                    continue 'outer;
                }
            }
        }
        out.push(item);
    }
    out
}

/// Stop one site owning the page.
///
/// Results beyond the cap are pushed down rather than dropped: they are still relevant, they
/// just should not crowd out everything else.
fn cap_per_domain(ranked: Vec<Ranked>, cap: usize) -> Vec<Ranked> {
    if cap == 0 {
        return ranked;
    }
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut kept: Vec<Ranked> = Vec::new();
    let mut deferred: Vec<Ranked> = Vec::new();

    for item in ranked {
        let domain = string_field(&item.hit, "domain").unwrap_or_default();
        let n = counts.entry(domain).or_insert(0);
        if *n < cap {
            *n += 1;
            kept.push(item);
        } else {
            deferred.push(item);
        }
    }
    kept.extend(deferred);
    kept
}

// --- field access ------------------------------------------------------------------------

fn f32_field(v: &Value, key: &str) -> Option<f32> {
    v.get(key).and_then(Value::as_f64).map(|f| f as f32)
}

fn string_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn source_of(v: &Value) -> Option<String> {
    string_field(v, "source_id")
}

fn simhash_of(v: &Value) -> Option<u64> {
    let s = v.get("simhash")?.as_str()?;
    u64::from_str_radix(s.trim_start_matches("0x"), 16).ok()
}

fn precision_of(v: &Value) -> DatePrecision {
    match v.get("published_at_precision").and_then(Value::as_str) {
        Some("second") => DatePrecision::Second,
        Some("day") => DatePrecision::Day,
        Some("month") => DatePrecision::Month,
        _ => DatePrecision::Unknown,
    }
}

fn age_days(v: &Value, now: i64) -> f32 {
    let published = v.get("published_at").and_then(Value::as_i64).unwrap_or(now);
    ((now - published).max(0) as f32) / 86_400.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_786_017_600;

    fn doc(id: &str, age_days: i64, quality: f32, source: &str) -> Value {
        json!({
            "id": id,
            "domain": format!("{source}.dz"),
            "source_id": source,
            "published_at": NOW - age_days * 86_400,
            "published_at_precision": "day",
            "quality_score": quality,
            "spam_score": 0.0,
        })
    }

    fn trust() -> HashMap<String, TrustTier> {
        let mut m = HashMap::new();
        m.insert("a-source".to_string(), TrustTier::A);
        m.insert("c-source".to_string(), TrustTier::C);
        m
    }

    #[test]
    fn relevance_dominates_every_other_signal() {
        // The invariant that keeps ranking honest: a fresh, high-quality, trusted document
        // cannot leapfrog many positions on those signals alone.
        //
        // Each filler gets its own domain so the per-domain cap stays out of the way. Capping
        // is a separate mechanism with its own test, and letting it fire here would make this
        // test pass for the wrong reason.
        let hits: Vec<Value> = (0..20)
            .map(|i| {
                let mut d = doc(&format!("d{i}"), 400, 0.1, "c-source");
                d["domain"] = json!(format!("filler{i}.dz"));
                d
            })
            .chain(std::iter::once(doc("boosted", 0, 1.0, "a-source")))
            .collect();

        let out = rerank(&hits, "الجزائر", NOW, &trust(), &Weights::default());
        let pos = out.iter().position(|r| r.hit["id"] == "boosted").unwrap();
        assert!(
            pos > 3,
            "a last-place-by-relevance document reached position {pos} on side signals alone"
        );
    }

    #[test]
    fn freshness_breaks_ties_among_equally_relevant_documents() {
        let hits = vec![
            doc("old", 365, 0.5, "a-source"),
            doc("new", 1, 0.5, "a-source"),
        ];
        let out = rerank(&hits, "الجزائر", NOW, &trust(), &Weights::default());
        // `old` is first by engine rank; a year of age should not be enough to keep it there.
        assert_eq!(
            out[0].hit["id"], "new",
            "fresh document should win a near-tie"
        );
    }

    #[test]
    fn news_intent_decays_much_faster_than_evergreen() {
        assert_eq!(infer_intent("اخر اخبار اليوم", &[]), Intent::News);
        assert_eq!(
            infer_intent("كيفاش ندير جواز السفر", &[]),
            Intent::Evergreen
        );
        assert_eq!(
            infer_intent("comment obtenir un passeport", &[]),
            Intent::Evergreen
        );
        assert_eq!(infer_intent("سونلغاز", &[]), Intent::Default);
        assert!(Intent::News.tau_days() < Intent::Default.tau_days());
        assert!(Intent::Default.tau_days() < Intent::Evergreen.tau_days());
    }

    #[test]
    fn intent_falls_back_to_what_actually_matched() {
        // No temporal word, but almost everything returned is from this week.
        let fresh_ages = vec![0.5, 1.0, 2.0, 3.0, 30.0];
        assert_eq!(infer_intent("سونلغاز", &fresh_ages), Intent::News);

        let old_ages = vec![100.0, 200.0, 300.0, 400.0, 500.0];
        assert_eq!(infer_intent("سونلغاز", &old_ages), Intent::Default);
    }

    #[test]
    fn a_guessed_date_earns_only_half_the_freshness() {
        let mut guessed = doc("guessed", 1, 0.5, "a-source");
        guessed["published_at_precision"] = json!("unknown");
        let known = doc("known", 1, 0.5, "a-source");

        let out = rerank(
            &[guessed, known],
            "الجزائر",
            NOW,
            &trust(),
            &Weights::default(),
        );
        let g = out.iter().find(|r| r.hit["id"] == "guessed").unwrap();
        let k = out.iter().find(|r| r.hit["id"] == "known").unwrap();
        assert!(
            g.explain.freshness < k.explain.freshness,
            "a guessed date must not earn full freshness credit"
        );
        assert!(!g.explain.date_trusted);
    }

    #[test]
    fn trust_tier_contributes() {
        let out = rerank(
            &[doc("c", 10, 0.5, "c-source"), doc("a", 10, 0.5, "a-source")],
            "الجزائر",
            NOW,
            &trust(),
            &Weights::default(),
        );
        let a = out.iter().find(|r| r.hit["id"] == "a").unwrap();
        let c = out.iter().find(|r| r.hit["id"] == "c").unwrap();
        assert!(a.explain.trust > c.explain.trust);
    }

    #[test]
    fn spam_is_subtracted() {
        let mut spammy = doc("spam", 1, 0.5, "a-source");
        spammy["spam_score"] = json!(0.9);
        let out = rerank(
            &[spammy, doc("clean", 1, 0.5, "a-source")],
            "x",
            NOW,
            &trust(),
            &Weights::default(),
        );
        assert_eq!(out[0].hit["id"], "clean");
        let s = out.iter().find(|r| r.hit["id"] == "spam").unwrap();
        assert!(
            s.explain.spam < 0.0,
            "spam should be a penalty, not a bonus"
        );
    }

    #[test]
    fn near_duplicates_collapse_into_one_result() {
        // The same press release republished. Six copies should occupy one slot.
        let mut a = doc("a", 1, 0.5, "a-source");
        let mut b = doc("b", 1, 0.5, "c-source");
        a["simhash"] = json!("ffffffffffffffff");
        b["simhash"] = json!("fffffffffffffffe"); // distance 1

        let out = rerank(&[a, b], "x", NOW, &trust(), &Weights::default());
        assert_eq!(out.len(), 1, "near-duplicates should collapse");
        assert_eq!(out[0].collapsed.len(), 1);
        assert_eq!(out[0].explain.collapsed, 1);
    }

    #[test]
    fn genuinely_different_documents_do_not_collapse() {
        let mut a = doc("a", 1, 0.5, "a-source");
        let mut b = doc("b", 1, 0.5, "c-source");
        a["simhash"] = json!("ffffffffffffffff");
        b["simhash"] = json!("0000000000000000"); // distance 64

        let out = rerank(&[a, b], "x", NOW, &trust(), &Weights::default());
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn the_more_accountable_copy_survives_collapse() {
        let mut low = doc("low", 1, 0.5, "c-source");
        let mut high = doc("high", 1, 0.5, "a-source");
        low["simhash"] = json!("ffffffffffffffff");
        high["simhash"] = json!("ffffffffffffffff");

        let out = rerank(&[low, high], "x", NOW, &trust(), &Weights::default());
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].hit["id"], "high",
            "the higher-trust copy should survive"
        );
    }

    #[test]
    fn one_domain_cannot_own_the_page() {
        let mut hits: Vec<Value> = (0..10)
            .map(|i| doc(&format!("h{i}"), 1, 0.9, "a-source"))
            .collect();
        hits.push(doc("other", 1, 0.5, "c-source"));

        let out = rerank(&hits, "x", NOW, &trust(), &Weights::default());
        let top4: Vec<&str> = out
            .iter()
            .take(4)
            .map(|r| r.hit["domain"].as_str().unwrap())
            .collect();
        let same = top4.iter().filter(|d| **d == "a-source.dz").count();
        assert!(same <= 3, "one domain took {same} of the top 4");
    }

    #[test]
    fn capped_results_are_deferred_not_dropped() {
        let hits: Vec<Value> = (0..10)
            .map(|i| doc(&format!("h{i}"), 1, 0.9, "a-source"))
            .collect();
        let out = rerank(&hits, "x", NOW, &trust(), &Weights::default());
        assert_eq!(out.len(), 10, "capping must reorder, never discard");
    }

    #[test]
    fn explain_components_sum_to_the_total() {
        let out = rerank(
            &[doc("a", 5, 0.6, "a-source")],
            "x",
            NOW,
            &trust(),
            &Weights::default(),
        );
        let e = &out[0].explain;
        let sum = e.relevance + e.freshness + e.trust + e.quality + e.spam;
        assert!((sum - e.total).abs() < 1e-5, "{sum} != {}", e.total);
    }

    #[test]
    fn empty_input_is_handled() {
        assert!(rerank(&[], "x", NOW, &trust(), &Weights::default()).is_empty());
    }

    #[test]
    fn missing_fields_do_not_panic() {
        let out = rerank(
            &[json!({"id": "bare"})],
            "x",
            NOW,
            &trust(),
            &Weights::default(),
        );
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn ranking_is_deterministic() {
        let hits: Vec<Value> = (0..8)
            .map(|i| doc(&format!("h{i}"), i, 0.5, "a-source"))
            .collect();
        let a = rerank(&hits, "x", NOW, &trust(), &Weights::default());
        let b = rerank(&hits, "x", NOW, &trust(), &Weights::default());
        let ids_a: Vec<&Value> = a.iter().map(|r| &r.hit["id"]).collect();
        let ids_b: Vec<&Value> = b.iter().map(|r| &r.hit["id"]).collect();
        assert_eq!(ids_a, ids_b);
    }

    #[test]
    fn default_weights_keep_relevance_dominant() {
        let w = Weights::default();
        let side_total = w.freshness + w.trust + w.quality;
        assert!(
            w.relevance > side_total,
            "the side signals together must not outweigh relevance"
        );

        // The bound that actually matters: the side signals must not be able to bridge the
        // relevance gap across twenty positions, or a barely-relevant document could reach
        // the top on freshness and trust alone.
        let gap_over_20 = w.relevance * (1.0 - (-20.0f32 / RELEVANCE_DECAY).exp());
        assert!(
            side_total < gap_over_20,
            "side signals ({side_total}) could bridge a 20-position relevance gap ({gap_over_20})"
        );
    }

    #[test]
    fn adjacent_candidates_are_reorderable_but_distant_ones_are_not() {
        // Both halves of the tuning question, asserted together so a future weight change
        // cannot quietly break one of them.
        let w = Weights::default();
        let adjacent_gap =
            w.relevance * ((-0.0f32 / RELEVANCE_DECAY).exp() - (-1.0f32 / RELEVANCE_DECAY).exp());
        let side_total = w.freshness + w.trust + w.quality;
        assert!(
            side_total > adjacent_gap,
            "side signals ({side_total}) cannot reorder neighbours ({adjacent_gap}) — \
             freshness would be decorative"
        );
    }
}
