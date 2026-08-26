//! Index settings, versioned in code and applied by an idempotent migration.
//!
//! Settings live here rather than being poked in by hand so that "what the index is configured
//! to do" is reviewable in a diff. A test asserts the live settings match these.

use serde_json::{json, Value};

/// Index names. The alias indirection lets a reindex swap `documents_v1` → `documents_v2`
/// atomically; for now the alias and the concrete index share a name.
pub const DOCUMENTS: &str = "documents";
pub const COMMENTS: &str = "comments";
pub const SOURCES: &str = "sources";
/// The knowledge index (M8-T01.3). Its field names come from `xustive-knowledge` rather than being
/// written out again here: a searchable attribute naming a field the document does not emit fails
/// silently — it matches nothing, which reads as bad relevance rather than as a typo.
pub use xustive_knowledge::index::INDEX as KNOWLEDGE;

/// Longest all-stop-word query the phrase rescue will attempt (M7-T01.5). A genuine short
/// function-word query ("who is the", "the and") is a handful of tokens; beyond that, an exact
/// match on a long stop-word run is vanishingly unlikely and not worth a second round trip.
pub const MAX_STOPWORD_PHRASE_TOKENS: usize = 6;

/// Whether every token of `query` is a tokeniser stop word — the case where the engine strips the
/// whole query and returns nothing. Uses [`STOP_WORDS`], the same list the index is configured
/// with, and is shared by the API handler and the eval harness so neither can drift (BUG-003).
/// Empty or over-long queries return false: there is nothing to rescue, or it is not the
/// short-query case the rescue exists for.
pub fn is_all_stop_words(query: &str) -> bool {
    let mut tokens = query.split_whitespace().peekable();
    if tokens.peek().is_none() {
        return false;
    }
    let mut count = 0;
    for tok in tokens {
        count += 1;
        if count > MAX_STOPWORD_PHRASE_TOKENS {
            return false;
        }
        let lower = tok.to_lowercase();
        if !STOP_WORDS.contains(&lower.as_str()) {
            return false;
        }
    }
    true
}

/// The engine's deep-pagination bound (`pagination.maxTotalHits` on the documents index), shared
/// with the query handler so the pages the API *advertises* never exceed the pages the engine can
/// *serve* — the two drifting apart is how dead pagination links happen (BUG-002).
pub const MAX_TOTAL_HITS: usize = 2000;

/// The tokeniser stop words, shared between the index settings below and the query handler.
///
/// One source of truth on purpose: the API's short-query guard (M7-T01.5) needs to recognise a
/// query that Meilisearch will strip to nothing — which it can only do if it sees the *same* list
/// the index was configured with. A second hand-copied list would drift and reintroduce the bug.
pub const STOP_WORDS: &[&str] = &[
    "من", "في", "على", "الى", "إلى", "عن", "مع", "هذا", "هذه", "التي", "الذي", "le", "la", "les",
    "de", "des", "du", "et", "un", "une", "pour", "dans", "the", "and", "of", "to", "in", "for",
    "a", "is",
];

/// Generate the `synonyms` setting from the expansion lexicon.
///
/// The lexicon is the single source of truth: the same file feeds both this and the query-time
/// expander, so the two can never drift apart into "the engine thinks these are equivalent but
/// the expander does not".
///
/// Meilisearch synonyms are **directional** — declaring `oran → وهران` does not imply the
/// reverse — so [`Expander::meili_synonyms`] emits every pair both ways. Getting that wrong is
/// the subtle failure here: expansion appears to work, but only for people typing one script.
pub fn synonyms() -> Value {
    let map = xustive_lang::Expander::default().meili_synonyms();
    serde_json::to_value(map).unwrap_or(Value::Null)
}

/// Settings for the `documents` index.
///
/// The ordering of `searchableAttributes` is load-bearing: the `attribute` ranking rule uses it,
/// so a match in `title` outranks a match in `body`.
pub fn documents_settings() -> Value {
    json!({
        "searchableAttributes": [
            "title",
            "excerpt",
            "entities",
            "body",
            // Text OCR'd from a page's images (M3-T07), weighted below body — it is real content but
            // noisier than prose the page wrote itself.
            "media.ocr_text",
            "translit_body",
            "author.name"
        ],
        "filterableAttributes": [
            "source_type", "source_id", "domain", "language", "script",
            "sentiment.label", "published_at", "crawled_at", "is_nsfw",
            // The News vertical excludes documents with a guessed date, which filters on the
            // precision, not the timestamp — so it must be filterable (M3 verticals).
            "published_at_precision",
            "quality_score", "spam_score", "geo.wilaya", "topics", "robots_indexable",
            // The discovery channel that found each URL, so the admin console can facet the index by
            // provenance — what the crawler found directly vs what came through external tools like
            // federation (M7). Facetable = filterable in Meilisearch.
            "discovery",
            // So the repass job (M2-T06.9) can find documents that were enriched under load.
            "enrichment_level",
            // The fetched MIME, so a "Files" vertical can select PDFs (M2-T14.3).
            "content_type",
            // So image-similarity results can be resolved back to documents with `id IN [...]`
            // in one query (M3-T05). The primary key is not filterable unless declared.
            "id"
        ],
        "sortableAttributes": [
            "published_at", "crawled_at", "quality_score", "engagement.likes"
        ],
        "displayedAttributes": [
            "id", "title", "url", "canonical_url", "excerpt", "source_type", "source_id",
            "domain", "author", "published_at", "published_at_precision", "sentiment",
            "engagement", "language", "media", "simhash", "quality_score", "comments_count",
            // Shown in the admin document list as a provenance badge (crawler vs external tools).
            "discovery",
            // The concepts a document covers, so the query pipeline can aggregate them across a
            // page's top results into "related searches" (M7-T03) without a second round trip.
            "entities", "topics",
            // The extracted body's length. Not shown to searchers — it is here so the admin
            // console can tell an article from a navigation page at a glance, which the excerpt
            // cannot: the excerpt is capped, so measuring it measures the truncation.
            "body_len"
        ],
        // `words`..`exactness` are the Meilisearch defaults; the two custom rules add
        // freshness and quality as tie-breakers *after* textual relevance, never before it.
        "rankingRules": [
            "words", "typo", "proximity", "attribute", "sort", "exactness",
            "published_at:desc",
            "quality_score:desc"
        ],
        "typoTolerance": {
            "enabled": true,
            "minWordSizeForTypos": {
                // Arabic roots are short: the default of 5 lets "وهران" match "إيران".
                "oneTypo": 4,
                "twoTypos": 9
            },
            // Proper nouns must match exactly.
            "disableOnAttributes": ["entities"],
            "disableOnWords": [
                "وهران", "قسنطينة", "عنابة", "سطيف", "تلمسان", "بجاية",
                "سونلغاز", "سيال", "موبيليس", "جيزي", "أوريدو",
                "Oran", "Setif", "Annaba", "Bejaia", "Tlemcen",
                "Sonelgaz", "Seaal", "CNAS", "ANEM", "Mobilis", "Djezzy", "Ooredoo"
            ]
        },
        "faceting": {
            "maxValuesPerFacet": 100,
            "sortFacetValuesBy": { "*": "count" }
        },
        // Bounds deep-pagination cost. The UI caps at page 100 regardless.
        "pagination": { "maxTotalHits": MAX_TOTAL_HITS },
        "separatorTokens": ["|", "·", "—", "–"],
        // Keep handles and hashtags as single tokens.
        "nonSeparatorTokens": ["@", "#", "_"],
        // Stops the tokeniser splitting known multi-part entity names.
        "dictionary": [
            "سونلغاز", "الجزائر", "Sonelgaz", "CNAS", "ANEM", "Seaal",
            "Ooredoo", "Djezzy", "Mobilis", "Naftal", "Sonatrach"
        ],
        "stopWords": STOP_WORDS,
        // Generated from data/expansion/*.tsv, the same files the query-time expander reads.
        "synonyms": synonyms()
    })
}

/// Settings for the `comments` index.
///
/// No custom freshness rule: comment recency is inherited from the parent document during the
/// re-rank, so ranking comments by their own date here would double-count it.
pub fn comments_settings() -> Value {
    json!({
        "searchableAttributes": ["body", "author.name"],
        "filterableAttributes": [
            "document_id", "source_type", "sentiment.label", "published_at", "language"
        ],
        "sortableAttributes": ["published_at", "likes"],
        "displayedAttributes": [
            "id", "document_id", "body", "author", "published_at", "sentiment",
            "likes", "language", "source_type"
        ],
        "rankingRules": ["words", "typo", "proximity", "attribute", "sort", "exactness"],
        "typoTolerance": {
            "enabled": true,
            "minWordSizeForTypos": { "oneTypo": 4, "twoTypos": 9 }
        },
        "pagination": { "maxTotalHits": 1000 }
    })
}

/// Settings for the `sources` registry index.
pub fn sources_settings() -> Value {
    json!({
        "searchableAttributes": ["display_name", "id", "notes"],
        "filterableAttributes": ["kind", "trust_tier", "approved", "legal_basis"],
        "sortableAttributes": ["last_run_at"],
        "rankingRules": ["words", "typo", "proximity", "attribute", "exactness"]
    })
}

/// The knowledge index: entities, matched by name.
///
/// Only names and descriptions are searchable. The nested entity carries image credits, licence
/// strings and authority identifiers, and an entity found by the contents of its own image credit
/// would be a match no reader could explain.
///
/// `prominence` is sortable because it is the tie-breaker when two entities share a name — the
/// number of language editions that bothered to write about something is a crude measure of how
/// likely a bare name means it, and a crude honest one.
pub fn knowledge_settings() -> Value {
    use xustive_knowledge::index as k;
    json!({
        "searchableAttributes": [k::F_NAMES, k::F_DESCRIPTIONS],
        "filterableAttributes": [k::F_KIND],
        "sortableAttributes": [k::F_PROMINENCE, k::F_UPDATED_AT],
        // No `typo` rule ahead of `exactness`: for a name, an exact match is almost always the
        // right answer, and typo tolerance that outranks it turns `Oran` into `Orano`.
        "rankingRules": ["words", "exactness", "typo", "proximity", "attribute", "sort"],
        "stopWords": [],
        // A name is short. Two typos in a short name is a different name.
        "typoTolerance": {
            "minWordSizeForTypos": { "oneTypo": 5, "twoTypos": 9 }
        }
    })
}

/// Every index and its settings, in creation order.
pub fn all() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        (DOCUMENTS, "id", documents_settings()),
        (COMMENTS, "id", comments_settings()),
        (SOURCES, "id", sources_settings()),
        (
            KNOWLEDGE,
            xustive_knowledge::index::F_ID,
            knowledge_settings(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_all_stop_word_query_is_recognised() {
        // The cases the phrase rescue must catch: every token is a stop word.
        assert!(is_all_stop_words("the and"));
        assert!(is_all_stop_words("من في"));
        assert!(is_all_stop_words("The Of")); // case-insensitive
                                              // A query with any content word is a normal query — the primary leg handles it.
        assert!(!is_all_stop_words("the president"));
        assert!(!is_all_stop_words("سونلغاز في"));
        // Nothing to rescue, or too long to be the short-query case.
        assert!(!is_all_stop_words(""));
        assert!(!is_all_stop_words("the a is of and in for to")); // over the token cap
    }

    fn strs(v: &Value, key: &str) -> Vec<String> {
        v[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn title_outranks_body_via_attribute_order() {
        let s = documents_settings();
        let searchable = strs(&s, "searchableAttributes");
        let title = searchable.iter().position(|a| a == "title").unwrap();
        let body = searchable.iter().position(|a| a == "body").unwrap();
        assert!(
            title < body,
            "title must precede body for the attribute ranking rule"
        );
    }

    #[test]
    fn custom_ranking_rules_come_after_textual_relevance() {
        let s = documents_settings();
        let rules = strs(&s, "rankingRules");
        let exactness = rules.iter().position(|r| r == "exactness").unwrap();
        let freshness = rules
            .iter()
            .position(|r| r.starts_with("published_at"))
            .unwrap();
        assert!(
            exactness < freshness,
            "freshness must not outrank textual relevance"
        );
    }

    #[test]
    fn typo_thresholds_are_tuned_for_arabic() {
        let s = documents_settings();
        // Default is 5; Arabic place names are short enough that 5 causes real errors.
        assert_eq!(s["typoTolerance"]["minWordSizeForTypos"]["oneTypo"], 4);
        assert_eq!(s["typoTolerance"]["minWordSizeForTypos"]["twoTypos"], 9);
    }

    #[test]
    fn entities_are_exempt_from_typo_tolerance() {
        let s = documents_settings();
        let disabled = strs(&s["typoTolerance"], "disableOnAttributes");
        assert!(disabled.contains(&"entities".to_string()));
    }

    #[test]
    fn wilaya_names_are_protected_from_typo_matching() {
        let s = documents_settings();
        let words = strs(&s["typoTolerance"], "disableOnWords");
        assert!(
            words.contains(&"وهران".to_string()),
            "Oran must be typo-protected"
        );
        assert!(words.contains(&"Oran".to_string()));
    }

    #[test]
    fn every_filterable_facet_the_api_exposes_is_declared() {
        let s = documents_settings();
        let filterable = strs(&s, "filterableAttributes");
        for required in ["source_type", "sentiment.label", "published_at", "language"] {
            assert!(
                filterable.contains(&required.to_string()),
                "{required} must be filterable"
            );
        }
    }

    #[test]
    fn every_field_the_filter_builder_can_reference_is_filterable() {
        // The bug this guards: the News vertical filtered on `published_at_precision`, which was not
        // filterable, so Meilisearch rejected the query and the tab showed "something went wrong".
        // Every field `Filters::to_expression` can emit a clause for must be declared here, or the
        // vertical/facet that uses it fails only at runtime, against the live index.
        let s = documents_settings();
        let filterable = strs(&s, "filterableAttributes");
        for field in [
            "source_type",
            "sentiment.label",
            "language",
            "published_at",
            "published_at_precision", // News vertical
            "domain",
            "content_type", // Files vertical
            "spam_score",
        ] {
            assert!(
                filterable.contains(&field.to_string()),
                "{field} is used in a filter clause but is not filterable"
            );
        }
    }

    #[test]
    fn body_is_searchable_but_not_displayed() {
        // Full text is retrieved for ranking but never served — that is the search-engine
        // posture on copyright, and it is enforced by the index, not by the handler.
        let s = documents_settings();
        assert!(strs(&s, "searchableAttributes").contains(&"body".to_string()));
        assert!(!strs(&s, "displayedAttributes").contains(&"body".to_string()));
    }

    #[test]
    fn deep_pagination_is_bounded() {
        let s = documents_settings();
        assert_eq!(s["pagination"]["maxTotalHits"], 2000);
    }

    #[test]
    fn comments_have_no_custom_freshness_rule() {
        let s = comments_settings();
        let rules = strs(&s, "rankingRules");
        assert!(
            !rules.iter().any(|r| r.contains(':')),
            "comments inherit recency from parent"
        );
    }

    #[test]
    fn all_indexes_declare_a_primary_key() {
        for (name, pk, _) in all() {
            assert_eq!(pk, "id", "{name} should key on id");
        }
        // A tripwire, not a fact: an index added here needs settings written on purpose, and the
        // count failing is how the author is asked whether they did that. Four since M8 added
        // `knowledge`.
        assert_eq!(all().len(), 4);
    }
}
