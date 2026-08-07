//! `GET /api/v1/search`.
//!
//! Current scope: normalise, detect language, filter, retrieve, shape. Query expansion,
//! re-ranking and the summary are still to come; the response *shape* is already the final one
//! so the UI does not have to change underneath as they land.

use std::time::Instant;

use axum::extract::{Query as AxumQuery, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use xustive_core::{DatePrecision, Lang, SentimentLabel, SourceType};
use xustive_lang::Detection;
use xustive_search::{filter::Filters, rank, Query};
use xustive_text::script::{self, Script};

use crate::error::ApiError;
use crate::metrics;
use crate::state::AppState;

/// Hard cap on query length, matching the API contract.
pub const MAX_QUERY_CHARS: usize = 512;
/// Spam suppression threshold. Documents above it stay indexed but out of default results.
const SPAM_THRESHOLD: f32 = 0.8;

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub q: Option<String>,
    #[serde(default)]
    pub page: Option<usize>,
    #[serde(default)]
    pub hits_per_page: Option<usize>,
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub sentiment: Option<String>,
    #[serde(default)]
    pub from: Option<i64>,
    #[serde(default)]
    pub to: Option<i64>,
    #[serde(default)]
    pub sort: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct QueryInfo {
    pub raw: String,
    pub normalized: String,
    pub language: &'static str,
    pub language_confidence: f32,
    pub expanded_terms: Vec<String>,
    pub corrected: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Pagination {
    pub page: usize,
    pub hits_per_page: usize,
    pub total_hits: usize,
    pub total_pages: usize,
    pub estimated: bool,
}

#[derive(Debug, Serialize)]
pub struct SentimentOut {
    pub label: &'static str,
    pub score: f32,
}

#[derive(Debug, Serialize)]
pub struct ResultCard {
    pub id: String,
    pub title: String,
    pub url: String,
    pub display_url: String,
    pub excerpt: String,
    pub source_type: String,
    pub source_name: String,
    pub author: Value,
    pub published_at: i64,
    pub published_at_precision: String,
    pub sentiment: Option<SentimentOut>,
    pub engagement: Value,
    pub language: String,
    pub thumbnail_url: Option<String>,
    pub matched_comments: Vec<Value>,
    pub score: f32,
    /// Near-duplicates folded into this result, shown as "+N similar".
    #[serde(skip_serializing_if = "is_zero")]
    pub similar_count: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub query: QueryInfo,
    /// Handed to `POST /v1/summary`. `None` when there is nothing to summarise, summaries are
    /// switched off, or the caller is past the first page. The client hides the block when it is
    /// absent rather than showing an empty one.
    pub summary_token: Option<String>,
    /// An instant answer, when the query *is* the question.
    ///
    /// Computed in microseconds from the raw query, so it rides along with the search rather than
    /// arriving separately. `None` is the overwhelmingly common case and renders nothing —
    /// an unwanted card pushing results down is worse than no card.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instant: Option<xustive_tools::Answer>,
    pub pagination: Pagination,
    pub took_ms: u64,
    pub results: Vec<ResultCard>,
    pub facets: Value,
}

/// Below this many primary hits, try the expanded leg.
///
/// Not zero. A query that returns three results is not obviously fine — for Arabizi it usually
/// means a couple of incidental Latin-script matches while the Arabic documents that actually
/// answer it were never reached.
const EXPANSION_THRESHOLD: usize = 5;

/// Retrieve again with expanded terms and merge into `hits`.
///
/// Returns the terms tried, for the response's `expanded_terms`. Failures are swallowed on
/// purpose: this leg is an improvement on the primary result, never a precondition for it, and a
/// search that already succeeded must not fail because an optional second attempt did.
#[allow(clippy::too_many_arguments)]
async fn expand_and_merge(
    state: &AppState,
    normalized: &str,
    language: xustive_core::Lang,
    index: &str,
    hits: &mut xustive_search::Hits<Value>,
    filters: &Filters,
    sort: &[&str],
) -> Vec<String> {
    let expansion = state.expander.expand(normalized, language);
    if expansion.is_empty() {
        return Vec::new();
    }

    // One query containing every variant rather than one query per variant. Meilisearch treats
    // the extra terms as optional, so this is a single round trip instead of N, and the
    // re-ranker sorts out which of them actually mattered.
    let terms: Vec<String> = expansion
        .variants
        .iter()
        .map(|v| v.text.clone())
        .take(MAX_EXPANDED_TERMS)
        .collect();
    if terms.is_empty() {
        return Vec::new();
    }

    let mut expanded = Query::new(terms.join(" ")).limit(state.config.search.candidate_pool);
    if let Some(expr) = filters.to_expression(SPAM_THRESHOLD) {
        expanded = expanded.filter(expr);
    }
    if !sort.is_empty() {
        expanded = expanded.sort(sort);
    }

    let Ok(extra) = state.search.search::<Value>(index, &expanded).await else {
        return terms;
    };

    // Merge, keeping the primary leg's order first. A document found by both legs belongs where
    // the primary put it: it matched the query as typed, which is stronger evidence than
    // matching a transliteration of it.
    let mut seen: std::collections::HashSet<String> = hits
        .hits
        .iter()
        .filter_map(|h| h.get("id")?.as_str().map(str::to_string))
        .collect();
    for hit in extra.hits {
        let Some(id) = hit.get("id").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        if seen.insert(id) {
            hits.hits.push(hit);
        }
    }
    hits.estimated_total_hits = hits.estimated_total_hits.max(extra.estimated_total_hits);

    state.metrics.incr(
        metrics::EXPANSION_USED,
        metrics::EXPANSION_USED_HELP,
        &[("lang", language.as_str())],
    );
    terms
}

/// Cap on variants sent to the engine.
///
/// Every extra term widens recall and dilutes precision. Beyond a handful the query stops being
/// about what the user asked and starts being about what the lexicon happens to contain.
const MAX_EXPANDED_TERMS: usize = 12;

pub async fn handler(
    State(state): State<AppState>,
    AxumQuery(params): AxumQuery<SearchParams>,
) -> Result<Json<SearchResponse>, ApiError> {
    let started = Instant::now();

    // --- validate -------------------------------------------------------------------
    let raw = params.q.clone().unwrap_or_default();
    // Computed from the raw query before it is consumed by the response. Normalisation folds the
    // characters an expression is made of, so `45*1.19` has to be seen intact to parse.
    let instant = xustive_tools::best(&raw);
    if raw.trim().is_empty() {
        return Err(ApiError::MissingQuery);
    }
    if raw.chars().count() > MAX_QUERY_CHARS {
        return Err(ApiError::QueryTooLong {
            max: MAX_QUERY_CHARS,
        });
    }

    let cfg = &state.config.search;
    let hits_per_page = params
        .hits_per_page
        .unwrap_or(cfg.default_hits_per_page)
        .clamp(1, cfg.max_hits_per_page);
    let page = params.page.unwrap_or(1).max(1);

    let filters = parse_filters(&params)?.normalise();
    let sort = match params.sort.as_deref() {
        None | Some("relevance") => Vec::new(),
        Some("recency") => vec!["published_at:desc"],
        Some(other) => {
            return Err(ApiError::InvalidParam {
                param: "sort",
                detail: format!("{other:?} is not one of relevance, recency"),
            })
        }
    };

    // --- normalise and detect -------------------------------------------------------
    let normalized = xustive_text::normalize(&raw);
    let detect_started = Instant::now();
    let detection = detect_language(&state, &normalized, params.lang.as_deref());
    state.metrics.observe(
        metrics::SEARCH_DURATION,
        metrics::SEARCH_DURATION_HELP,
        &[("stage", "detect")],
        detect_started.elapsed().as_secs_f64(),
    );
    let (language, confidence) = (detection.lang, detection.confidence);
    state.metrics.incr(
        metrics::LANG_DETECTED,
        metrics::LANG_DETECTED_HELP,
        &[
            ("lang", language.as_str()),
            ("script", script_label(detection.script)),
        ],
    );

    // --- retrieve -------------------------------------------------------------------
    // Pull a candidate pool rather than one page: re-ranking can only reorder what it is
    // given, so paging at the engine would freeze the engine's ordering into the result.
    let offset = (page - 1) * hits_per_page;
    let pool = cfg.candidate_pool.max(hits_per_page);
    let mut query = Query::new(&normalized)
        .limit(pool)
        .offset(0)
        .facets(&["source_type", "sentiment.label", "language"])
        .highlight(&["excerpt", "title"]);
    if let Some(expr) = filters.to_expression(SPAM_THRESHOLD) {
        query = query.filter(expr);
    }
    if !sort.is_empty() {
        query = query.sort(&sort);
    }

    let retrieval_started = Instant::now();
    let index = state.documents_index();
    let mut hits = state
        .search
        .search::<Value>(&index, &query)
        .await
        .map_err(ApiError::from)?;

    // --- expanded leg ---------------------------------------------------------------------
    //
    // A second retrieval, run only when the first found too little. The eval harness measured
    // 19 of 20 Arabizi queries returning nothing at all — `ch7al`, `alouzir alaoul`, `3taf` —
    // because the expander existed but nothing called it. An Algeria-first engine where Darija
    // typed in Latin script finds zero results is not doing the thing it exists for.
    //
    // Conditional rather than always-on: expansion costs a round trip, and for a query that
    // already retrieved well it adds only weaker matches that the re-ranker then has to push
    // back down.
    let expanded_terms = if hits.hits.len() < EXPANSION_THRESHOLD {
        expand_and_merge(
            &state,
            &normalized,
            language,
            &index,
            &mut hits,
            &filters,
            &sort,
        )
        .await
    } else {
        Vec::new()
    };

    state.metrics.observe(
        metrics::SEARCH_DURATION,
        metrics::SEARCH_DURATION_HELP,
        &[("stage", "retrieve")],
        retrieval_started.elapsed().as_secs_f64(),
    );

    // --- re-rank --------------------------------------------------------------------
    let rerank_started = Instant::now();
    let trust = state.trust_tiers.as_ref();
    let ranked = rank::rerank(
        &hits.hits,
        &normalized,
        xustive_core::now_unix(),
        trust,
        &state.ranking,
    );
    state.metrics.observe(
        metrics::SEARCH_DURATION,
        metrics::SEARCH_DURATION_HELP,
        &[("stage", "rerank")],
        rerank_started.elapsed().as_secs_f64(),
    );

    // --- shape ----------------------------------------------------------------------
    let results: Vec<ResultCard> = ranked
        .iter()
        .skip(offset)
        .take(hits_per_page)
        .map(|r| {
            let mut card = to_card(&r.hit);
            card.score = r.score;
            card.similar_count = r.collapsed.len();
            card
        })
        .collect();
    // Register the top documents for a summary the browser will ask for separately. Built from
    // the re-ranked head rather than the page the user is on: a summary of page 7 is not what
    // anyone means by "summarise these results".
    let summary_token = if state.config.ml.summaries_enabled && !ranked.is_empty() && page == 1 {
        let top: Vec<&Value> = ranked
            .iter()
            .take(xustive_ml::prompt::MAX_PASSAGES)
            .map(|r| &r.hit)
            .collect();
        let passages = crate::summary::passages_from_hits(&top, xustive_ml::prompt::MAX_PASSAGES);
        (!passages.is_empty()).then(|| {
            state.pending.insert(
                normalized.clone(),
                xustive_ml::OutputLang::from_detected(language.as_str()),
                passages,
            )
        })
    } else {
        None
    };

    let total_hits = hits.estimated_total_hits;
    let total_pages = total_hits.div_ceil(hits_per_page).min(100);

    if results.is_empty() {
        state.metrics.incr(
            metrics::SEARCH_ZERO,
            metrics::SEARCH_ZERO_HELP,
            &[("lang", language.as_str())],
        );
    }
    state.metrics.incr(
        metrics::SEARCH_RESULTS,
        metrics::SEARCH_RESULTS_HELP,
        &[
            ("lang", language.as_str()),
            ("bucket", count_bucket(total_hits)),
        ],
    );

    // NOTE: `normalized` is echoed to the caller who sent it, which is fine. It must never
    // reach a log line, a metric label, or a trace attribute.
    Ok(Json(SearchResponse {
        query: QueryInfo {
            raw,
            normalized,
            language: language.as_str(),
            language_confidence: confidence,
            expanded_terms,
            corrected: None,
        },
        summary_token,
        instant,
        pagination: Pagination {
            page,
            hits_per_page,
            total_hits,
            total_pages,
            estimated: true,
        },
        took_ms: started.elapsed().as_millis() as u64,
        results,
        facets: hits.facet_distribution,
    }))
}

/// Detect the query language, honouring an explicit client hint.
///
/// An explicit `lang` from the client wins outright: the user chose it, and second-guessing a
/// stated preference with a heuristic is worse than trusting it.
fn detect_language(state: &AppState, normalized: &str, hint: Option<&str>) -> Detection {
    if let Some(h) = hint.filter(|h| *h != "auto") {
        if let Some(lang) = Lang::parse(h) {
            return Detection {
                lang,
                confidence: 1.0,
                script: script::detect(normalized),
                secondary: None,
            };
        }
    }
    state.detector.detect_normalized(normalized)
}

/// Bounded label for the script metric.
fn script_label(s: Script) -> &'static str {
    match s {
        Script::Arabic => "arabic",
        Script::Latin => "latin",
        Script::Mixed => "mixed",
        Script::Unknown => "unknown",
    }
}

fn parse_filters(p: &SearchParams) -> Result<Filters, ApiError> {
    let mut f = Filters {
        exclude_spam: true,
        ..Default::default()
    };

    if let Some(s) = &p.source {
        for part in s.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let st = SourceType::parse(part).ok_or_else(|| ApiError::InvalidParam {
                param: "source",
                detail: format!("unknown source {part:?}"),
            })?;
            f.source_types.push(st);
        }
    }
    if let Some(s) = &p.sentiment {
        for part in s.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let sl = SentimentLabel::parse(part).ok_or_else(|| ApiError::InvalidParam {
                param: "sentiment",
                detail: format!("unknown sentiment {part:?}"),
            })?;
            f.sentiments.push(sl);
        }
    }
    // An explicit `lang` both overrides detection and restricts results.
    //
    // It served only as a detection hint until the facet UI shipped, at which point clicking
    // "French" changed how the query was interpreted and left the result set untouched — a
    // control that appears to act and does not. Someone who says French wants French.
    if let Some(s) = &p.lang {
        for part in s.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            f.languages.push(part.to_ascii_lowercase());
        }
    }

    f.published_from = p.from;
    f.published_to = p.to;
    Ok(f)
}

/// Map a raw index hit to a result card.
///
/// Highlighted text comes from Meilisearch's `_formatted` object, which contains `<em>` markers.
/// Everything else is escaped by the client; `<em>` is the only markup it may render.
fn to_card(hit: &Value) -> ResultCard {
    let formatted = hit.get("_formatted").unwrap_or(hit);
    let s = |v: &Value, k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };

    let url = s(hit, "url");
    let sentiment = hit.get("sentiment").and_then(|snt| {
        let label = snt.get("label")?.as_str()?;
        let confidence = snt.get("confidence").and_then(Value::as_f64).unwrap_or(0.0);
        // Below the confidence floor the UI shows no badge at all — absence is more honest
        // than a shrug.
        if confidence < 0.35 {
            return None;
        }
        Some(SentimentOut {
            label: SentimentLabel::parse(label)
                .map(|l| l.as_str())
                .unwrap_or("neutral"),
            score: snt.get("score").and_then(Value::as_f64).unwrap_or(0.0) as f32,
        })
    });

    ResultCard {
        id: s(hit, "id"),
        title: s(formatted, "title"),
        display_url: display_url(&url),
        url,
        excerpt: s(formatted, "excerpt"),
        source_type: s(hit, "source_type"),
        source_name: s(hit, "source_id"),
        author: hit.get("author").cloned().unwrap_or(Value::Null),
        published_at: hit.get("published_at").and_then(Value::as_i64).unwrap_or(0),
        published_at_precision: hit
            .get("published_at_precision")
            .and_then(Value::as_str)
            .unwrap_or(match DatePrecision::Unknown {
                DatePrecision::Unknown => "unknown",
                _ => "unknown",
            })
            .to_string(),
        sentiment,
        engagement: hit.get("engagement").cloned().unwrap_or(Value::Null),
        language: s(hit, "language"),
        thumbnail_url: hit
            .get("media")
            .and_then(Value::as_array)
            .and_then(|m| m.first())
            .and_then(|m| m.get("thumb_url").or_else(|| m.get("url")))
            .and_then(Value::as_str)
            .map(str::to_string),
        matched_comments: Vec::new(),
        score: 0.0,
        similar_count: 0,
    }
}

/// Breadcrumb-style URL for display: `elkhabar.com › economie`.
fn display_url(url: &str) -> String {
    let Ok(parsed) = url::Url::parse(url) else {
        return url.to_string();
    };
    let host = parsed
        .host_str()
        .unwrap_or_default()
        .trim_start_matches("www.");
    // Percent-decoded. Algerian sites put the Arabic headline in the slug, so a raw path segment
    // renders as forty characters of %D8%A7%D9%84 — which filled the widest line on every result
    // card with something no reader can use.
    let segments: Vec<String> = parsed
        .path_segments()
        .map(|s| {
            s.filter(|p| !p.is_empty())
                .take(2)
                .map(percent_decode)
                .collect()
        })
        .unwrap_or_default();

    if segments.is_empty() {
        return host.to_string();
    }
    // A decoded Arabic slug is still a whole headline. The breadcrumb is orientation, not
    // content — the title directly beneath it already says what the page is.
    let crumbs: Vec<String> = segments.iter().map(|s| truncate_chars(s, 28)).collect();
    format!("{host} › {}", crumbs.join(" › "))
}

/// Decode `%XX` escapes, leaving anything malformed as it was.
///
/// Hand-rolled rather than adding a dependency: this decodes one path segment for display, and
/// invalid input must survive rather than panic — a URL we cannot decode is still a URL we have
/// to render.
fn percent_decode(segment: &str) -> String {
    let bytes = segment.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        // Hyphens and underscores are word separators in a slug, and reading them as such is
        // the difference between a breadcrumb and a filename.
        out.push(if bytes[i] == b'-' || bytes[i] == b'_' {
            b' '
        } else {
            bytes[i]
        });
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| segment.to_string())
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

/// Bucket a result count for metrics. Keeps label cardinality bounded.
fn count_bucket(n: usize) -> &'static str {
    match n {
        0 => "0",
        1..=10 => "1-10",
        11..=100 => "11-100",
        101..=1000 => "101-1000",
        _ => "1000+",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_arabic_slug_is_decoded_rather_than_shown_as_percent_escapes() {
        // Algerian sites put the headline in the slug. Raw, it renders as forty characters of
        // %D8%A7%D9%84 and fills the widest line on the card with something unreadable.
        let out = display_url(
            "https://www.elkhabar.com/nation/%D8%A7%D9%84%D8%AC%D8%B2%D8%A7%D8%A6%D8%B1-273644",
        );
        assert!(out.contains("الجزائر"), "got {out}");
        assert!(!out.contains('%'), "got {out}");
    }

    #[test]
    fn a_long_slug_is_truncated_to_a_breadcrumb() {
        let long = "a".repeat(200);
        let out = display_url(&format!("https://example.dz/news/{long}"));
        assert!(
            out.chars().count() < 80,
            "got {} chars",
            out.chars().count()
        );
        assert!(out.ends_with('…'));
    }

    #[test]
    fn a_malformed_escape_survives_rather_than_panicking() {
        // A URL we cannot decode is still a URL we have to render.
        assert_eq!(percent_decode("%ZZ"), "%ZZ");
        assert_eq!(percent_decode("%"), "%");
        assert_eq!(percent_decode("a%"), "a%");
    }

    #[test]
    fn slug_separators_read_as_words() {
        assert_eq!(percent_decode("prix-du-gaz"), "prix du gaz");
    }

    #[test]
    fn display_url_is_a_breadcrumb() {
        assert_eq!(
            display_url("https://www.elkhabar.com/economie/article/123"),
            "elkhabar.com › economie › article"
        );
        assert_eq!(display_url("https://example.dz/"), "example.dz");
        assert_eq!(display_url("not a url"), "not a url");
    }

    #[test]
    fn count_buckets_are_bounded() {
        assert_eq!(count_bucket(0), "0");
        assert_eq!(count_bucket(5), "1-10");
        assert_eq!(count_bucket(50), "11-100");
        assert_eq!(count_bucket(5_000), "1000+");
    }

    #[test]
    fn script_labels_are_bounded() {
        // Metric label cardinality must stay fixed.
        for s in [
            Script::Arabic,
            Script::Latin,
            Script::Mixed,
            Script::Unknown,
        ] {
            assert!(!script_label(s).is_empty());
        }
    }

    #[test]
    fn filters_parse_csv_lists() {
        let p = SearchParams {
            q: Some("x".into()),
            page: None,
            hits_per_page: None,
            lang: None,
            source: Some("web, facebook".into()),
            sentiment: Some("negative".into()),
            from: None,
            to: None,
            sort: None,
        };
        let f = parse_filters(&p).unwrap();
        assert_eq!(f.source_types, vec![SourceType::Web, SourceType::Facebook]);
        assert_eq!(f.sentiments, vec![SentimentLabel::Negative]);
        assert!(f.exclude_spam, "spam suppression should be on by default");
    }

    #[test]
    fn unknown_facet_value_is_a_client_error() {
        let p = SearchParams {
            q: Some("x".into()),
            page: None,
            hits_per_page: None,
            lang: None,
            source: Some("myspace".into()),
            sentiment: None,
            from: None,
            to: None,
            sort: None,
        };
        let err = parse_filters(&p).unwrap_err();
        assert_eq!(err.code(), "invalid_filter");
    }

    #[test]
    fn card_prefers_highlighted_fields() {
        let hit = json!({
            "id": "1", "url": "https://example.dz/a",
            "title": "plain", "excerpt": "plain excerpt",
            "_formatted": { "title": "high<em>light</em>", "excerpt": "ex<em>cerpt</em>" }
        });
        let card = to_card(&hit);
        assert_eq!(card.title, "high<em>light</em>");
        assert_eq!(card.excerpt, "ex<em>cerpt</em>");
    }

    #[test]
    fn low_confidence_sentiment_shows_no_badge() {
        let hit = json!({
            "id": "1", "url": "https://example.dz/a",
            "sentiment": { "label": "negative", "score": -0.4, "confidence": 0.1 }
        });
        assert!(
            to_card(&hit).sentiment.is_none(),
            "low-confidence sentiment must be omitted"
        );
    }

    #[test]
    fn confident_sentiment_is_surfaced() {
        let hit = json!({
            "id": "1", "url": "https://example.dz/a",
            "sentiment": { "label": "negative", "score": -0.4, "confidence": 0.8 }
        });
        let s = to_card(&hit).sentiment.expect("should be present");
        assert_eq!(s.label, "negative");
    }

    #[test]
    fn missing_fields_do_not_panic() {
        let card = to_card(&json!({}));
        assert_eq!(card.id, "");
        assert_eq!(card.published_at, 0);
        assert!(card.sentiment.is_none());
        assert!(card.thumbnail_url.is_none());
    }

    #[test]
    fn thumbnail_falls_back_to_media_url() {
        let hit = json!({ "media": [ { "url": "https://cdn.dz/x.jpg" } ] });
        assert_eq!(
            to_card(&hit).thumbnail_url.as_deref(),
            Some("https://cdn.dz/x.jpg")
        );

        let hit = json!({ "media": [ { "url": "https://cdn.dz/x.jpg", "thumb_url": "https://cdn.dz/t.jpg" } ] });
        assert_eq!(
            to_card(&hit).thumbnail_url.as_deref(),
            Some("https://cdn.dz/t.jpg")
        );
    }
}
