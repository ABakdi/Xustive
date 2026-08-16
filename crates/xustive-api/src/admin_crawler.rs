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

use crate::admin::{escape_html, Peer};
use crate::state::AppState;

/// How often the live stream sends a frame.
///
/// One second. The number people watch is the document count climbing, and a count that jumps in
/// five-second steps reads as a stalled crawler.
const FRAME: Duration = Duration::from_secs(1);

fn stats(state: &AppState) -> Option<xustive_ingest::crawl_stats::CrawlStats> {
    xustive_ingest::crawl_stats::CrawlStats::connect(&state.config.queue.url)
}

fn frontier(state: &AppState) -> Option<xustive_ingest::frontier::Frontier> {
    xustive_ingest::frontier::Frontier::connect(&state.config.queue.url).ok()
}

/// The full picture: counters, recent URLs, per-host activity, frontier depth.
async fn snapshot(state: &AppState) -> xustive_ingest::crawl_stats::Snapshot {
    let Some(s) = stats(state) else {
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

    // The product's own typed query, not a raw body. A second, hand-built search path would drift
    // from the one users hit — and the point of this section is to see what they would see.
    let mut query = xustive_search::Query::new(params.q.clone().unwrap_or_default())
        .limit(per_page)
        .offset((page - 1) * per_page)
        .sort(&["crawled_at:desc"]);
    if !filters.is_empty() {
        query = query.filter(filters.join(" AND "));
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

/// The crawler section of the console, as HTML.
pub fn section_live() -> String {
    // Server-rendered skeleton; the SSE stream fills the numbers. Rendering the first frame here
    // too would double the code that formats it, and the stream's first frame arrives within a
    // second.
    r#"<h1>Crawler — live</h1>
<div id="crawl-unavailable" class="lede state-bad" hidden>
  Cannot read crawler state. This is <strong>not</strong> the same as an idle crawler —
  Redis is unreachable, so we do not know what it is doing.
</div>

<div class="tiles" id="crawl-tiles">
  <div class="tile"><span class="tile-n" id="c-indexed">–</span><span class="tile-l">indexed</span></div>
  <div class="tile"><span class="tile-n" id="c-fetched">–</span><span class="tile-l">fetched</span></div>
  <div class="tile"><span class="tile-n" id="c-revisited">–</span><span class="tile-l">of them revisits</span></div>
  <div class="tile"><span class="tile-n" id="c-discovered">–</span><span class="tile-l">discovered</span></div>
  <div class="tile"><span class="tile-n" id="c-waiting">–</span><span class="tile-l">queued</span></div>
  <div class="tile"><span class="tile-n" id="c-deferred">–</span><span class="tile-l">revisits booked</span></div>
  <div class="tile"><span class="tile-n" id="c-failed">–</span><span class="tile-l">failed</span></div>
</div>

<h2>Recent</h2>
<p class="muted">The last fifty URLs. This is what shows whether it is collecting articles or tag
pages — no total can.</p>
<table class="admin wide"><thead>
  <tr><th>outcome</th><th>words</th><th>host</th><th>url</th></tr>
</thead><tbody id="crawl-recent"></tbody></table>

<h2>Skips</h2>
<p class="muted">"Collecting nothing" has to resolve to which rule is eating everything.</p>
<table class="admin"><tbody id="crawl-skips"></tbody></table>

<h2>Hosts</h2>
<p class="muted">A host that stopped answering is the row that stops moving.</p>
<table class="admin"><tbody id="crawl-hosts"></tbody></table>
"#
    .to_string()
}

pub fn section_documents() -> String {
    format!(
        r#"<h1>Documents</h1>
<p class="lede">Everything indexed, newest first. This is the section that answers whether the
crawler is collecting the <em>right</em> things — a count rises the same whether we are finding
news or four hundred copies of one calendar page.</p>

<form class="admin" id="doc-filters">
  <input type="search" id="doc-q" placeholder="search title, url, body — press /" autocomplete="off">
  <input type="text" id="doc-host" placeholder="domain, e.g. aps.dz" autocomplete="off">
  <select id="doc-lang">
    <option value="">any language</option>
    <option value="ar">العربية</option>
    <option value="ary">الدارجة</option>
    <option value="fr">Français</option>
    <option value="en">English</option>
  </select>
  <button type="submit">Filter</button>
</form>

<p class="muted" id="doc-count">{placeholder}</p>
<table class="admin wide"><thead>
  <tr><th>title</th><th>domain</th><th>language</th><th>length</th><th>published</th></tr>
</thead><tbody id="doc-rows"></tbody></table>
<div id="doc-pager"></div>
"#,
        placeholder = escape_html("…")
    )
}

/// `GET /admin/crawler` — the live section.
pub async fn page_live(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Err(d) = crate::admin::authorise(&state, peer, &headers) {
        return d.json().into_response();
    }
    axum::response::Html(crate::admin::console("/admin/crawler", &section_live())).into_response()
}

/// `GET /admin/documents` — what has been collected.
pub async fn page_documents(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Err(d) = crate::admin::authorise(&state, peer, &headers) {
        return d.json().into_response();
    }
    axum::response::Html(crate::admin::console(
        "/admin/documents",
        &section_documents(),
    ))
    .into_response()
}

/// `GET /admin` — the overview.
///
/// Answers "is anything wrong" in one screen. Every number that could be *unknown* says so rather
/// than showing zero, because a zero and an unreachable dependency look identical and the second
/// is the one that needs attention.
pub async fn page_overview(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Err(d) = crate::admin::authorise(&state, peer, &headers) {
        return d.json().into_response();
    }

    // The **real** document count, from the index, not from a search.
    //
    // A search reports at most `maxTotalHits`, which is 2000 — so watching a search count to see
    // the crawler working shows a number that stops moving at 2000 while indexing continues
    // perfectly well. That is exactly how "the crawler is not indexing" gets diagnosed wrongly.
    let indexed = match state.search.stats(&state.documents_index()).await {
        Ok(s) => s.number_of_documents.to_string(),
        Err(_) => "unknown".to_string(),
    };

    let snap = snapshot(&state).await;
    let searches =
        state
            .metrics
            .counter_where(crate::metrics::HTTP_REQUESTS, "route", "/api/v1/search");
    let suggests =
        state
            .metrics
            .counter_where(crate::metrics::HTTP_REQUESTS, "route", "/api/v1/suggest");
    let summaries =
        state
            .metrics
            .counter_where(crate::metrics::HTTP_REQUESTS, "route", "/api/v1/summary");
    let tools = state.metrics.counter_total(crate::metrics::INSTANT_ANSWERS);

    let crawl_state = if snap.unavailable {
        "unknown".to_string()
    } else {
        snap.state.clone()
    };

    let body = format!(
        r#"<h1>Overview</h1>
<p class="lede">Everything at a glance. A number that reads <em>unknown</em> means we could not
reach the thing that knows — which is not the same as zero.</p>

<h2>Index</h2>
<div class="tiles">
  <div class="tile"><span class="tile-n">{indexed}</span><span class="tile-l">documents indexed</span></div>
  <div class="tile"><span class="tile-n">{waiting}</span><span class="tile-l">urls queued</span></div>
  <div class="tile"><span class="tile-n">{discovered}</span><span class="tile-l">discovered</span></div>
</div>
<p class="muted">The document count comes from the index itself, not from a search. A search
reports at most 2&nbsp;000 results by design, so watching that number makes a healthy crawl look
stalled the moment it passes the cap.</p>

<h2>Crawler</h2>
<div class="tiles">
  <div class="tile"><span class="tile-n">{crawl_state}</span><span class="tile-l">state</span></div>
  <div class="tile"><span class="tile-n">{fetched}</span><span class="tile-l">fetched</span></div>
  <div class="tile"><span class="tile-n">{failed}</span><span class="tile-l">failed</span></div>
</div>

<h2>Usage</h2>
<p class="muted">Since this process started. Counts only — no queries are recorded, here or
anywhere.</p>
<div class="tiles">
  <div class="tile"><span class="tile-n">{searches}</span><span class="tile-l">searches</span></div>
  <div class="tile"><span class="tile-n">{suggests}</span><span class="tile-l">suggestions</span></div>
  <div class="tile"><span class="tile-n">{tools}</span><span class="tile-l">tool answers</span></div>
  <div class="tile"><span class="tile-n">{summaries}</span><span class="tile-l">summaries</span></div>
</div>
"#,
        waiting = snap.waiting,
        discovered = snap.discovered,
        fetched = snap.fetched,
        failed = snap.failed,
    );

    axum::response::Html(crate::admin::console("/admin", &body)).into_response()
}

// ── Sources ─────────────────────────────────────────────────────────────────────────────────

/// One line of the seed file.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Seed {
    pub source_id: String,
    pub url: String,
    pub trust: String,
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
    let metrics = match stats(state) {
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
    let metrics = match stats(&state) {
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
            "terms": [],
        })));
    }
    let terms = xustive_ingest::weak_coverage::WeakCoverage::connect_in(
        &state.config.queue.url,
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
    Ok(Json(json!({
        "enabled": true,
        "k_anonymity": disc.effective_k(),
        "terms": rows,
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
    pub note: Option<String>,
}

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
        let line = format!("{source_id}\t{}\t{trust}\t{note}\n", safe.as_str());
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

/// `GET /admin/sources` — the page.
pub async fn page_sources(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Err(d) = crate::admin::authorise(&state, peer, &headers) {
        return d.json().into_response();
    }
    let body = r#"<h1>Sources</h1>
<p class="lede">The seed list. Adding one queues it <strong>at the front</strong>, so it is
crawled next rather than behind everything already known.</p>

<form class="admin" id="seed-form">
  <input type="url" id="seed-url" placeholder="https://example.dz/" required>
  <select id="seed-trust">
    <option value="A">A — established, accountable</option>
    <option value="B" selected>B — credible, narrower</option>
    <option value="C">C — user-generated</option>
  </select>
  <button type="submit">Add and crawl next</button>
</form>
<p class="muted" id="seed-msg"></p>

<table class="admin wide"><thead>
  <tr><th>source</th><th>trust</th><th>url</th><th>note</th><th></th></tr>
</thead><tbody id="seed-rows"></tbody></table>

<p class="muted">Removing a source stops it being crawled. Documents already collected from it stay
in the index — those are separate actions, and conflating them would make "stop crawling this"
silently delete part of the index.</p>
"#;
    axum::response::Html(crate::admin::console("/admin/sources", body)).into_response()
}

/// `GET /admin/sources/health` — the per-source quality dashboard (M2-T11.5).
///
/// Rendered client-side from `/admin/crawler/sources/health` so the page loads instantly and the
/// numbers refresh without a full reload. The thresholds coloured here are the §7 healthy bands, so
/// a cell that turns amber is the same signal the lifecycle automation degrades on.
pub async fn page_source_health(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Err(d) = crate::admin::authorise(&state, peer, &headers) {
        return d.json().into_response();
    }
    let body = r#"<h1>Source health</h1>
<p class="lede">Per-source quality, joined from the registry and the live crawl counters. A cell
reads <span class="muted">—</span> until the source has data. Amber marks a value outside its
healthy band (§7) — the same signal the lifecycle automation degrades on.</p>
<p class="muted" id="sh-msg">Loading…</p>
<table class="admin wide"><thead>
  <tr>
    <th>source</th><th>state</th><th>tier</th>
    <th>fetched</th><th>indexed</th>
    <th>fetch ok</th><th>extraction</th><th>duplicate</th><th>spam</th><th>date&nbsp;?</th>
  </tr>
</thead><tbody id="sh-rows"></tbody></table>

<script>
const pct = (v) => v === null || v === undefined ? '<span class="muted">—</span>'
  : (v * 100).toFixed(0) + '%';
// Healthy bands from §7: fetch >95%, extraction >90%, duplicate <30%, spam <0.2, date-unknown <10%.
const band = (v, ok) => v === null || v === undefined ? '' : (ok(v) ? '' : ' class="warn"');
async function load() {
  const msg = document.getElementById('sh-msg');
  try {
    const r = await fetch('/admin/crawler/sources/health', { headers: { 'accept': 'application/json' } });
    if (!r.ok) { msg.textContent = 'Could not load source health (' + r.status + ').'; return; }
    const data = await r.json();
    const rows = data.sources || [];
    const tbody = document.getElementById('sh-rows');
    tbody.innerHTML = '';
    for (const s of rows) {
      const q = s.quality, c = s.counts;
      const name = s.display_name || s.id;
      const tr = document.createElement('tr');
      tr.innerHTML =
        '<td>' + name + ' <span class="muted">' + s.id + '</span></td>' +
        '<td>' + (s.lifecycle || '<span class="muted">—</span>') + '</td>' +
        '<td>' + (s.trust_tier || '—') + '</td>' +
        '<td>' + c.fetched + '</td>' +
        '<td>' + c.indexed + '</td>' +
        '<td' + band(q.fetch_success_rate, v => v > 0.95) + '>' + pct(q.fetch_success_rate) + '</td>' +
        '<td' + band(q.extraction_success_rate, v => v > 0.90) + '>' + pct(q.extraction_success_rate) + '</td>' +
        '<td' + band(q.duplicate_ratio, v => v < 0.30) + '>' + pct(q.duplicate_ratio) + '</td>' +
        '<td' + band(q.spam_mean, v => v < 0.20) + '>' + pct(q.spam_mean) + '</td>' +
        '<td' + band(q.date_unknown_ratio, v => v < 0.10) + '>' + pct(q.date_unknown_ratio) + '</td>';
      tbody.appendChild(tr);
    }
    msg.textContent = rows.length + ' source(s). Refreshing every 10s.';
  } catch (e) { msg.textContent = 'Could not load source health.'; }
}
load();
setInterval(load, 10000);
</script>
"#;
    axum::response::Html(crate::admin::console("/admin/sources/health", body)).into_response()
}

/// `GET /admin/discovery` — the per-channel yield dashboard (M2-T16.8).
pub async fn page_channels(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Err(d) = crate::admin::authorise(&state, peer, &headers) {
        return d.json().into_response();
    }
    let body = r#"<h1>Discovery yield</h1>
<p class="lede">The funnel per discovery channel: how many URLs each introduced, how many were
fetched, and how many survived to an indexed document. <strong>Yield</strong> is indexed ÷
discovered; <strong>unique</strong> is the share of a channel's documents that were not duplicates
of something a cheaper channel already found. This is the number that decides whether an expensive
channel earns its place (M2-T16.8).</p>
<p class="muted" id="ch-msg">Loading…</p>
<table class="admin wide"><thead>
  <tr>
    <th>channel</th><th>discovered</th><th>fetched</th><th>indexed</th>
    <th>duplicate</th><th>yield</th><th>unique</th>
  </tr>
</thead><tbody id="ch-rows"></tbody></table>

<script>
const pct = (v) => v === null || v === undefined ? '<span class="muted">—</span>'
  : (v * 100).toFixed(0) + '%';
async function load() {
  const msg = document.getElementById('ch-msg');
  try {
    const r = await fetch('/admin/crawler/channels', { headers: { 'accept': 'application/json' } });
    if (!r.ok) { msg.textContent = 'Could not load discovery yield (' + r.status + ').'; return; }
    const rows = (await r.json()).channels || [];
    const tbody = document.getElementById('ch-rows');
    tbody.innerHTML = '';
    for (const c of rows) {
      const tr = document.createElement('tr');
      tr.innerHTML =
        '<td>' + c.channel + '</td>' +
        '<td>' + c.discovered + '</td>' +
        '<td>' + c.fetched + '</td>' +
        '<td>' + c.indexed + '</td>' +
        '<td>' + c.duplicate + '</td>' +
        '<td>' + pct(c.yield_rate) + '</td>' +
        '<td>' + pct(c.unique_rate) + '</td>';
      tbody.appendChild(tr);
    }
    msg.textContent = rows.length
      ? rows.length + ' channel(s). Refreshing every 10s.'
      : 'No discovery activity recorded yet.';
  } catch (e) { msg.textContent = 'Could not load discovery yield.'; }
}
load();
setInterval(load, 10000);
</script>
"#;
    axum::response::Html(crate::admin::console("/admin/discovery", body)).into_response()
}

/// `GET /admin/weak-coverage` — the query-driven discovery queue (M2-T16.5).
pub async fn page_weak_coverage(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Err(d) = crate::admin::authorise(&state, peer, &headers) {
        return d.json().into_response();
    }
    let body = r#"<h1>Weak coverage</h1>
<p class="lede">Searches the corpus could not answer — the precise gaps worth finding sources for
(M2-T16.4). Under <a href="https://" onclick="return false">ADR-0008</a> this is <strong>off by
default</strong> and <strong>k-anonymous</strong>: a term appears only once at least
<span id="wc-k">20</span> searches have hit it, so nothing here identifies a query or a person.</p>
<p class="muted" id="wc-msg">Loading…</p>
<table class="admin wide"><thead>
  <tr><th>term</th><th>searches</th></tr>
</thead><tbody id="wc-rows"></tbody></table>
<p class="muted">Resolving a gap to URLs needs an external discovery source (Brave, SERP, Common
Crawl), which is a later task. For now these are the gaps, surfaced for a human to act on.</p>

<script>
async function load() {
  const msg = document.getElementById('wc-msg');
  try {
    const r = await fetch('/admin/crawler/weak-coverage', { headers: { 'accept': 'application/json' } });
    if (!r.ok) { msg.textContent = 'Could not load weak coverage (' + r.status + ').'; return; }
    const data = await r.json();
    document.getElementById('wc-k').textContent = data.k_anonymity;
    const tbody = document.getElementById('wc-rows');
    tbody.innerHTML = '';
    if (!data.enabled) {
      msg.textContent = 'Query-driven discovery is disabled (the default). Enable discovery.weak_coverage_enabled to collect coverage gaps.';
      return;
    }
    const rows = data.terms || [];
    for (const t of rows) {
      const tr = document.createElement('tr');
      const term = document.createElement('td');
      term.textContent = t.term;   // textContent, never innerHTML — the term is user-derived text
      const count = document.createElement('td');
      count.textContent = t.count;
      tr.appendChild(term); tr.appendChild(count);
      tbody.appendChild(tr);
    }
    msg.textContent = rows.length
      ? rows.length + ' coverage gap(s), each searched ≥ ' + data.k_anonymity + ' times.'
      : 'No coverage gaps have crossed the k-anonymity floor yet.';
  } catch (e) { msg.textContent = 'Could not load weak coverage.'; }
}
load();
setInterval(load, 15000);
</script>
"#;
    axum::response::Html(crate::admin::console("/admin/weak-coverage", body)).into_response()
}
