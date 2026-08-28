//! The crawler sections of the admin console.
//!
//! See [[UI - Admin Console]] for the interface and [[Crawler Console]] for the behaviour.
//!
//! Everything here reads the counters the crawler publishes to Redis rather than keeping its own
//! tally. Two tallies drift, and when a console and a dashboard disagree nothing tells an operator
//! which is lying.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::Json;
use futures_util::stream::Stream;
use serde::Deserialize;
use serde_json::json;

use crate::admin::Peer;
use crate::state::AppState;

/// How often the live stream sends a frame.
///
/// One second. The number people watch is the document count climbing, and a count that jumps in
/// five-second steps reads as a stalled crawler.
const FRAME: Duration = Duration::from_secs(1);

/// The shared, once-connected crawl-stats store. If it was not connected at startup (Redis was
/// down), try once now and cache it — so the Live page self-heals when Redis comes back without a
/// restart, and still never reconnects per SSE frame in the healthy case.
async fn stats(state: &AppState) -> Option<xustive_ingest::crawl_stats::CrawlStats> {
    if let Some(s) = state.crawl_stats() {
        return Some(s);
    }
    state.connect_crawl_stats().await;
    state.crawl_stats()
}

fn frontier(state: &AppState) -> Option<xustive_ingest::frontier::Frontier> {
    xustive_ingest::frontier::Frontier::connect(&state.config.queue.url)
        .ok()
        .map(|f| {
            f.with_limits(xustive_ingest::frontier::FrontierLimits::from_config(
                &state.config.crawl,
            ))
        })
}

/// The full picture: counters, recent URLs, per-host activity, frontier depth.
pub(crate) async fn snapshot(state: &AppState) -> xustive_ingest::crawl_stats::Snapshot {
    let Some(s) = stats(state).await else {
        return xustive_ingest::crawl_stats::Snapshot {
            unavailable: true,
            state: "unknown".into(),
            ..Default::default()
        };
    };
    let mut snap = s.snapshot().await;
    if let Some(f) = frontier(state) {
        let (waiting, inflight) = f.depth().await;
        snap.waiting = waiting;
        snap.inflight = inflight;
        snap.deferred = f.deferred().await;
    }
    snap
}

/// `GET /admin/crawler/status`
pub async fn status(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;
    Ok(Json(
        serde_json::to_value(snapshot(&state).await).unwrap_or_else(|_| json!({})),
    ))
}

/// `GET /admin/crawler/events` — the live stream.
///
/// One connection carries every live number on the page. Several would mean several reconnect
/// storms whenever the API restarts, and they would drift apart between frames.
pub async fn events(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;

    let stream = async_stream::stream! {
        let mut ticker = tokio::time::interval(FRAME);
        loop {
            ticker.tick().await;
            let snap = snapshot(&state).await;
            // Absolute values, never deltas. A client that misses a frame then loses nothing;
            // with deltas it would drift silently until someone reloaded.
            let payload = serde_json::to_string(&snap)
                .unwrap_or_else(|_| r#"{"unavailable":true}"#.to_string());
            yield Ok::<Event, Infallible>(Event::default().data(payload));
        }
    };
    Ok(sse(stream))
}

fn sse<S>(stream: S) -> impl IntoResponse
where
    S: Stream<Item = Result<Event, Infallible>> + Send + 'static,
{
    Sse::new(stream).keep_alive(KeepAlive::default().interval(Duration::from_secs(15)))
}

#[derive(Debug, Deserialize)]
pub struct DocumentQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub lang: Option<String>,
    /// Discovery channel, e.g. `federation`, `seed`, `link` — provenance filter (M7). Empty = all.
    #[serde(default)]
    pub channel: Option<String>,
    /// `image` or `video`: only documents carrying media of that kind (M9). Empty = all.
    #[serde(default)]
    pub media: Option<String>,
    #[serde(default)]
    pub page: Option<usize>,
}

/// `GET /admin/crawler/documents` — what has actually been collected.
///
/// Backed by the product's own index, so it is the same engine and fast at any corpus size. Paged
/// rather than "all": a list that loads everything is fine at a thousand documents and unusable at
/// a million, and that failure arrives exactly when the crawler starts working.
pub async fn documents(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
    Query(params): Query<DocumentQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;

    let page = params.page.unwrap_or(1).max(1);
    let per_page = state.config.crawl.documents_page_size;

    let mut filters: Vec<String> = Vec::new();
    // `domain` and `language`, which is what the index actually calls them. The first version
    // guessed `host` and `lang`: Meilisearch accepted the query and returned nulls, so the filter
    // silently matched nothing rather than erroring.
    if let Some(host) = params.host.as_deref().filter(|h| !h.is_empty()) {
        // `www.` is stripped when the document is stored, so it is stripped here too. Typing the
        // host exactly as it appears in the browser is the obvious thing to do and returned zero
        // results, which reads as "nothing crawled" rather than "wrong spelling".
        // Lower-cased *before* stripping: `trim_start_matches` is case-sensitive, so doing it the
        // other way round left `WWW.APS.DZ` matching nothing.
        let host = host.trim().to_ascii_lowercase();
        let host = host.trim_start_matches("www.");
        // Quoted, because a host is user input arriving in a filter expression.
        filters.push(format!("domain = {host:?}"));
    }
    if let Some(lang) = params.lang.as_deref().filter(|l| !l.is_empty()) {
        filters.push(format!("language = {lang:?}"));
    }
    // The provenance filter is kept *separate* from the scope filters (host/lang) on purpose: the
    // composition facet is computed over the scope alone, so it always shows every channel to pick
    // from, while the document list drills into the chosen one. Applying the channel filter to the
    // facet too would collapse the breakdown to a single bar.
    let channel_filter = params
        .channel
        .as_deref()
        .filter(|c| !c.is_empty())
        .map(|c| format!("discovery = {c:?}"));
    let scope = filters.join(" AND ");
    let q = params.q.clone().unwrap_or_default();

    // 1. Composition: facet the index by `discovery` within the scope, no documents needed. This is
    //    the "crawler vs external tools" breakdown the console shows.
    // `media.type` is faceted beside provenance so the page can enumerate documents with images
    // and with videos apart from plain pages (M9). Meilisearch counts a document once per
    // distinct value, so a page with three images counts once under `image`.
    let mut composition_q = xustive_search::Query::new(q.clone())
        .limit(0)
        .facets(&["discovery", "media.type"]);
    if !scope.is_empty() {
        composition_q = composition_q.filter(scope.clone());
    }
    let composition = match state
        .search
        .search::<serde_json::Value>(&state.documents_index(), &composition_q)
        .await
    {
        Ok(h) => h.facet_distribution,
        // A facet failure must not sink the whole page — the list below is the primary content.
        Err(e) => {
            tracing::warn!(error = %e, "index composition facet failed");
            json!({})
        }
    };

    // 2. The document page, with scope + the optional channel drill-in.
    let mut list_filters = filters.clone();
    if let Some(cf) = &channel_filter {
        list_filters.push(cf.clone());
    }
    // The media drill-in, like the channel one: kept out of the composition scope so the counts
    // always show every kind to pick from.
    if let Some(kind) = params
        .media
        .as_deref()
        .filter(|k| matches!(*k, "image" | "video"))
    {
        list_filters.push(format!("media.type = {kind:?}"));
    }
    let mut query = xustive_search::Query::new(q)
        .limit(per_page)
        .offset((page - 1) * per_page)
        .sort(&["crawled_at:desc"]);
    if !list_filters.is_empty() {
        query = query.filter(list_filters.join(" AND "));
    }

    match state
        .search
        .search::<serde_json::Value>(&state.documents_index(), &query)
        .await
    {
        Ok(hits) => Ok(Json(json!({
            "hits": hits.hits,
            "estimated_total": hits.estimated_total_hits,
            "page": page,
            "per_page": per_page,
            // `{ "discovery": { "federation": 12, "seed": 340, ... } }` — the index by provenance.
            "composition": composition.get("discovery").cloned().unwrap_or_else(|| json!({})),
            // `{ "image": 176, "video": 21 }` — documents carrying media of each kind, within the
            // scope. Enumerated apart from pages on purpose (M9).
            "media": composition.get("media.type").cloned().unwrap_or_else(|| json!({})),
        }))),
        Err(e) => {
            tracing::warn!(error = %e, "admin document list failed");
            Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": {"code": "index_unavailable", "message": e.to_string()}})),
            ))
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct EnqueueRequest {
    pub url: String,
    #[serde(default)]
    pub front: bool,
}

/// `POST /admin/crawler/enqueue`
///
/// Reorders; does not grant. The URL passes `SafeUrl` and the trap detectors exactly as a
/// discovered link does — an admin form that could fetch anything would be an SSRF hole with a
/// login page in front of it, and the login is not the part that stops it.
/// `POST /admin/crawler/pause` — hold or release the crawl (PROB-003).
///
/// The one control the console lacked entirely: state was displayed everywhere and changeable
/// nowhere. The flag lives in Redis beside the crawl state, every worker polls it in its guard
/// probe (effect within seconds), and it survives restarts of both the console and the crawler.
/// Pausing holds claims only — in-flight fetches finish, nothing is dropped, and the frontier is
/// untouched, so resuming costs nothing.
/// `POST /admin/crawler/weak-coverage/forget` — dismiss one weak-coverage term (PROB-003).
///
/// `WeakCoverage::forget` existed in code with no endpoint: an operator who judged a term not
/// worth chasing (a typo, a name, noise) could only wait out its window. If the gap is real, the
/// term re-accumulates past the k-floor on its own — forgetting is dismissal, not suppression.
pub async fn weak_forget(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;
    let Some(term) = body.get("term").and_then(|v| v.as_str()).map(str::trim) else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"code": "missing_term", "message": "pass {\"term\": \"...\"}"}})),
        ));
    };
    if term.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"code": "missing_term", "message": "term must not be empty"}})),
        ));
    }
    let disc = &state.config.discovery;
    let Some(store) = xustive_ingest::weak_coverage::WeakCoverage::connect_in(
        state.config.queue.signals_url(),
        "discovery",
        disc.effective_k(),
        std::time::Duration::from_secs(disc.weak_coverage_window_days * 86_400),
    ) else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(
                json!({"error": {"code": "redis_unavailable", "message": "cannot reach the signals store"}}),
            ),
        ));
    };
    store.forget(term).await;
    Ok(Json(json!({ "ok": true })))
}

pub async fn pause(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;
    let paused = body.get("paused").and_then(|v| v.as_bool()).unwrap_or(true);
    let Some(stats) =
        xustive_ingest::crawl_stats::CrawlStats::connect(&state.config.queue.url).await
    else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(
                json!({"error": {"code": "redis_unavailable", "message": "cannot reach the crawl state store"}}),
            ),
        ));
    };
    stats.set_paused(paused).await;
    tracing::warn!(paused, "crawl pause toggled by operator");
    Ok(Json(json!({ "ok": true, "paused": paused })))
}

pub async fn enqueue(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
    Json(req): Json<EnqueueRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;

    let safe = xustive_core::SafeUrl::parse(&req.url).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"code": "unsafe_url", "message": e.to_string()}})),
        )
    })?;

    let parsed = safe.as_url().clone();
    if let Some(trap) = xustive_ingest::frontier::detect_trap(&parsed) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"code": "trap", "message": trap.as_str()}})),
        ));
    }

    let Some(f) = frontier(&state) else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": {"code": "no_frontier", "message": "cannot reach the frontier"}})),
        ));
    };

    let host = safe.authority();
    let pending = xustive_ingest::frontier::Pending {
        url: xustive_ingest::frontier::canonical(&parsed),
        host: host.clone(),
        source_id: "admin".into(),
        depth: 0,
        // A URL an operator typed is as trusted as a seed, and its outlinks inherit that.
        trust: 100,
        channel: xustive_core::DiscoveryChannel::Seed,
        priority: xustive_ingest::frontier::priority_for(0, 100, true),
    };

    let added = f.add(&pending).await.unwrap_or(false);
    if req.front {
        f.promote(&host, &pending.url).await;
    }

    tracing::info!(peer = ?peer, url = %pending.url, front = req.front, added, "queued from admin");
    Ok(Json(json!({
        "ok": true,
        "added": added,
        "already_known": !added,
        "url": pending.url,
    })))
}

// ── Sources ─────────────────────────────────────────────────────────────────────────────────

/// One line of the seed file.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Seed {
    pub source_id: String,
    pub url: String,
    pub trust: String,
    /// One of: news, government, education, health, science-tech, sport, culture, business,
    /// reference. Empty for legacy rows or operator-added seeds without one.
    pub category: String,
    /// `dz` (Algerian) or `global`. Empty when unspecified.
    pub region: String,
    pub note: String,
}

/// Read the seed file.
///
/// Comments and blank lines are kept out of the parsed list but preserved on write — the file
/// carries the reasoning for several decisions (why social platforms are absent, why some hosts
/// are unreachable) and an admin console that silently ate it would be destroying the only place
/// that explains the list.
fn read_seeds(path: &str) -> (Vec<Seed>, String) {
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    let seeds = raw
        .lines()
        .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let mut parts = l.split('\t');
            Some(Seed {
                source_id: parts.next()?.trim().to_string(),
                url: parts.next()?.trim().to_string(),
                trust: parts.next().unwrap_or("B").trim().to_string(),
                category: parts.next().unwrap_or("").trim().to_string(),
                region: parts.next().unwrap_or("").trim().to_string(),
                note: parts.next().unwrap_or("").trim().to_string(),
            })
        })
        .filter(|s| s.url.starts_with("http"))
        .collect();
    (seeds, raw)
}

/// `GET /admin/crawler/sources`
pub async fn sources(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;
    let (seeds, _) = read_seeds(&state.config.crawl.seeds_path);
    Ok(Json(json!({ "seeds": seeds })))
}

/// Build the per-source quality rows (M2-T11.5): each registry source joined to its crawl counters,
/// with the §7 ratios computed. A ratio is `null` when the source has no data yet, so the console
/// shows "—" rather than a misleading 0 %. Sources with no registry entry but with counters (e.g.
/// TSV-only seeds) are included too, so the dashboard never hides work the crawler actually did.
async fn source_health_rows(state: &AppState) -> Vec<serde_json::Value> {
    let metrics = match stats(state).await {
        Some(s) => s.source_metrics().await,
        None => std::collections::HashMap::new(),
    };
    let registry = xustive_core::Registry::load(&state.config.crawl.registry_path).ok();

    let mut rows = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Registry sources first, in id order, each with whatever counters it has accrued.
    if let Some(reg) = &registry {
        let mut sources: Vec<&xustive_core::Source> = reg.sources().iter().collect();
        sources.sort_by(|a, b| a.id.cmp(&b.id));
        for s in sources {
            seen.insert(s.id.clone());
            let m = metrics.get(&s.id).cloned().unwrap_or_default();
            rows.push(row_json(&s.id, Some(s), &m));
        }
    }
    // Then any source with counters but no registry record.
    let mut orphans: Vec<(&String, &xustive_ingest::crawl_stats::SourceMetrics)> = metrics
        .iter()
        .filter(|(id, _)| !seen.contains(*id))
        .collect();
    orphans.sort_by(|a, b| a.0.cmp(b.0));
    for (id, m) in orphans {
        rows.push(row_json(id, None, m));
    }
    rows
}

fn row_json(
    id: &str,
    source: Option<&xustive_core::Source>,
    m: &xustive_ingest::crawl_stats::SourceMetrics,
) -> serde_json::Value {
    // A ratio is emitted only when it is defined; `null` reads as "—" on the page.
    let ratio = |v: Option<f32>| v.map(|x| (x * 1000.0).round() / 1000.0);
    json!({
        "id": id,
        "display_name": source.map(|s| s.display_name.clone()),
        "lifecycle": source.map(|s| format!("{:?}", s.lifecycle).to_lowercase()),
        "trust_tier": source.map(|s| format!("{:?}", s.trust_tier)),
        "approved": source.map(|s| s.approved),
        "crawlable": source.map(|s| s.is_crawlable()),
        // The crawl policy, so the health page can offer the lifecycle/policy controls (PROB-003)
        // next to the numbers that motivate using them.
        "policy": source.map(|s| json!({
            "enabled": s.crawl_policy.enabled,
            "frequency": format!("{:?}", s.crawl_policy.frequency).to_lowercase(),
            "max_docs_per_run": s.crawl_policy.max_docs_per_run,
            "crawl_delay_ms": s.crawl_policy.crawl_delay_ms,
            "depth_limit": s.crawl_policy.depth_limit,
        })),
        "counts": {
            "fetched": m.fetched,
            "failed": m.failed,
            "indexed": m.indexed,
            "thin": m.thin,
            "duplicate": m.duplicate,
        },
        "quality": {
            "fetch_success_rate": ratio(m.fetch_success_rate()),
            "extraction_success_rate": ratio(m.extraction_success_rate()),
            "duplicate_ratio": ratio(m.duplicate_ratio()),
            "spam_mean": ratio(m.spam_mean()),
            "date_unknown_ratio": ratio(m.date_unknown_ratio()),
        },
    })
}

/// `GET /admin/crawler/sources/health` — per-source quality metrics as JSON (M2-T11.5).
pub async fn sources_health(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;
    let rows = source_health_rows(&state).await;
    Ok(Json(json!({ "sources": rows })))
}

/// `GET /admin/crawler/channels` — per-channel discovery yield as JSON (M2-T16.8).
///
/// The funnel per discovery channel: discovered → fetched → indexed → survived dedup, plus the
/// yield and unique rates. This is the number that decides whether an expensive channel earns its
/// place. Rows are ordered by indexed descending, so the channels doing the work sort to the top.
pub async fn channels(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;
    let metrics = match stats(&state).await {
        Some(s) => s.channel_metrics().await,
        None => std::collections::HashMap::new(),
    };
    let ratio = |v: Option<f32>| v.map(|x| (x * 1000.0).round() / 1000.0);
    let mut rows: Vec<serde_json::Value> = metrics
        .into_iter()
        .map(|(channel, m)| {
            json!({
                "channel": channel,
                "discovered": m.discovered,
                "fetched": m.fetched,
                "indexed": m.indexed,
                "duplicate": m.duplicate,
                "yield_rate": ratio(m.yield_rate()),
                "unique_rate": ratio(m.unique_rate()),
            })
        })
        .collect();
    // Ordered by indexed desc; the channel doing the most work is what an operator looks at first.
    rows.sort_by(|a, b| {
        b["indexed"]
            .as_u64()
            .cmp(&a["indexed"].as_u64())
            .then_with(|| a["channel"].as_str().cmp(&b["channel"].as_str()))
    });
    Ok(Json(json!({ "channels": rows })))
}

/// `GET /admin/crawler/weak-coverage` — the weak-coverage queue as JSON (M2-T16.5).
///
/// Surfaces only the terms `WeakCoverage` is willing to return, which are already k-anonymous
/// (≥ 20 searches). When the feature is off — the default — it reports that plainly rather than an
/// empty list, so an operator can tell "disabled" from "no gaps".
pub async fn weak_coverage(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;
    let disc = &state.config.discovery;
    if !disc.weak_coverage_enabled {
        return Ok(Json(json!({
            "enabled": false,
            "k_anonymity": disc.effective_k(),
            "resolution": {
                "serp_enabled": disc.serp_enabled,
                "brave_usable": disc.brave_usable(),
            },
            "terms": [],
        })));
    }
    let terms = xustive_ingest::weak_coverage::WeakCoverage::connect_in(
        state.config.queue.signals_url(),
        "discovery",
        disc.effective_k(),
        std::time::Duration::from_secs(disc.weak_coverage_window_days * 86_400),
    );
    let rows: Vec<serde_json::Value> = match terms {
        Some(w) => w
            .weak_terms(200)
            .await
            .into_iter()
            .map(|t| json!({ "term": t.term, "count": t.count }))
            .collect(),
        None => Vec::new(),
    };
    // Entity demand (M8-T09): panel-shaped queries that resolved to nothing. A different kind of
    // gap from a weak search — this one wants a harvest, not a crawl source — so it is listed
    // separately rather than mixed into the same table.
    let entity_rows: Vec<serde_json::Value> =
        match xustive_ingest::weak_coverage::WeakCoverage::connect_in(
            state.config.queue.signals_url(),
            crate::knowledge::DEMAND_NAMESPACE,
            disc.effective_k(),
            std::time::Duration::from_secs(disc.weak_coverage_window_days * 86_400),
        ) {
            Some(w) => w
                .weak_terms(200)
                .await
                .into_iter()
                .map(|t| json!({ "term": t.term, "count": t.count }))
                .collect(),
            None => Vec::new(),
        };

    Ok(Json(json!({
        "enabled": true,
        "k_anonymity": disc.effective_k(),
        // Whether anything can actually RESOLVE these terms (PROB-003): the page used to promise
        // "the crawler chases these automatically" with no way to see that no source was wired.
        "resolution": {
            "serp_enabled": disc.serp_enabled,
            "brave_usable": disc.brave_usable(),
        },
        "terms": rows,
        "entities": entity_rows,
    })))
}

#[derive(Debug, Deserialize)]
pub struct AddSeed {
    pub url: String,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub trust: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

/// The known categories, in display order. An operator-supplied category outside this set is stored
/// verbatim (the UI groups it under "other") rather than rejected — the list should not block a
/// reasonable label it has not seen.
pub const CATEGORIES: [&str; 9] = [
    "news",
    "government",
    "education",
    "health",
    "science-tech",
    "sport",
    "culture",
    "business",
    "reference",
];

/// `POST /admin/crawler/sources` — add a seed and crawl it next.
///
/// **Enqueued at the front**, which is the point. Priority sorts trusted sources first, so a newly
/// added tier-B seed otherwise sits behind everything already known — with a frontier thousands of
/// URLs deep that is days, and "I added a source" and "I can search it" being days apart is
/// exactly the confusion this control exists to remove.
///
/// The URL still passes `SafeUrl` and the trap detectors. The console changes ordering, never
/// permission.
pub async fn add_source(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
    Json(req): Json<AddSeed>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;

    let safe = xustive_core::SafeUrl::parse(req.url.trim()).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"code": "unsafe_url", "message": e.to_string()}})),
        )
    })?;
    let parsed = safe.as_url().clone();
    if let Some(trap) = xustive_ingest::frontier::detect_trap(&parsed) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"code": "trap", "message": trap.as_str()}})),
        ));
    }

    let host = safe.authority();
    // Derived from the host when not given, so adding a source is one field rather than three.
    let source_id = req
        .source_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            host.trim_start_matches("www.")
                .replace('.', "-")
                .to_ascii_lowercase()
        });
    let trust = match req.trust.as_deref() {
        Some("A") => "A",
        Some("C") => "C",
        // B by default: credible until we have a reason to say otherwise, and the tier is about
        // accountability rather than agreement.
        _ => "B",
    };

    let path = &state.config.crawl.seeds_path;
    let (existing, raw) = read_seeds(path);
    let already = existing.iter().any(|s| s.url == safe.as_str());

    if !already {
        let note = req
            .note
            .as_deref()
            .unwrap_or("added from the admin console");
        // Category from the operator (else blank → "other" in the UI). Region is derived: a .dz host
        // is Algerian, everything else global — the same split the catalog uses.
        let category = req.category.as_deref().map(str::trim).unwrap_or("");
        let region = if host.trim_end_matches('.').ends_with(".dz") || host == "dz" {
            "dz"
        } else {
            "global"
        };
        let line = format!(
            "{source_id}\t{}\t{trust}\t{category}\t{region}\t{note}\n",
            safe.as_str()
        );
        // Written whole through a temporary file. A partial write to the seed list would leave the
        // crawler reading a truncated file on its next start, and the failure would look like
        // sources vanishing.
        let tmp = format!("{path}.tmp");
        let updated = format!("{}\n{line}", raw.trim_end_matches('\n'));
        if std::fs::write(&tmp, &updated).is_err() || std::fs::rename(&tmp, path).is_err() {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    json!({"error": {"code": "write_failed", "message": "could not write the seed file"}}),
                ),
            ));
        }
    }

    // Queued at the front regardless of whether the file already had it — "add" on a known source
    // is a reasonable way to say "crawl this now".
    let mut queued = false;
    if let Some(f) = frontier(&state) {
        let pending = xustive_ingest::frontier::Pending {
            url: xustive_ingest::frontier::canonical(&parsed),
            host: host.clone(),
            source_id: source_id.clone(),
            depth: 0,
            trust: 100,
            channel: xustive_core::DiscoveryChannel::Seed,
            priority: i64::MIN / 2,
        };
        let added = f.add(&pending).await.unwrap_or(false);
        f.promote(&host, &pending.url).await;
        queued = added;
    }

    tracing::info!(peer = ?peer, url = %safe.as_str(), %source_id, trust, "source added");
    Ok(Json(json!({
        "ok": true,
        "source_id": source_id,
        "url": safe.as_str(),
        "trust": trust,
        "already_listed": already,
        "queued": queued,
    })))
}

#[derive(Debug, Deserialize)]
pub struct RemoveSeed {
    pub url: String,
}

/// `POST /admin/crawler/sources/remove`
///
/// Removes the line from the seed file. It does **not** remove what has already been crawled from
/// that source — those are separate actions, and conflating them would make "stop crawling this"
/// silently delete a section of the index.
pub async fn remove_source(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
    Json(req): Json<RemoveSeed>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;

    let path = &state.config.crawl.seeds_path;
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    let target = req.url.trim();

    let mut removed = 0usize;
    let kept: Vec<&str> = raw
        .lines()
        .filter(|l| {
            let is_seed = !l.trim_start().starts_with('#') && l.contains('\t');
            if is_seed && l.split('\t').nth(1).map(str::trim) == Some(target) {
                removed += 1;
                return false;
            }
            true
        })
        .collect();

    if removed == 0 {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"code": "not_listed", "message": "no seed with that url"}})),
        ));
    }

    let tmp = format!("{path}.tmp");
    let updated = format!("{}\n", kept.join("\n"));
    if std::fs::write(&tmp, &updated).is_err() || std::fs::rename(&tmp, path).is_err() {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                json!({"error": {"code": "write_failed", "message": "could not write the seed file"}}),
            ),
        ));
    }

    tracing::info!(peer = ?peer, url = %target, "source removed");
    Ok(Json(json!({
        "ok": true,
        "removed": removed,
        // Said explicitly, because the obvious assumption is the opposite.
        "note": "documents already crawled from this source remain in the index",
    })))
}

#[derive(Debug, Deserialize)]
pub struct RegistryEdit {
    pub id: String,
    /// One of `approve`, `activate`, `disable`. Absent when only the policy changes.
    #[serde(default)]
    pub action: Option<String>,
    /// Recorded on `disable` — the registry keeps *why* a human turned a source off.
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub policy: Option<PolicyEdit>,
}

/// Every field optional: the operator changes one knob, the rest stay as they are.
#[derive(Debug, Deserialize)]
pub struct PolicyEdit {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub frequency: Option<String>,
    #[serde(default)]
    pub max_docs_per_run: Option<u32>,
    #[serde(default)]
    pub crawl_delay_ms: Option<u64>,
    #[serde(default)]
    pub depth_limit: Option<u8>,
}

/// `POST /admin/crawler/registry` — the registry lifecycle and per-source crawl policy (PROB-003).
///
/// The same transitions as `xustive registry approve|activate|disable`, plus the policy fields the
/// CLI never had a verb for (frequency, per-run doc cap, crawl delay, depth). The guards match the
/// CLI exactly — an archived source must be re-proposed, not resurrected from a console button —
/// and the policy floors keep a typo from becoming an impolite crawler: the delay can be raised
/// freely but never set below 500 ms, and `respect_robots` is not editable at all.
pub async fn registry_edit(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
    Json(req): Json<RegistryEdit>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;

    let bad = |code: &str, message: String| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"code": code, "message": message}})),
        )
    };

    let path = &state.config.crawl.registry_path;
    let mut reg = xustive_core::Registry::load(path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": {"code": "registry_unreadable", "message": e.to_string()}})),
        )
    })?;
    let Some(s) = reg.get_mut(req.id.trim()) else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"code": "unknown_source", "message": "no source with that id"}})),
        ));
    };

    let mut changed: Vec<&'static str> = Vec::new();

    match req.action.as_deref() {
        None => {}
        Some("approve") => {
            if s.lifecycle == xustive_core::Lifecycle::Archived {
                return Err(bad(
                    "archived",
                    "this source is archived; re-propose it before approving".into(),
                ));
            }
            s.approved = true;
            s.lifecycle = xustive_core::Lifecycle::Approved;
            changed.push("approved");
        }
        Some("activate") => {
            if s.lifecycle == xustive_core::Lifecycle::Archived {
                return Err(bad(
                    "archived",
                    "this source is archived; re-propose it before activating".into(),
                ));
            }
            s.approved = true;
            s.lifecycle = xustive_core::Lifecycle::Active;
            changed.push("activated");
        }
        Some("disable") => {
            let reason = req
                .reason
                .as_deref()
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .unwrap_or("operator disabled (console)");
            if !s.disable_at(reason, xustive_core::now_unix()) {
                return Err(bad(
                    "already_disabled",
                    "this source is already disabled or archived".into(),
                ));
            }
            changed.push("disabled");
        }
        Some(other) => {
            return Err(bad(
                "unknown_action",
                format!("unknown action {other:?} — expected approve, activate, or disable"),
            ));
        }
    }

    if let Some(p) = &req.policy {
        if let Some(enabled) = p.enabled {
            s.crawl_policy.enabled = enabled;
            changed.push("policy.enabled");
        }
        if let Some(f) = p.frequency.as_deref() {
            s.crawl_policy.frequency = match f {
                "realtime" => xustive_core::CrawlFrequency::Realtime,
                "hourly" => xustive_core::CrawlFrequency::Hourly,
                "daily" => xustive_core::CrawlFrequency::Daily,
                "weekly" => xustive_core::CrawlFrequency::Weekly,
                other => {
                    return Err(bad(
                        "bad_frequency",
                        format!(
                            "unknown frequency {other:?} — expected realtime, hourly, daily, or weekly"
                        ),
                    ));
                }
            };
            changed.push("policy.frequency");
        }
        if let Some(n) = p.max_docs_per_run {
            if !(1..=100_000).contains(&n) {
                return Err(bad(
                    "bad_max_docs",
                    "max_docs_per_run must be between 1 and 100000".into(),
                ));
            }
            s.crawl_policy.max_docs_per_run = n;
            changed.push("policy.max_docs_per_run");
        }
        if let Some(ms) = p.crawl_delay_ms {
            // The politeness floor: the console can slow a source down as far as it likes, but
            // never below half a second — the same spirit as robots.rs never undercutting a
            // declared Crawl-delay.
            if ms < 500 {
                return Err(bad(
                    "bad_delay",
                    "crawl_delay_ms must be at least 500".into(),
                ));
            }
            s.crawl_policy.crawl_delay_ms = ms;
            changed.push("policy.crawl_delay_ms");
        }
        if let Some(d) = p.depth_limit {
            if !(1..=10).contains(&d) {
                return Err(bad(
                    "bad_depth",
                    "depth_limit must be between 1 and 10".into(),
                ));
            }
            s.crawl_policy.depth_limit = d;
            changed.push("policy.depth_limit");
        }
    }

    if changed.is_empty() {
        return Err(bad(
            "nothing_to_do",
            "no action and no policy field given".into(),
        ));
    }

    let lifecycle = format!("{:?}", s.lifecycle).to_lowercase();
    let crawlable = s.is_crawlable();

    // Written whole through a temporary file, like the seed list: a partial registry is sources
    // vanishing on the crawler's next start.
    let text = reg.to_jsonl().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": {"code": "serialize_failed", "message": e.to_string()}})),
        )
    })?;
    let tmp = format!("{path}.tmp");
    if std::fs::write(&tmp, &text).is_err() || std::fs::rename(&tmp, path).is_err() {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                json!({"error": {"code": "write_failed", "message": "could not write the registry"}}),
            ),
        ));
    }

    tracing::info!(peer = ?peer, id = %req.id, ?changed, %lifecycle, "registry edited");
    Ok(Json(json!({
        "ok": true,
        "id": req.id,
        "changed": changed,
        "lifecycle": lifecycle,
        "crawlable": crawlable,
        // The crawler reads the registry when it seeds, not continuously.
        "note": "takes effect on the crawler's next seeding pass",
    })))
}
