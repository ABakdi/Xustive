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
  <div class="tile"><span class="tile-n" id="c-discovered">–</span><span class="tile-l">discovered</span></div>
  <div class="tile"><span class="tile-n" id="c-waiting">–</span><span class="tile-l">queued</span></div>
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
  <tr><th>title</th><th>domain</th><th>language</th><th>excerpt</th><th>published</th></tr>
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
