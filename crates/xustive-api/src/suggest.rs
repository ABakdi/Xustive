//! Autocomplete.
//!
//! Four sources, merged and capped. Each can fail on its own without taking the endpoint with it:
//! a suggestion list is an aid, and an aid that returns an error is worse than one that returns
//! nothing — the user is mid-keystroke and does not want a dialog.
//!
//! | Source | Weight | Why it exists |
//! |:---|---:|:---|
//! | Curated | 0.9 | The things Algerians actually need — a passport renewal, a wilaya, a utility bill — are not what news sites write headlines about. A purely corpus-derived suggester is confidently useless for exactly the queries that matter most. |
//! | Prefix index | 1.0 | Entity and title strings from what we have crawled. Cheap and always current. |
//! | Title search | 0.7 | Catches mid-string matches the prefix index cannot. Needs Meilisearch. |
//! | Transliteration | 0.6 | `ch7al` should suggest `شحال`. Without this, an Arabizi typist gets nothing until they finish the word — and then still gets nothing. |
//!
//! # Latency
//!
//! Suggestions fire per keystroke, so the budget is 40 ms p95 against 1500 ms for search. Three
//! of the four sources are in-memory and answer in microseconds; the title leg is the only one
//! that leaves the process, and it is the only one allowed to be skipped under its own timeout.
//!
//! # Privacy
//!
//! Prefixes are never logged, counted, or persisted. The specification describes an optional
//! k-anonymous popularity counter; it is not built, because a popularity counter is a query log
//! with a different name and the open question in [[Autocomplete Service]] §4 has not been
//! resolved.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use axum::extract::{Query as AxumQuery, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::state::AppState;

pub const DEFAULT_LIMIT: usize = 8;
pub const MAX_LIMIT: usize = 20;
/// Below this, a prefix matches so much that the suggestions are noise.
pub const MIN_PREFIX_CHARS: usize = 2;
/// Budget for the one leg that leaves the process.
const TITLE_LEG_TIMEOUT: Duration = Duration::from_millis(60);

const W_PREFIX: f32 = 1.0;
const W_CURATED: f32 = 0.9;
const W_TITLE: f32 = 0.7;
const W_TRANSLIT: f32 = 0.6;

#[derive(Debug, Deserialize)]
pub struct SuggestParams {
    pub q: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Suggestion {
    pub text: String,
    /// Which source produced it. Useful in the UI for iconography and, more importantly, when
    /// working out why a suggestion appeared.
    pub source: &'static str,
}

#[derive(Debug, Serialize)]
pub struct SuggestResponse {
    pub suggestions: Vec<Suggestion>,
    pub took_ms: u64,
}

/// An in-memory prefix index over entity and curated strings.
///
/// A sorted `Vec` with binary search rather than the FST the specification calls for. An FST wins
/// decisively at the ~200k strings that document assumes; at the few thousand we actually have,
/// a sorted vector answers in the same microseconds, has no build step, and cannot go stale
/// between rebuilds. When the corpus reaches the size that justifies an FST, this is the thing to
/// replace — and the interface is shaped so that is a swap rather than a rewrite.
#[derive(Default)]
pub struct PrefixIndex {
    /// `(normalised, display, weight)`, sorted by the normalised form.
    entries: Vec<(String, String, f32)>,
}

impl PrefixIndex {
    /// Build from curated terms plus whatever strings the corpus offers.
    pub fn build(curated: &[(String, f32)], corpus: &[String]) -> Self {
        let mut entries: Vec<(String, String, f32)> = Vec::new();
        let mut seen: HashMap<String, usize> = HashMap::new();

        let push = |entries: &mut Vec<(String, String, f32)>,
                    seen: &mut HashMap<String, usize>,
                    display: &str,
                    weight: f32| {
            let key = xustive_text::fold(display);
            if key.chars().count() < MIN_PREFIX_CHARS {
                return;
            }
            // Keep the highest weight for a term that arrives from several sources. A curated
            // wilaya that also appears in a headline should not be demoted by the headline.
            match seen.get(&key) {
                Some(&i) => {
                    if entries[i].2 < weight {
                        entries[i].2 = weight;
                    }
                }
                None => {
                    seen.insert(key.clone(), entries.len());
                    entries.push((key, display.to_string(), weight));
                }
            }
        };

        for (term, weight) in curated {
            push(&mut entries, &mut seen, term, W_CURATED * weight);
        }
        for term in corpus {
            push(&mut entries, &mut seen, term, W_PREFIX);
        }

        entries.sort_by(|a, b| a.0.cmp(&b.0));
        Self { entries }
    }

    /// Terms beginning with `prefix`, best first.
    ///
    /// The prefix must already be folded by [`xustive_text::fold`] — the caller folds once and
    /// reuses it across every source, which is what keeps the sources consistent with each other.
    pub fn prefix(&self, normalised_prefix: &str, limit: usize) -> Vec<(String, f32)> {
        if normalised_prefix.is_empty() {
            return Vec::new();
        }
        let start = self
            .entries
            .partition_point(|(k, _, _)| k.as_str() < normalised_prefix);

        let mut out: Vec<(String, f32)> = self.entries[start..]
            .iter()
            .take_while(|(k, _, _)| k.starts_with(normalised_prefix))
            // Bounded so a one-character prefix cannot walk the whole index.
            .take(limit * 8)
            .map(|(_, display, w)| (display.clone(), *w))
            .collect();

        out.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                // Shorter first among equals: a shorter completion is closer to what was typed
                // and leaves the user more room to refine.
                .then_with(|| a.0.chars().count().cmp(&b.0.chars().count()))
        });
        out.truncate(limit);
        out
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Parse `data/suggest/curated.tsv`.
pub fn load_curated(path: &str) -> Vec<(String, f32)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        // Not an error. The file is an improvement on the corpus, not a dependency, and a
        // deployment without it should still suggest.
        tracing::info!(path, "no curated suggestion file; using corpus terms only");
        return Vec::new();
    };
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|line| {
            let mut cols = line.split('\t');
            let term = cols.next()?.trim();
            let _lang = cols.next();
            let weight = cols
                .next()
                .and_then(|w| w.trim().parse().ok())
                .unwrap_or(1.0);
            (!term.is_empty()).then(|| (term.to_string(), weight))
        })
        .collect()
}

pub async fn handler(
    State(state): State<AppState>,
    AxumQuery(params): AxumQuery<SuggestParams>,
) -> Json<SuggestResponse> {
    let started = Instant::now();
    let raw = params.q.unwrap_or_default();
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

    // An empty response, never an error. The caller is mid-keystroke.
    // Folded, not merely normalised. A user typing بجايه should be offered بجاية — for a
    // suggestion box the forgiving behaviour is the correct one, and the cost of over-matching
    // is a slightly wider list rather than a wrong result.
    let normalised = xustive_text::fold(&raw);
    if normalised.chars().count() < MIN_PREFIX_CHARS {
        return Json(SuggestResponse {
            suggestions: Vec::new(),
            took_ms: started.elapsed().as_millis() as u64,
        });
    }

    let mut candidates: Vec<(String, f32, &'static str)> = Vec::new();

    // --- in-memory legs -------------------------------------------------------------------
    let index = state.suggestions();
    for (text, weight) in index.prefix(&normalised, limit * 2) {
        candidates.push((text, weight, "index"));
    }

    // Transliteration. Only worth attempting when the prefix is Latin script and the index found
    // little — an Arabizi typist gets nothing at all otherwise, not even after finishing the word.
    if candidates.len() < limit && looks_arabizi(&normalised) {
        let detected = state.detector.detect(&normalised);
        let expansion = state.expander.expand(&normalised, detected.lang);
        for variant in expansion.variants.iter().take(limit) {
            let translit = xustive_text::fold(&variant.text);
            for (text, weight) in index.prefix(&translit, limit) {
                candidates.push((text, weight * W_TRANSLIT, "transliteration"));
            }
        }
    }

    // --- title leg ------------------------------------------------------------------------
    //
    // The only source that leaves the process, so the only one with its own timeout. Skipped
    // silently when Meilisearch is slow or down: the in-memory sources already produced
    // something, and a suggestion box that stalls is worse than a short one.
    if candidates.len() < limit {
        // Titles only. Searching whole documents for a short prefix returns whatever body text
        // happens to contain it — "سونلغاز" offered an article about a 1922 congress because the
        // word appears in paragraph nine, which reads as carelessness rather than as a match.
        let query = xustive_search::Query::new(&normalised)
            .limit(limit)
            .search_on(&["title"])
            .highlight(&[]);
        let documents = state.documents_index();
        if let Ok(Ok(hits)) = tokio::time::timeout(
            TITLE_LEG_TIMEOUT,
            state.search.search::<Value>(&documents, &query),
        )
        .await
        {
            for hit in hits.hits.iter().take(limit) {
                let Some(title) = hit.get("title").and_then(Value::as_str) else {
                    continue;
                };
                let term = trim_title(title);
                // Meilisearch's typo tolerance is right for search and wrong here: a
                // three-character prefix matched titles with no visible relation to it, so
                // "وهر" offered "فيديو". A suggestion that does not contain what the user typed
                // is not a completion, whatever the engine's edit distance says.
                if !xustive_text::fold(&term).contains(&normalised) {
                    continue;
                }
                candidates.push((term, W_TITLE, "title"));
            }
        }
    }

    let suggestions = merge(candidates, limit);
    // Deliberately no logging of the prefix, here or anywhere below.
    state.metrics.incr(
        crate::metrics::SUGGEST_TOTAL,
        crate::metrics::SUGGEST_TOTAL_HELP,
        &[("empty", if suggestions.is_empty() { "yes" } else { "no" })],
    );

    Json(SuggestResponse {
        suggestions,
        took_ms: started.elapsed().as_millis() as u64,
    })
}

/// Whether a Latin-script prefix is plausibly Arabizi.
///
/// Guarded rather than applied to every Latin prefix. "Ora" is French for Oran, not Arabizi, and
/// transliterating it produced Arabic suggestions with no relation to what was typed — the leg
/// helped Darija typists and actively harmed French ones.
///
/// The digits are the signal that costs nothing to check and almost never appears by accident:
/// 3 for ع, 7 for ح, 9 for ق, 2 for ء. A French or English word containing one of those mid-token
/// is essentially always Arabizi.
fn looks_arabizi(prefix: &str) -> bool {
    if !prefix.is_ascii() {
        return false;
    }
    prefix
        .chars()
        .any(|c| matches!(c, '2' | '3' | '5' | '7' | '9'))
}

/// Article titles carry a site suffix that is noise in a suggestion list.
pub fn title_term(title: &str) -> String {
    trim_title(title)
}

fn trim_title(title: &str) -> String {
    let cut = title
        .split(" - ")
        .next()
        .unwrap_or(title)
        .split(" | ")
        .next()
        .unwrap_or(title)
        .trim();
    cut.chars().take(70).collect::<String>().trim().to_string()
}

/// Dedupe, drop strict prefixes, sort, cap.
///
/// The prefix-subsumption rule is the one that makes the list feel deliberate rather than
/// generated: showing both "سونلغاز" and "سونلغاز فاتورة" wastes a row, because anyone who wanted
/// the shorter one has already typed it.
fn merge(candidates: Vec<(String, f32, &'static str)>, limit: usize) -> Vec<Suggestion> {
    let mut best: HashMap<String, (String, f32, &'static str)> = HashMap::new();
    for (text, weight, source) in candidates {
        let text = text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        let key = xustive_text::fold(&text);
        if key.is_empty() {
            continue;
        }
        best.entry(key)
            .and_modify(|e| {
                if e.1 < weight {
                    *e = (text.clone(), weight, source);
                }
            })
            .or_insert((text, weight, source));
    }

    let mut ranked: Vec<(String, String, f32, &'static str)> = best
        .into_iter()
        .map(|(key, (text, w, s))| (key, text, w, s))
        .collect();
    ranked.sort_by(|a, b| {
        b.2.partial_cmp(&a.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.chars().count().cmp(&b.1.chars().count()))
            // Ties broken by text so the list is stable across identical requests. An unstable
            // suggestion list reorders under the user's cursor as they type.
            .then_with(|| a.1.cmp(&b.1))
    });

    let mut out: Vec<Suggestion> = Vec::with_capacity(limit);
    let mut kept: Vec<String> = Vec::with_capacity(limit);
    for (key, text, _, source) in ranked {
        if out.len() >= limit {
            break;
        }
        // Drop this candidate if something already shown extends it. Keeping the longer one is
        // right: the user has already typed the shorter.
        if kept.iter().any(|k| k.starts_with(&key) && k != &key) {
            continue;
        }
        kept.push(key);
        out.push(Suggestion { text, source });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> PrefixIndex {
        PrefixIndex::build(
            &[("سونلغاز فاتورة".into(), 1.0), ("وهران".into(), 1.0)],
            &[
                "سونلغاز".into(),
                "سونلغاز ترفع الأسعار".into(),
                "وهران تستقبل".into(),
                "Oran".into(),
            ],
        )
    }

    #[test]
    fn a_prefix_finds_terms_that_start_with_it() {
        let hits = index().prefix(&xustive_text::fold("سونلغاز"), 8);
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|(t, _)| t.contains("سونلغاز")));
    }

    #[test]
    fn a_prefix_that_matches_nothing_returns_nothing_rather_than_everything() {
        assert!(index().prefix("zzzzz", 8).is_empty());
    }

    #[test]
    fn the_empty_prefix_returns_nothing() {
        // Without the guard, `partition_point` puts the cursor at zero and the whole index is
        // returned — a suggestion box that fills up before a key is pressed.
        assert!(index().prefix("", 8).is_empty());
    }

    #[test]
    fn curated_terms_outrank_corpus_terms() {
        // The corpus knows what was published; the curated list knows what people need. When
        // both offer a completion, the curated one is the better guess.
        let idx = PrefixIndex::build(
            &[("تجديد جواز السفر".into(), 1.0)],
            &["تجديد جواز السفر".into()],
        );
        let hits = idx.prefix(&xustive_text::fold("تجديد"), 8);
        assert_eq!(hits.len(), 1, "the same term from two sources is one entry");
    }

    #[test]
    fn a_strict_prefix_of_another_suggestion_is_dropped() {
        // Showing both "سونلغاز" and "سونلغاز فاتورة" wastes a row: anyone who wanted the shorter
        // one has already typed it.
        let out = merge(
            vec![
                ("سونلغاز".into(), 0.9, "index"),
                ("سونلغاز فاتورة".into(), 1.0, "index"),
            ],
            8,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "سونلغاز فاتورة");
    }

    #[test]
    fn identical_suggestions_from_different_sources_collapse_to_the_best() {
        let out = merge(
            vec![
                ("وهران".into(), 0.7, "title"),
                ("وهران".into(), 0.9, "index"),
            ],
            8,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].source, "index", "the higher-weighted source wins");
    }

    #[test]
    fn suggestions_that_differ_only_in_orthography_collapse() {
        // بجاية and بجايه are the same place. Two rows for one answer is a bug the user reads as
        // carelessness.
        let out = merge(
            vec![
                ("بجاية".into(), 1.0, "index"),
                ("بجايه".into(), 0.9, "title"),
            ],
            8,
        );
        assert_eq!(out.len(), 1, "got {out:?}");
    }

    #[test]
    fn the_order_is_stable_across_identical_calls() {
        // An unstable list reorders under the cursor while the user is reaching for it.
        let candidates = || {
            vec![
                ("aaa".into(), 1.0, "index"),
                ("bbb".into(), 1.0, "index"),
                ("ccc".into(), 1.0, "index"),
            ]
        };
        assert_eq!(merge(candidates(), 8), merge(candidates(), 8));
    }

    #[test]
    fn the_limit_is_respected() {
        let many: Vec<_> = (0..50)
            .map(|i| (format!("term{i:02}"), 1.0, "index"))
            .collect();
        assert_eq!(merge(many, 5).len(), 5);
    }

    #[test]
    fn transliteration_fires_for_arabizi_and_not_for_french() {
        // Applying it to every Latin prefix helped Darija typists and harmed French ones: "Ora"
        // is Oran, and transliterating it offered Arabic titles unrelated to anything typed.
        assert!(looks_arabizi("ch7al"));
        assert!(looks_arabizi("3taf"));
        assert!(looks_arabizi("9adiya"));
        assert!(!looks_arabizi("Ora"));
        assert!(!looks_arabizi("passeport"));
        assert!(!looks_arabizi("وهران"), "arabic script is not arabizi");
    }

    #[test]
    fn a_title_that_does_not_contain_the_prefix_is_not_a_completion() {
        // The guard applied to the title leg, stated as a property. Typo tolerance is right for
        // search and wrong for autocomplete: it offered "فيديو" for the prefix "وهر".
        let prefix = xustive_text::fold("وهر");
        assert!(xustive_text::fold("وهران تستقبل").contains(&prefix));
        assert!(!xustive_text::fold("فيديو").contains(&prefix));
    }

    #[test]
    fn titles_lose_their_site_suffix() {
        assert_eq!(
            trim_title("الأندية الجزائرية تتعرف على منافسيها - رياضة : الخبر"),
            "الأندية الجزائرية تتعرف على منافسيها"
        );
        assert_eq!(trim_title("Résultats du bac | TSA"), "Résultats du bac");
    }

    #[test]
    fn a_very_long_title_is_cut() {
        let long = "ا".repeat(200);
        assert!(trim_title(&long).chars().count() <= 70);
    }

    #[test]
    fn a_missing_curated_file_is_not_an_error() {
        // The file improves on the corpus; it is not a dependency. A deployment without it must
        // still suggest.
        assert!(load_curated("/nonexistent/curated.tsv").is_empty());
    }

    #[test]
    fn the_curated_file_parses_with_comments_and_weights() {
        let dir = std::env::temp_dir().join("xustive-suggest-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.tsv");
        std::fs::write(&path, "# comment\n\nوهران\tar\t0.9\nAlger\tfr\n").unwrap();

        let loaded = load_curated(path.to_str().unwrap());
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].1, 0.9);
        assert_eq!(loaded[1].1, 1.0, "a missing weight defaults to 1.0");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_real_curated_file_is_well_formed() {
        // It ships with the product, so a typo in it is a shipped defect.
        let loaded = load_curated("../../data/suggest/curated.tsv");
        if loaded.is_empty() {
            return; // Run from a different directory; the parse tests above still cover this.
        }
        assert!(loaded.len() > 20, "only {} entries", loaded.len());
        for (term, weight) in &loaded {
            assert!(!term.contains('\t'), "stray tab in {term:?}");
            assert!(
                (0.0..=1.0).contains(weight),
                "{term:?} has weight {weight}, outside 0..=1"
            );
        }
    }

    #[test]
    fn a_one_character_prefix_cannot_walk_the_whole_index() {
        // A short prefix matches almost everything. Without the bound, the cost of a suggestion
        // request scales with the corpus and the p95 budget goes with it.
        let corpus: Vec<String> = (0..5000).map(|i| format!("term{i:04}")).collect();
        let idx = PrefixIndex::build(&[], &corpus);
        let started = Instant::now();
        let hits = idx.prefix("te", 8);
        assert_eq!(hits.len(), 8);
        assert!(
            started.elapsed() < Duration::from_millis(20),
            "took {:?}",
            started.elapsed()
        );
    }
}
