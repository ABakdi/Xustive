//! `GET /api/v1/search`.
//!
//! M0 scope: normalise, filter, retrieve, shape. Language detection is script-based and there is
//! no query expansion, no re-ranking and no summary yet — those arrive in M1. The response
//! *shape* is already the final one so the UI does not have to change underneath.

use std::time::Instant;

use axum::extract::{Query as AxumQuery, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use xustive_core::{DatePrecision, Lang, SentimentLabel, SourceType};
use xustive_search::{filter::Filters, Query};
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
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub query: QueryInfo,
    /// `None` until the summariser exists. The client hides the block when it is absent.
    pub summary_token: Option<String>,
    pub pagination: Pagination,
    pub took_ms: u64,
    pub results: Vec<ResultCard>,
    pub facets: Value,
}

pub async fn handler(
    State(state): State<AppState>,
    AxumQuery(params): AxumQuery<SearchParams>,
) -> Result<Json<SearchResponse>, ApiError> {
    let started = Instant::now();

    // --- validate -------------------------------------------------------------------
    let raw = params.q.clone().unwrap_or_default();
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
    let (language, confidence) = detect_language(&normalized, params.lang.as_deref());

    // --- retrieve -------------------------------------------------------------------
    let offset = (page - 1) * hits_per_page;
    let mut query = Query::new(&normalized)
        .limit(hits_per_page)
        .offset(offset)
        .facets(&["source_type", "sentiment.label", "language"])
        .highlight(&["excerpt", "title"]);
    if let Some(expr) = filters.to_expression(SPAM_THRESHOLD) {
        query = query.filter(expr);
    }
    if !sort.is_empty() {
        query = query.sort(&sort);
    }

    let retrieval_started = Instant::now();
    let hits = state
        .search
        .search::<Value>(&cfg.documents_index, &query)
        .await
        .map_err(ApiError::from)?;
    state.metrics.observe(
        metrics::SEARCH_DURATION,
        metrics::SEARCH_DURATION_HELP,
        &[("stage", "retrieve")],
        retrieval_started.elapsed().as_secs_f64(),
    );

    // --- shape ----------------------------------------------------------------------
    let results: Vec<ResultCard> = hits.hits.iter().map(to_card).collect();
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
            expanded_terms: Vec::new(),
            corrected: None,
        },
        summary_token: None,
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

/// Placeholder language detection: script only.
///
/// M1 replaces this with the real cascade (lingua + Darija markers). Until then `Und` is the
/// honest answer for anything ambiguous, and `Und` is safe — retrieval widens rather than
/// narrowing wrongly.
fn detect_language(normalized: &str, hint: Option<&str>) -> (Lang, f32) {
    if let Some(h) = hint {
        if h != "auto" {
            if let Some(l) = Lang::parse(h) {
                return (l, 1.0);
            }
        }
    }
    match script::detect(normalized) {
        Script::Arabic => (Lang::Ar, 0.5),
        Script::Latin => (Lang::Und, 0.0),
        Script::Mixed => (Lang::Mixed, 0.4),
        Script::Unknown => (Lang::Und, 0.0),
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
    let segments: Vec<&str> = parsed
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).take(2).collect())
        .unwrap_or_default();
    if segments.is_empty() {
        host.to_string()
    } else {
        format!("{host} › {}", segments.join(" › "))
    }
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
    fn language_hint_overrides_detection() {
        let (l, c) = detect_language("wach rak", Some("ary"));
        assert_eq!(l, Lang::Ary);
        assert_eq!(c, 1.0);
    }

    #[test]
    fn auto_hint_falls_through_to_detection() {
        let (l, _) = detect_language("الجزائر", Some("auto"));
        assert_eq!(l, Lang::Ar);
    }

    #[test]
    fn ambiguous_latin_is_undetermined_not_guessed() {
        // Guessing `fr` on an Arabizi query would narrow retrieval and return nothing.
        let (l, _) = detect_language("wach rak", None);
        assert_eq!(l, Lang::Und);
    }

    #[test]
    fn empty_query_is_undetermined() {
        assert_eq!(detect_language("", None).0, Lang::Und);
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
