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
use std::time::Duration;

use crate::deadline::{Deadline, Stage};
use crate::state::AppState;

/// Hard cap on query length, matching the API contract.
pub const MAX_QUERY_CHARS: usize = 512;
/// Spam suppression threshold, shared with the eval harness (BUG-003).
use xustive_search::filter::SPAM_THRESHOLD;

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
    /// Search vertical: `all` (default) or `news`. A vertical is a saved filter over the one index —
    /// `news` is web documents that carry a real publication date. Shareable in the URL (`?v=news`).
    #[serde(default)]
    pub v: Option<String>,
    /// Interface language, for tool labels. Distinct from `lang`, which filters results —
    /// someone reading a French interface searching in Darija is the normal case.
    #[serde(default)]
    pub ui: Option<String>,
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
    /// True when this result came from live federation (SearXNG), not yet the local index (M7). Shown
    /// with a "web" badge; it is indexed in the background under the same URL-derived id, so a later
    /// search returns it as a normal local result and this flag disappears.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub from_web: bool,
    /// The media an article carries, for the Images and Videos tabs (M9-T01.6). Capped so one
    /// gallery article cannot fill a whole grid; empty for ordinary results.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media: Vec<MediaOut>,
}

/// One image or video on a result, as the tile needs it. The upstream URL is never handed to
/// the browser as-is — the web tier signs and proxies it ([[ADR-0021]]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaOut {
    pub kind: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub width: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub height: u32,
}

fn is_zero_u32(n: &u32) -> bool {
    *n == 0
}

/// Time kept back from the federation strip wait so the page can still be shaped and sent inside
/// the search deadline after the wait ends. Shaping is a few milliseconds; the margin covers a
/// slow machine, not a slow tool.
const FEDERATION_SHAPE_MARGIN: Duration = Duration::from_millis(150);

/// How many images one page may contribute to a grid. Three keeps a photo gallery from being the
/// whole first screen while still showing that a page has several.
const MEDIA_PER_CARD: usize = 3;

/// The media entries worth putting on the wire.
///
/// Tracking pixels and layout spacers were already dropped at parse time; what is left here is
/// the `og:image` and the article figures. Videos are all kept — a page rarely has more than one.
fn media_of(hit: &Value) -> Vec<MediaOut> {
    let Some(items) = hit.get("media").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut images = 0usize;
    items
        .iter()
        .filter_map(|m| {
            let kind = m.get("type")?.as_str()?;
            let url = m.get("url")?.as_str()?;
            if kind == "image" {
                images += 1;
                if images > MEDIA_PER_CARD {
                    return None;
                }
            }
            Some(MediaOut {
                kind: kind.to_string(),
                url: url.to_string(),
                thumb_url: m
                    .get("thumb_url")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                provider: m
                    .get("provider")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                width: m.get("width").and_then(Value::as_u64).unwrap_or(0) as u32,
                height: m.get("height").and_then(Value::as_u64).unwrap_or(0) as u32,
            })
        })
        .collect()
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
    /// Opaque token the click beacon returns so a click can be attributed to this query without the
    /// query text (M6-T03). `None` when interaction signals are off. Never logged — `token` is a
    /// forbidden telemetry field name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interaction_token: Option<String>,
    /// True when the query reads as a question rather than a topic.
    ///
    /// The client uses this to decide *where* the summary goes, not whether to fetch it. Someone
    /// typing a topic wants a list of pages; someone asking a question wants an answer, and ten
    /// blue links above it makes them do the work themselves.
    #[serde(default)]
    pub is_question: bool,
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
    /// True when facets were dropped under time pressure rather than genuinely empty. Lets the UI
    /// say filtering is temporarily unavailable instead of silently showing no filters, which
    /// reads as "this search has nothing to filter by" — a different, wrong message.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub facets_degraded: bool,
    /// Related searches (M7-T03): concepts (entities/topics) that recur across this query's top
    /// results — "people searching this find pages about …". Empty past the first page or when the
    /// results share no recurring concept beyond the query itself.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
}

/// Below this many primary hits, try the expanded leg.
///
/// Not zero. A query that returns three results is not obviously fine — for Arabizi it usually
/// means a couple of incidental Latin-script matches while the Arabic documents that actually
/// answer it were never reached.
const EXPANSION_THRESHOLD: usize = 5;

// The weak-top-score expansion trigger (M7-T01.3) and the all-stop-word phrase rescue (M7-T01.5)
// live in `xustive-search` (`rank::top_result_is_weak`, `settings::is_all_stop_words`), shared with
// the offline eval harness so the harness scores the same retrieval production runs (BUG-003).
use xustive_search::rank::top_result_is_weak;
use xustive_search::settings::is_all_stop_words;

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

    // Same highlights as the primary leg (BUG-022): expansion-only hits otherwise rendered with no
    // <em> marks while primary-leg cards on the same page had them.
    let mut expanded = Query::new(terms.join(" "))
        .limit(state.config.search.candidate_pool)
        .highlight(&["excerpt", "title"]);
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
    // The connection address, used for one thing only: turning "weather" with no place in it into
    // a wilaya, by an in-process lookup that discards the address immediately ([[ADR-0020]]).
    crate::admin::Peer(peer): crate::admin::Peer,
    AxumQuery(params): AxumQuery<SearchParams>,
) -> Result<Json<SearchResponse>, ApiError> {
    let started = Instant::now();

    // --- validate -------------------------------------------------------------------
    let raw = params.q.clone().unwrap_or_default();
    // Computed before `raw` is consumed downstream, and on the raw query rather than the
    // normalised one — normalisation folds the question mark, which is the single clearest signal
    // a reader can give.
    let asked_a_question = xustive_lang::is_question(&raw);
    // Computed from the raw query before it is consumed by the response. Normalisation folds the
    // characters an expression is made of, so `45*1.19` has to be seen intact to parse.
    // Rendered in the interface language the caller asked for, so an Arabic reader gets
    // "2 قنطار → كيلوغرام" rather than the English unit names.
    let ui_lang = params.ui.as_deref().unwrap_or("en");
    // Pure matchers first — they answer in microseconds and cover most tools.
    //
    // Weather is separate because its data lives in a cache, and a matcher that reached for
    // Redis would put a round trip on every search that is not about weather. Only consulted
    // when nothing pure matched.
    let instant = match xustive_tools::best_in(&raw, ui_lang) {
        Some(answer) => Some(answer),
        // Weather and currency both live in the tool cache, so they are consulted only when no
        // pure matcher answered — otherwise every search that is not about either would pay a
        // Redis round trip. Currency first: `20 eur dzd` names no weather word, and the two
        // cannot both match.
        None => match crate::currency::answer(&state, &raw, ui_lang).await {
            Some(a) => Some(a),
            None => crate::weather::answer(&state, &raw, ui_lang, peer).await,
        },
    };
    if let Some(a) = &instant {
        state.metrics.incr(
            crate::metrics::INSTANT_ANSWERS,
            crate::metrics::INSTANT_ANSWERS_HELP,
            &[("tool", a.tool)],
        );
    }

    // The budget starts here, once, and is absolute. Passing a duration down the chain would let
    // every stage believe it had the whole thing.
    let deadline = Deadline::new(Duration::from_millis(state.config.api.timeout_search_ms));

    // Operators come off the raw query, before normalisation. Normalisation folds quotes and
    // punctuation, so a phrase parsed afterwards is no longer a phrase.
    let operators = xustive_search::parse_operators(&raw);
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

    let mut filters = parse_filters(&params)?.normalise();
    // `site:` narrows to a domain. Taken from the operator rather than a query parameter, so the
    // URL a user shares carries exactly what they typed.
    if operators.site.is_some() {
        filters.domain = operators.site.clone();
    }
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
    let normalized = xustive_text::normalize(&operators.engine_query());
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

    // --- federation (M7-T05): borrow recall from the web, off the response's critical path --------
    // A real metasearch aggregation takes seconds, which the response cannot wait on. So the fetch
    // runs **detached** with a generous budget and, on results, eager-indexes them and feeds the
    // crawler — so they become real results within seconds and the next search finds them. The
    // response then waits only briefly (`budget_ms`) for a best-effort "from the web" strip; if the
    // fetch has not answered by then, the task keeps running and indexes it anyway. Only on page 1,
    // only when the runtime switch is on and a gateway client exists.
    let federation = if page == 1
        && state
            .federation_enabled
            .load(std::sync::atomic::Ordering::Relaxed)
    {
        state.federator.clone().map(|client| {
            let q = normalized.clone();
            let bg = state.clone();
            let fetch_budget = state.config.federation.fetch_budget_ms;
            // The Images and Videos tabs federate in their own category (M9-T06): the same
            // gateway, budget and fail-open, asking SearXNG's image or video engines instead.
            let category = xustive_ingest::federation::Category::from_vertical(params.v.as_deref());
            tokio::spawn(async move {
                let fetch_started = Instant::now();
                let hits = client.federate_in(&q, category, Some(fetch_budget)).await;
                // Per-tool latency (M7-T09.2). Detached, so this measures the tool, not the search.
                bg.metrics.observe(
                    metrics::FEDERATION_DURATION,
                    metrics::FEDERATION_DURATION_HELP,
                    &[],
                    fetch_started.elapsed().as_secs_f64(),
                );
                bg.metrics.incr(
                    metrics::FEDERATION_SEARCHES,
                    metrics::FEDERATION_SEARCHES_HELP,
                    &[("outcome", if hits.is_empty() { "empty" } else { "hits" })],
                );
                if !hits.is_empty() {
                    bg.metrics.incr_by(
                        metrics::FEDERATION_FED,
                        metrics::FEDERATION_FED_HELP,
                        &[],
                        hits.len() as u64,
                    );
                    // Index (eager) + queue for full crawl. Runs regardless of the strip timeout.
                    ingest_federated(&bg, &hits);
                }
                hits
            })
        })
    } else {
        None
    };

    // --- retrieve -------------------------------------------------------------------
    // Pull a candidate pool rather than one page: re-ranking can only reorder what it is
    // given, so paging at the engine would freeze the engine's ordering into the result.
    let offset = (page - 1) * hits_per_page;
    // The pool must reach past the requested page: paging happens *after* re-ranking, by skipping
    // into the ranked pool, so a pool that stops at `candidate_pool` made every page past
    // `candidate_pool / hits_per_page` silently empty while the pagination advertised it
    // (BUG-002). Deep pages fetch a deeper pool, bounded by the engine's own maxTotalHits.
    let pool = cfg
        .candidate_pool
        .max(offset + hits_per_page)
        .min(xustive_search::settings::MAX_TOTAL_HITS);
    // Facets are given up before re-ranking: losing the filter chips costs less than losing
    // result quality.
    let want_facets = deadline.allows(Stage::Facets);
    let mut query = Query::new(&normalized)
        .limit(pool)
        .offset(0)
        .ranking_score(true)
        .highlight(&["excerpt", "title"]);
    if want_facets {
        query = query.facets(&["source_type", "sentiment.label", "language"]);
    }
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

    // --- short-query stop-word guard (M7-T01.5) -------------------------------------------
    // A query made entirely of stop words ("the and", "من في") is stripped to nothing by the
    // tokeniser, so the primary leg returns zero hits — not because the corpus lacks those words
    // but because the engine threw them away. Meilisearch keeps stop words *inside a phrase*, so we
    // re-issue the query quoted. It matches the exact run of function words where it occurs, which
    // is the only sensible reading of such a query, instead of an empty page.
    if hits.hits.is_empty() && is_all_stop_words(&normalized) {
        let phrase = format!("\"{normalized}\"");
        let mut retry = Query::new(&phrase)
            .limit(pool)
            .offset(0)
            .ranking_score(true)
            .highlight(&["excerpt", "title"]);
        if want_facets {
            retry = retry.facets(&["source_type", "sentiment.label", "language"]);
        }
        if let Some(expr) = filters.to_expression(SPAM_THRESHOLD) {
            retry = retry.filter(expr);
        }
        if !sort.is_empty() {
            retry = retry.sort(&sort);
        }
        if let Ok(recovered) = state.search.search::<Value>(&index, &retry).await {
            hits = recovered;
        }
    }

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
    // The weak-score half of the trigger reads the *first engine hit*, which is only the best
    // match in relevance order — under an explicit sort it is merely the newest (or whatever the
    // sort puts first), whose score says nothing about match quality, and the leg fired on nearly
    // every recency-sorted search (BUG-013). The count half stays: too few results is too few
    // results in any order.
    let few_or_weak = hits.hits.len() < EXPANSION_THRESHOLD
        || (sort.is_empty() && top_result_is_weak(&hits.hits));
    let expanded_terms = if few_or_weak && deadline.allows(Stage::Expansion) {
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

    // --- semantic (dense) recall + fusion (M7-T02) ----------------------------------
    // Embed the query, k-NN the text collection, and fuse those candidates with the lexical ones by
    // reciprocal-rank fusion — so a query worded differently from a document can still reach it.
    // Fail-open: no engine, or any error, leaves `hits.hits` exactly as lexical retrieval produced
    // it. Runs before the interaction and re-rank stages so they see the fused candidate set.
    //
    // Deadline-gated (M7-T02.4): the dense leg costs a query embed + a Qdrant round-trip, so under
    // time pressure it degrades like the expansion leg — dropped rather than blowing the budget, and
    // the search still returns its lexical results.
    if state.text_search.is_some() && !deadline.allows(Stage::Expansion) {
        state.metrics.incr(
            metrics::DEGRADED,
            metrics::DEGRADED_HELP,
            &[("stage", "semantic")],
        );
    }
    if let Some(dense) = state
        .text_search
        .as_ref()
        .filter(|_| deadline.allows(Stage::Expansion))
    {
        let dense_ids = dense.candidates(&normalized).await;
        if !dense_ids.is_empty() {
            let lex_ids: std::collections::HashSet<&str> = hits
                .hits
                .iter()
                .filter_map(|h| h.get("id").and_then(Value::as_str))
                .collect();
            let missing: Vec<String> = dense_ids
                .iter()
                .filter(|id| !lex_ids.contains(id.as_str()))
                .cloned()
                .collect();
            let dense_docs = fetch_by_ids(&state, &index, &missing).await;
            // Three outcomes, not two (BUG-025): a genuine recall attempt whose id-fetch came back
            // empty is a *failure*, and labelling it "reinforce" hid dense-recall outages from the
            // dashboard behind the label that means "everything worked, nothing new to add".
            let kind = if !dense_docs.is_empty() {
                "recall"
            } else if missing.is_empty() {
                "reinforce"
            } else {
                "fetch_failed"
            };
            state.metrics.incr(
                metrics::SEMANTIC_FUSED,
                metrics::SEMANTIC_FUSED_HELP,
                &[("kind", kind)],
            );
            hits.hits = crate::text_search::rrf_fuse(
                std::mem::take(&mut hits.hits),
                &dense_ids,
                dense_docs,
                pool,
            );
        }
    }

    // --- re-rank --------------------------------------------------------------------
    let rerank_started = Instant::now();
    let trust = state.trust_tiers.as_ref();
    // Under real pressure the engine's own order is returned instead. Worse results, but
    // results — and the user cannot tell a search that returns nothing from an outage.
    if !deadline.allows(Stage::Rerank) {
        state.metrics.incr(
            metrics::DEGRADED,
            metrics::DEGRADED_HELP,
            &[("stage", Stage::Rerank.as_str())],
        );
    }
    // Anonymous CTR over the candidate ids, if interaction signals are on (M6-T04). Read before the
    // re-rank so it can nudge ordering; empty when disabled, below the k-floor, or Redis is down —
    // in which case rerank sees no interaction data and behaves exactly as before.
    let interaction_of = match state.interactions() {
        Some(store) => {
            let ids: Vec<String> = hits
                .hits
                .iter()
                .filter_map(|h| h.get("id").and_then(Value::as_str).map(str::to_string))
                .collect();
            store.ctr_for(&normalized, &ids).await
        }
        None => std::collections::HashMap::new(),
    };
    let ranked = rank::rerank(
        &hits.hits,
        &normalized,
        xustive_core::now_unix(),
        trust,
        state.authority.as_ref(),
        &interaction_of,
        &state.ranking,
        Some(ui_lang),
    );
    state.metrics.observe(
        metrics::SEARCH_DURATION,
        metrics::SEARCH_DURATION_HELP,
        &[("stage", "rerank")],
        rerank_started.elapsed().as_secs_f64(),
    );

    // Best-effort live federation hits (M7): wait up to the strip budget for the detached fetch, to
    // MIX into the page below. The fetch keeps running and eager-indexes regardless, so whatever is
    // not ready in time still becomes a normal local result on the next search.
    let federation_armed = federation.is_some();
    let federated_hits = match federation {
        Some(mut handle) => {
            // Bounded by the search's own deadline, with room left to shape the page. The strip
            // budget is a cap, not an entitlement: with `budget_ms` equal to `timeout_search_ms`
            // — which the dev config had — a slow SearXNG made every search a 504, on every
            // vertical, and nobody saw it because federation was off (found by M9-T06).
            let cap = Duration::from_millis(state.config.federation.budget_ms);
            let wait = deadline
                .budget_for(cap)
                .saturating_sub(FEDERATION_SHAPE_MARGIN);
            if wait.is_zero() {
                Vec::new()
            } else {
                match tokio::time::timeout(wait, &mut handle).await {
                    Ok(Ok(hits)) => hits,
                    _ => Vec::new(),
                }
            }
        }
        None => Vec::new(),
    };

    // --- shape ----------------------------------------------------------------------
    // `-term` is applied here rather than in the engine query: Meilisearch has no negation in
    // its query syntax, and expressing it as a filter would need every excluded word to be a
    // filterable attribute.
    let excluded: Vec<String> = operators
        .excluded
        .iter()
        .map(|t| xustive_text::fold(t))
        .filter(|t| !t.is_empty())
        .collect();
    let pool_before = ranked.len();
    let ranked: Vec<_> = if excluded.is_empty() {
        ranked
    } else {
        ranked
            .into_iter()
            .filter(|r| {
                let haystack = xustive_text::fold(&format!(
                    "{} {}",
                    r.hit
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    r.hit
                        .get("body")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                ));
                !excluded.iter().any(|term| haystack.contains(term))
            })
            .collect()
    };

    let mut results: Vec<ResultCard> = ranked
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

    // Mix live web results into the page (M7): federated hits that are not already a local result,
    // each flagged `from_web`. Deduped by the URL-derived id — the same id the eager index uses — so
    // once a URL has been crawled it appears as a normal local result and its web card drops out. The
    // ids also flow into impression/click capture below, so interaction ranking (M6) covers them too.
    if page == 1 && !federated_hits.is_empty() {
        merge_federated(&mut results, &federated_hits);
    }
    // The convergence measure (M7-T09.2): of the cards served on federation-armed first pages, how
    // many came from the web vs the local index. Counted on *every* armed page-1 search — including
    // the ones where no web card arrived — because the falling web share over time is exactly the
    // index catching up, and skipping the all-local pages would hide that.
    if federation_armed {
        let web = results.iter().filter(|c| c.from_web).count() as u64;
        let local = results.len() as u64 - web;
        state.metrics.incr_by(
            metrics::FEDERATION_BLEND,
            metrics::FEDERATION_BLEND_HELP,
            &[("source", "web")],
            web,
        );
        state.metrics.incr_by(
            metrics::FEDERATION_BLEND,
            metrics::FEDERATION_BLEND_HELP,
            &[("source", "local")],
            local,
        );
    }
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
                // The reader's language, not the query's: the nav-bar choice is what they want
                // to read, whatever language the pages it cites were written in.
                xustive_ml::OutputLang::from_ui(ui_lang),
                passages,
            )
        })
    } else {
        None
    };

    // The result count this search returned — computed here so the interaction capture below can
    // record it (M7-T10). Exclusions are applied to the candidate pool, not the whole corpus, so the
    // engine's count no longer describes what the user is shown; scale it by the observed survival
    // rate and mark it an estimate. Reporting the unfiltered 395 while showing a filtered list is
    // simply false, and the user has no way to tell.
    let (total_hits, estimated) = if excluded.is_empty() || pool_before == 0 {
        (hits.estimated_total_hits, hits.estimated_total_hits > 0)
    } else {
        let survival = ranked.len() as f64 / pool_before as f64;
        (
            (hits.estimated_total_hits as f64 * survival).round() as usize,
            true,
        )
    };
    // Advertise only pages that can actually be served: the UI's own cap of 100, and the engine's
    // deep-pagination bound — a pool can never exceed maxTotalHits, so a page needing an offset
    // past it would always come back empty (BUG-002). Honest beats impressive.
    let servable_pages = xustive_search::settings::MAX_TOTAL_HITS / hits_per_page;
    let total_pages = total_hits
        .div_ceil(hits_per_page)
        .min(100)
        .min(servable_pages.max(1));

    // Anonymous interaction capture (M6-T02/T03 + M7-T10 search history), best-effort, gated on the
    // store. No client call and no new egress — the serving plane records straight into Redis.
    // Records: an impression for each shown document; the query with its **result count** and coarse
    // category (k-anonymously); and an opaque token the click beacon returns so a click can be
    // attributed to this query WITHOUT the query text ever being in the click request.
    let interaction_token = if let Some(store) = state.interactions() {
        let page_ids: Vec<String> = results.iter().map(|c| c.id.clone()).collect();
        if !page_ids.is_empty() {
            store.impressions(&normalized, &page_ids).await;
            // Note each shown document's URL so the crawler's hot-doc re-crawl (M6-T06.1) can resolve
            // a clicked doc id back to a URL to revisit. A local (non-web) card only — a federated
            // card is not yet an indexed document to re-crawl.
            let urls: Vec<(String, String)> = results
                .iter()
                .filter(|c| !c.from_web && !c.url.is_empty())
                .map(|c| (c.id.clone(), c.url.clone()))
                .collect();
            store.note_urls(&urls).await;
        }
        store
            .query_seen(
                &normalized,
                interaction_category(params.v.as_deref()),
                total_hits as u32,
            )
            .await;
        Some(mint_interaction_token(&state, &store, &normalized))
    } else {
        None
    };

    if results.is_empty() {
        state.metrics.incr(
            metrics::SEARCH_ZERO,
            metrics::SEARCH_ZERO_HELP,
            &[("lang", language.as_str())],
        );
    }

    // Query-driven discovery (M2-T16.4). A search that came up short is a coverage gap worth
    // finding sources for — but recording the query is only permissible under ADR-0008 when it is
    // opt-in and k-anonymous, both of which live in `WeakCoverage`. Off by default: with the flag
    // clear, nothing about this search is recorded anywhere. The normalised query, never the raw
    // one, and never into a log.
    let disc = &state.config.discovery;
    if disc.weak_coverage_enabled && total_hits <= disc.weak_coverage_result_floor {
        if let Some(w) = xustive_ingest::weak_coverage::WeakCoverage::connect_in(
            state.config.queue.signals_url(),
            "discovery",
            disc.effective_k(),
            std::time::Duration::from_secs(disc.weak_coverage_window_days * 86_400),
        ) {
            w.record(&normalized).await;
        }
    }
    state.metrics.incr(
        metrics::SEARCH_RESULTS,
        metrics::SEARCH_RESULTS_HELP,
        &[
            ("lang", language.as_str()),
            ("bucket", count_bucket(total_hits)),
        ],
    );

    // Related searches (M7-T03): the concepts (entities/topics) that recur across this query's top
    // results. A concept common to many of the pages a query surfaces is what that query is "also
    // about" — computed from the results we already have, so it needs no graph or extra round trip.
    let related: Vec<String> = if page == 1 {
        let fq = xustive_text::fold(&normalized);
        let mut counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for r in ranked.iter().take(20) {
            for field in ["entities", "topics"] {
                let Some(arr) = r.hit.get(field).and_then(Value::as_array) else {
                    continue;
                };
                for concept in arr.iter().filter_map(Value::as_str) {
                    let folded = xustive_text::fold(concept);
                    if folded.chars().count() < 3 {
                        continue;
                    }
                    // Drop the query itself and its sub/superstrings — those are not "related".
                    if fq.contains(&folded) || folded.contains(&fq) {
                        continue;
                    }
                    *counts.entry(concept.to_string()).or_default() += 1;
                }
            }
        }
        // A concept must recur across at least two of the top results to count as a theme, not a
        // one-off. Most frequent first, capped; ties broken by name for a stable order.
        let mut themes: Vec<(String, u32)> = counts.into_iter().filter(|(_, n)| *n >= 2).collect();
        themes.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        themes.into_iter().take(8).map(|(c, _)| c).collect()
    } else {
        Vec::new()
    };

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
        interaction_token,
        is_question: asked_a_question,
        instant,
        pagination: Pagination {
            page,
            hits_per_page,
            total_hits,
            total_pages,
            estimated,
        },
        took_ms: started.elapsed().as_millis() as u64,
        results,
        // Never `null`: when facets were skipped (deadline) or the engine sent nothing, an empty
        // object keeps the contract "facets is a map" — a `null` here crashed the results page in
        // exactly the degraded state `facets_degraded` exists to soften (BUG-001).
        facets: if hits.facet_distribution.is_object() {
            hits.facet_distribution
        } else {
            serde_json::json!({})
        },
        // Facets were asked for but the deadline cut them, versus simply absent. Only the first is
        // a degradation worth signalling.
        facets_degraded: !want_facets,
        related,
    }))
}

/// Fetch documents by id from the index, for the semantic fusion leg (M7-T02). Order is whatever the
/// engine returns — the fuser re-orders by fused rank. Fail-open: an index error yields an empty
/// list, so the dense leg simply contributes nothing and search proceeds on the lexical candidates.
async fn fetch_by_ids(state: &AppState, index: &str, ids: &[String]) -> Vec<Value> {
    if ids.is_empty() {
        return Vec::new();
    }
    // Ids are `u-<hex>` or ULIDs — `{:?}` quotes them safely for the filter expression.
    let list = ids
        .iter()
        .map(|id| format!("{id:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let query = Query::new("")
        .filter(format!("id IN [{list}]"))
        .limit(ids.len());
    match state.search.search::<Value>(index, &query).await {
        Ok(h) => h.hits,
        Err(e) => {
            tracing::debug!(error = %e, "semantic fusion: fetching dense documents failed");
            Vec::new()
        }
    }
}

/// Ingest federated hits (M7). Fire-and-forget — the search response never waits on it.
///
/// Two effects, both keyed on the *same* URL-derived id so they converge on one document:
/// 1. **Eager index** (when `federation.eager_index` is on): each hit is indexed *immediately* as a
///    thin document — its SearXNG title and snippet — so it appears as a real result within seconds
///    rather than only after a full crawl. Low `quality_score`, so a placeholder never outranks a
///    real page.
/// 2. **Crawl-feed** (always): the URL is queued for a full-page crawl, front-promoted since a user
///    just asked for it. That crawl produces the same id (`id_for_url`, set in the orchestrator for
///    the federation channel), so it **overwrites** the thin document instead of duplicating it.
///
/// The search plane only *writes* Redis (the index queue and the frontier); the crawler *reads* the
/// frontier ([[ADR-0001 - Two-Plane Architecture]]). Each URL passes the same `SafeUrl` and trap
/// checks a discovered link does.
fn ingest_federated(state: &AppState, hits: &[xustive_ingest::federation::FederatedHit]) {
    let queue_url = state.config.queue.url.clone();
    let index_stream = state.config.queue.index_stream.clone();
    let stream_max_len = state.config.queue.index_stream_max_len;
    // The crawl-feed enforces the same frontier ceilings as the crawler itself (PROB-001) — a
    // federated URL is a discovery like any other and gets no bypass around the bounds.
    let frontier_limits =
        xustive_ingest::frontier::FrontierLimits::from_config(&state.config.crawl);
    let eager = state.config.federation.eager_index;
    let now = xustive_core::now_unix();

    // Canonicalise once, synchronously, so the eager id and the crawl-feed id are identical.
    struct Entry {
        curl: String,
        host: String,
        title: String,
        snippet: String,
        lang: xustive_core::Lang,
        /// The image or video the hit was (M9-T06), stored on the eager document so the Images
        /// and Videos tabs find it locally next time, before the crawl lands.
        media: Vec<xustive_core::model::Media>,
    }
    let mut entries: Vec<Entry> = Vec::new();
    for h in hits {
        let Ok(safe) = xustive_core::SafeUrl::parse(&h.url) else {
            continue;
        };
        let parsed = safe.as_url().clone();
        if xustive_ingest::frontier::detect_trap(&parsed).is_some() {
            continue;
        }
        entries.push(Entry {
            curl: xustive_ingest::frontier::canonical(&parsed),
            host: safe.authority(),
            title: h.title.clone(),
            snippet: h.snippet.clone(),
            // The hit's OWN language, from its title + snippet (BUG-023): stamping the query's
            // detected language mislabelled every cross-language result — a French query indexing
            // an English page put `fr` on it, corrupting the language filter and facet until the
            // full crawl overwrote the thin doc.
            lang: state
                .detector
                .detect(&format!("{} {}", h.title, h.snippet))
                .lang,
            media: h
                .media
                .iter()
                .map(|m| xustive_core::model::Media {
                    kind: if m.kind == "video" {
                        xustive_core::model::MediaKind::Video
                    } else {
                        xustive_core::model::MediaKind::Image
                    },
                    url: m.src.clone(),
                    thumb_url: m.thumb.clone(),
                    width: 0,
                    height: 0,
                    ocr_text: None,
                    ocr_lang: None,
                    embedding_id: None,
                    phash: None,
                    provider: (m.kind == "video").then(|| {
                        xustive_ingest::video::from_url(&m.src)
                            .map(|v| v.provider.as_str().to_string())
                            .unwrap_or_else(|| "web".to_string())
                    }),
                })
                .collect(),
        });
    }
    if entries.is_empty() {
        return;
    }

    tokio::spawn(async move {
        // 1. Eager index — thin documents to the index queue, overwritten later by the full crawl.
        if eager {
            if let Ok(producer) = xustive_queue::Queue::connect_producer(&queue_url, &index_stream)
                .await
                .map(|q| q.with_max_len(stream_max_len))
            {
                let jobs: Vec<xustive_queue::indexer::IndexJob> = entries
                    .iter()
                    .map(|e| {
                        let body = if e.snippet.trim().is_empty() {
                            e.title.clone()
                        } else {
                            e.snippet.clone()
                        };
                        let mut doc = xustive_core::Document::new(
                            xustive_core::id_for_url(&e.curl),
                            e.curl.clone(),
                            xustive_core::SourceType::Web,
                        );
                        doc.title = e.title.clone();
                        doc.body_len = body.split_whitespace().count();
                        doc.excerpt = body.clone();
                        doc.content_hash = xustive_core::hash::content_hash(&body);
                        doc.body = body;
                        doc.discovery = xustive_core::DiscoveryChannel::Federation;
                        doc.source_id = "federation".into();
                        doc.media = e.media.clone();
                        doc.language = e.lang;
                        doc.crawled_at = now;
                        doc.indexed_at = now;
                        // A placeholder from a snippet, not a crawled page — kept low so it never
                        // outranks a real document, and replaced the moment the crawl lands.
                        doc.quality_score = 0.1;
                        // Only the minimum ran — the full crawl owes it the optional enrichment,
                        // which the repass or the crawl itself will do.
                        doc.enrichment_level = xustive_core::EnrichmentLevel::Partial;
                        xustive_queue::indexer::IndexJob {
                            document: serde_json::to_value(&doc).unwrap_or_default(),
                            index: None,
                        }
                    })
                    .collect();
                if let Err(e) = producer.produce_many(&jobs).await {
                    tracing::warn!(error = %e, "eager federation index submit failed");
                }
            }
        }

        // 2. Crawl-feed — full-page crawl, front-promoted, sharing the eager id so it overwrites.
        if let Ok(frontier) = xustive_ingest::frontier::Frontier::connect(&queue_url)
            .map(|f| f.with_limits(frontier_limits.clone()))
        {
            for e in &entries {
                let pending = xustive_ingest::frontier::Pending {
                    url: e.curl.clone(),
                    host: e.host.clone(),
                    source_id: "federation".into(),
                    depth: 0,
                    trust: 40,
                    channel: xustive_core::DiscoveryChannel::Federation,
                    priority: xustive_ingest::frontier::priority_for(0, 40, false),
                };
                if frontier.add(&pending).await.is_ok() {
                    frontier.promote(&e.host, &pending.url).await;
                }
            }
        }
    });
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

    // Verticals are saved filters over the same index, not separate corpora. `news` is web content
    // with a date we actually know (a guessed date is not news). Unknown verticals fall back to All
    // rather than erroring, so a stale `?v=` link still returns results.
    match p.v.as_deref() {
        Some("news") => {
            f.source_types = vec![SourceType::Web];
            f.exclude_unknown_dates = true;
        }
        // Files: documents extracted from a PDF rather than a web page.
        Some("files") => {
            f.content_type = Some("application/pdf".to_string());
        }
        // Images and Videos (M9): pages that carry media of that kind. A tile is a page that has
        // the picture, ranked by the page's relevance and the image's OCR text.
        Some("images") => f.media_kind = Some("image".to_string()),
        Some("videos") => f.media_kind = Some("video".to_string()),
        _ => {}
    }

    Ok(f)
}

/// Map a raw index hit to a result card.
///
/// Highlighted text comes from Meilisearch's `_formatted` object, which contains `<em>` markers.
/// Everything else is escaped by the client; `<em>` is the only markup it may render.
/// The bounded, `&'static` category recorded with a query (M6-T02.2). Keyed on the search vertical,
/// which is already an enumerable set — never free text, so query analytics stay low-cardinality.
fn interaction_category(vertical: Option<&str>) -> &'static str {
    match vertical {
        Some("news") => "news",
        Some("files") => "files",
        _ => "web",
    }
}

/// Mint an opaque search→click token and store it in memory against the query's hash (never the
/// query text). Swept of expired entries on the way in, so the map cannot grow without bound
/// (M6-T03.1). The token is a fresh ULID — it carries no information about the query.
fn mint_interaction_token(
    state: &AppState,
    store: &xustive_ingest::interaction::Interactions,
    normalized_query: &str,
) -> String {
    use std::time::Instant;
    const TTL: std::time::Duration = std::time::Duration::from_secs(120);
    /// Ceiling on outstanding tokens (BUG-024), mirroring `PendingStore::MAX_PENDING`: the TTL
    /// sweep alone left the map bounded only by request rate, so a flood grew it without limit.
    const MAX_TOKENS: usize = 4096;
    let token = ulid::Ulid::new().to_string();
    // The store's hash, salted per deployment (BUG-036) — a static would not know the salt.
    let qh = store.qhash(normalized_query);
    if let Ok(mut map) = state.interaction_tokens.write() {
        let now = Instant::now();
        map.retain(|_, (_, minted)| now.duration_since(*minted) < TTL);
        if map.len() >= MAX_TOKENS {
            // Under genuine pressure, drop the oldest: a lost click attribution on an old search
            // beats an unbounded map — the same trade the summary token store makes.
            if let Some(oldest) = map
                .iter()
                .min_by_key(|(_, (_, minted))| *minted)
                .map(|(k, _)| k.clone())
            {
                map.remove(&oldest);
            }
        }
        map.insert(token.clone(), (qh, now));
    }
    token
}

pub(crate) fn to_card(hit: &Value) -> ResultCard {
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
        media: media_of(hit),
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
        from_web: false,
    }
}

/// Mix live web results into `results`: append each federated hit not already present as a local
/// result, flagged `from_web`. Dedup is by the URL-derived id — the same id the eager index and the
/// later full crawl both assign — so once a federated URL has been crawled it appears as a normal
/// local result and its web card drops out. This is the M7-T06 convergence mechanism: the blend
/// share of any given URL falls to zero the moment the crawler catches up with it.
fn merge_federated(
    results: &mut Vec<ResultCard>,
    federated_hits: &[xustive_ingest::federation::FederatedHit],
) {
    let mut seen_ids: std::collections::HashSet<String> =
        results.iter().map(|c| c.id.clone()).collect();
    // Also dedup by canonical URL (BUG-007): the URL-derived id only matches documents the
    // *federation* channel indexed — a page the crawler discovered on its own carries a ULID id,
    // so an id-only check let the same URL appear twice, once local and once "from the web".
    let mut seen_urls: std::collections::HashSet<String> = results
        .iter()
        .filter_map(|c| canonical_of(&c.url))
        .collect();
    for hit in federated_hits {
        let Some(canonical) = canonical_of(&hit.url) else {
            continue;
        };
        let id = xustive_core::id_for_url(&canonical);
        if seen_ids.insert(id.clone()) && seen_urls.insert(canonical.clone()) {
            results.push(federated_card(hit, &canonical, id));
        }
    }
}

/// A result URL folded to the same canonical form the frontier and the eager index key on, so the
/// federation merge compares like with like. `None` for anything unparseable.
fn canonical_of(url: &str) -> Option<String> {
    let safe = xustive_core::SafeUrl::parse(url).ok()?;
    Some(xustive_ingest::frontier::canonical(safe.as_url()))
}

/// Build a result card from a live federation hit (M7): its SearXNG title, snippet and engine. The id
/// is the URL-derived id the eager index uses, so an impression or click on this card attributes to
/// the same document once it is crawled — and so it dedups against the local result for the same URL.
fn federated_card(
    hit: &xustive_ingest::federation::FederatedHit,
    canonical: &str,
    id: String,
) -> ResultCard {
    ResultCard {
        media: federated_media(hit),
        id,
        title: if hit.title.is_empty() {
            canonical.to_string()
        } else {
            hit.title.clone()
        },
        url: canonical.to_string(),
        display_url: display_url(canonical),
        excerpt: hit.snippet.clone(),
        source_type: "web".into(),
        source_name: hit.engine.clone(),
        author: Value::Null,
        published_at: 0,
        published_at_precision: "unknown".into(),
        sentiment: None,
        engagement: Value::Null,
        language: String::new(),
        thumbnail_url: None,
        matched_comments: Vec::new(),
        score: 0.0,
        similar_count: 0,
        from_web: true,
    }
}

/// The tile for an image or video hit (M9-T06). Same shape as a crawled page's `media[]`, so the
/// Images and Videos tabs render a federated tile and a local one identically — and so the eager
/// index can store it and the next search finds it locally.
fn federated_media(hit: &xustive_ingest::federation::FederatedHit) -> Vec<MediaOut> {
    let Some(m) = &hit.media else {
        return Vec::new();
    };
    let provider = if m.kind == "video" {
        xustive_ingest::video::from_url(&m.src)
            .map(|v| v.provider.as_str().to_string())
            .or_else(|| Some("web".to_string()))
    } else {
        None
    };
    vec![MediaOut {
        kind: m.kind.clone(),
        url: m.src.clone(),
        thumb_url: m.thumb.clone(),
        provider,
        width: 0,
        height: 0,
    }]
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

    /// The id the eager index / full crawl assigns a URL — the same derivation both the federation
    /// merge and the indexer use, so a card built with it dedups against the crawled document.
    fn id_of(url: &str) -> String {
        let safe = xustive_core::SafeUrl::parse(url).expect("test url parses");
        let canonical = xustive_ingest::frontier::canonical(safe.as_url());
        xustive_core::id_for_url(&canonical)
    }

    fn fed_hit(url: &str) -> xustive_ingest::federation::FederatedHit {
        xustive_ingest::federation::FederatedHit {
            url: url.into(),
            title: "web title".into(),
            snippet: "web snippet".into(),
            engine: "duckduckgo".into(),
            rank: 1,
            media: None,
        }
    }

    #[test]
    fn a_federated_image_hit_becomes_a_tile_and_a_video_hit_links_to_its_watch_page() {
        // M9-T06: the same card shape as a crawled page's media, so a federated tile and a local
        // one render identically — and the provider is named from the watch URL.
        let mut img = fed_hit("https://observalgerie.com/visiter-alger");
        img.media = Some(xustive_ingest::federation::FederatedMedia {
            kind: "image".into(),
            src: "https://observalgerie.com/vue.jpg".into(),
            thumb: Some("https://tse1.mm.bing.net/th/x".into()),
            detail: Some("1980 x 1200".into()),
        });
        let mut results = Vec::new();
        merge_federated(&mut results, &[img]);
        assert_eq!(results[0].media.len(), 1);
        assert_eq!(results[0].media[0].kind, "image");
        assert_eq!(
            results[0].media[0].thumb_url.as_deref(),
            Some("https://tse1.mm.bing.net/th/x")
        );
        assert!(results[0].from_web);

        let mut vid = fed_hit("https://www.youtube.com/watch?v=TqIensHhtyY");
        vid.media = Some(xustive_ingest::federation::FederatedMedia {
            kind: "video".into(),
            src: "https://www.youtube.com/watch?v=TqIensHhtyY".into(),
            thumb: Some("https://i.ytimg.com/vi/TqIensHhtyY/hqdefault.jpg".into()),
            detail: Some("184.0".into()),
        });
        let mut results = Vec::new();
        merge_federated(&mut results, &[vid]);
        let m = &results[0].media[0];
        assert_eq!(m.kind, "video");
        assert_eq!(m.provider.as_deref(), Some("youtube"));
        assert!(!m.url.contains("embed"), "never an embed: {}", m.url);
    }

    fn local_card(id: String, url: &str) -> ResultCard {
        ResultCard {
            from_web: false,
            ..federated_card(&fed_hit(url), url, id)
        }
    }

    #[test]
    fn a_crawled_federated_url_converges_to_a_local_result() {
        // M7-T06.3. The engine has already crawled example.dz/a — it is a normal local result. The
        // same URL comes back from federation on the next identical query; it must NOT reappear as a
        // second "from the web" card. A never-seen federated URL (example.dz/b) still gets mixed in.
        let url_a = "https://example.dz/a";
        let mut results = vec![local_card(id_of(url_a), url_a)];

        merge_federated(
            &mut results,
            &[fed_hit(url_a), fed_hit("https://example.dz/b")],
        );

        // The crawled URL stayed a single, local (non-web) result — it converged.
        let a: Vec<_> = results.iter().filter(|c| c.id == id_of(url_a)).collect();
        assert_eq!(a.len(), 1, "the crawled URL must not double up");
        assert!(
            !a[0].from_web,
            "the crawled URL answers locally, not from the web"
        );
        // The genuinely new federated URL was mixed in, flagged from_web.
        let b = results
            .iter()
            .find(|c| c.id == id_of("https://example.dz/b"))
            .expect("the new federated URL is added");
        assert!(b.from_web);
    }

    #[test]
    fn a_url_indexed_under_another_channel_does_not_duplicate() {
        // BUG-007. A page the *crawler* discovered carries a ULID id, not the URL-derived one, so
        // an id-only dedup let the same URL show twice — local result + "from the web" card. The
        // merge must recognise the URL itself.
        let url = "https://example.dz/organic";
        let mut results = vec![local_card("01ARZ3NDEKTSV4RRFFQ69G5FAV".into(), url)];

        merge_federated(&mut results, &[fed_hit(url)]);

        assert_eq!(results.len(), 1, "same URL must not appear twice");
        assert!(!results[0].from_web, "the local result wins");
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
            ui: None,
            q: Some("x".into()),
            page: None,
            hits_per_page: None,
            lang: None,
            source: Some("web, facebook".into()),
            sentiment: Some("negative".into()),
            from: None,
            to: None,
            sort: None,
            v: None,
        };
        let f = parse_filters(&p).unwrap();
        assert_eq!(f.source_types, vec![SourceType::Web, SourceType::Facebook]);
        assert_eq!(f.sentiments, vec![SentimentLabel::Negative]);
        assert!(f.exclude_spam, "spam suppression should be on by default");
    }

    #[test]
    fn the_news_vertical_filters_to_dated_web_documents() {
        let p = SearchParams {
            ui: None,
            q: Some("x".into()),
            page: None,
            hits_per_page: None,
            lang: None,
            source: None,
            sentiment: None,
            from: None,
            to: None,
            sort: None,
            v: Some("news".into()),
        };
        let f = parse_filters(&p).unwrap();
        assert_eq!(f.source_types, vec![SourceType::Web]);
        assert!(
            f.exclude_unknown_dates,
            "news requires a real publication date"
        );
    }

    #[test]
    fn an_unknown_vertical_falls_back_to_all() {
        let p = SearchParams {
            ui: None,
            q: Some("x".into()),
            page: None,
            hits_per_page: None,
            lang: None,
            source: None,
            sentiment: None,
            from: None,
            to: None,
            sort: None,
            v: Some("wibble".into()),
        };
        let f = parse_filters(&p).unwrap();
        assert!(
            f.source_types.is_empty(),
            "an unknown vertical must not filter"
        );
        assert!(!f.exclude_unknown_dates);
    }

    #[test]
    fn unknown_facet_value_is_a_client_error() {
        let p = SearchParams {
            ui: None,
            q: Some("x".into()),
            page: None,
            hits_per_page: None,
            lang: None,
            source: Some("myspace".into()),
            sentiment: None,
            from: None,
            to: None,
            sort: None,
            v: None,
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
