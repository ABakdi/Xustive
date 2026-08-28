//! First-party search events ([[ADR-0030 - First-Party Search Data, Kept to Learn From]], M11).
//!
//! Every search, every opened result and every "not relevant" report becomes one document in the
//! `events` index, with the visitor and session ids our own cookies carry. This is the raw
//! material for everything the [[Privacy Relaxation Audit]] ranks first — spelling, autocomplete
//! and synonyms from what people type, evaluation on real traffic, a ranker that learns — and the
//! record the operator asked for: which documents readers actually opened.
//!
//! # Never on the critical path
//!
//! A search must not wait for its own bookkeeping. Events go into a bounded channel and one
//! writer task drains it in batches into Meilisearch; when the channel is full the event is
//! dropped and counted, never blocked on. The same task applies the per-document counters
//! (`hits.opens`, `hits.reports`) as partial updates — the index merges them — so a document
//! remembers being opened without a read first.
//!
//! # What is not here
//!
//! No IP address, no user agent, no device. The query text goes into the *store* (that is the
//! point) and still never into a log line: `scripts/lint-telemetry.sh` forbids it there, and the
//! writer logs counts only. The click and report beacons carry a token, never the query — the
//! server resolves it to the query it kept in memory when it answered the search.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use xustive_search::settings::EVENTS;
use xustive_search::MeiliClient;

use crate::state::AppState;

/// One event. `kind` is `search`, `click` or `report`; the fields a kind does not use are absent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub id: String,
    pub kind: String,
    /// Unix seconds.
    pub at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visitor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    pub query: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub normalized: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertical: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_hits: Option<u64>,
    /// Document ids shown, in rank order (search events).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shown: Vec<String>,
    /// The document opened or reported (click/report events), and its rank on the page shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
}

impl Event {
    fn new(kind: &str, query: &str) -> Self {
        Self {
            id: ulid::Ulid::new().to_string(),
            kind: kind.to_string(),
            at: now(),
            visitor: None,
            session: None,
            query: query.to_string(),
            normalized: String::new(),
            ui: None,
            lang: None,
            vertical: None,
            page: None,
            total_hits: None,
            shown: Vec::new(),
            doc: None,
            rank: None,
            reason: None,
            latency_ms: None,
        }
    }
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The visitor and session ids from our own cookies, `xv` and `xs`. Anything that is not a
/// ULID-shaped token is ignored: the cookie is ours, but the request is anyone's.
pub fn cookie_ids(headers: &HeaderMap) -> (Option<String>, Option<String>) {
    let mut visitor = None;
    let mut session = None;
    for value in headers.get_all(axum::http::header::COOKIE) {
        let Ok(text) = value.to_str() else { continue };
        for pair in text.split(';') {
            let pair = pair.trim();
            if let Some(v) = pair.strip_prefix("xv=") {
                if is_token(v) {
                    visitor = Some(v.to_string());
                }
            } else if let Some(v) = pair.strip_prefix("xs=") {
                if is_token(v) {
                    session = Some(v.to_string());
                }
            }
        }
    }
    (visitor, session)
}

fn is_token(v: &str) -> bool {
    v.len() == 26 && v.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// What the writer task receives.
enum Job {
    Event(Event),
    /// `hits.opens` / `hits.reports` on a document: +1 and, for opens, the time.
    Hit {
        doc: String,
        opened: bool,
    },
}

/// The sink the handlers write to. Cheap to clone; the channel is bounded.
#[derive(Clone)]
pub struct EventSink {
    tx: mpsc::Sender<Job>,
    pub dropped: Arc<AtomicU64>,
    pub written: Arc<AtomicU64>,
}

const QUEUE: usize = 4096;
const BATCH: usize = 200;
const FLUSH_EVERY: Duration = Duration::from_secs(2);

impl EventSink {
    /// Start the writer task. The index alias is resolved once per flush, so an alias swap is
    /// followed without a restart.
    pub fn start(client: Arc<MeiliClient>) -> Self {
        let (tx, rx) = mpsc::channel(QUEUE);
        let sink = Self {
            tx,
            dropped: Arc::new(AtomicU64::new(0)),
            written: Arc::new(AtomicU64::new(0)),
        };
        tokio::spawn(writer(client, rx, sink.written.clone()));
        sink
    }

    fn push(&self, job: Job) {
        if self.tx.try_send(job).is_err() {
            // Full channel: the search is more important than its record.
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn search(
        &self,
        headers: &HeaderMap,
        query: &str,
        normalized: &str,
        ui: &str,
        lang: Option<&str>,
        vertical: Option<&str>,
        page: u32,
        total_hits: u64,
        shown: Vec<String>,
        latency_ms: u64,
    ) {
        let (visitor, session) = cookie_ids(headers);
        let mut e = Event::new("search", query);
        e.visitor = visitor;
        e.session = session;
        e.normalized = normalized.to_string();
        e.ui = Some(ui.to_string());
        e.lang = lang.map(str::to_string);
        e.vertical = Some(vertical.unwrap_or("all").to_string());
        e.page = Some(page);
        e.total_hits = Some(total_hits);
        e.shown = shown;
        e.latency_ms = Some(latency_ms);
        self.push(Job::Event(e));
    }

    /// A click or a report on a result. `reason` is `None` for a click.
    pub fn hit(
        &self,
        headers: &HeaderMap,
        query: &str,
        doc: &str,
        rank: Option<u32>,
        reason: Option<&str>,
    ) {
        let (visitor, session) = cookie_ids(headers);
        let mut e = Event::new(if reason.is_some() { "report" } else { "click" }, query);
        e.visitor = visitor;
        e.session = session;
        e.doc = Some(doc.to_string());
        e.rank = rank;
        e.reason = reason.map(str::to_string);
        self.push(Job::Event(e));
        self.push(Job::Hit {
            doc: doc.to_string(),
            opened: reason.is_none(),
        });
    }
}

async fn writer(client: Arc<MeiliClient>, mut rx: mpsc::Receiver<Job>, written: Arc<AtomicU64>) {
    let mut events: Vec<Event> = Vec::new();
    // doc -> (opens, reports)
    let mut hits: HashMap<String, (u32, u32)> = HashMap::new();
    let mut tick = tokio::time::interval(FLUSH_EVERY);
    loop {
        tokio::select! {
            job = rx.recv() => match job {
                Some(Job::Event(e)) => events.push(e),
                Some(Job::Hit { doc, opened }) => {
                    let e = hits.entry(doc).or_insert((0, 0));
                    if opened { e.0 += 1 } else { e.1 += 1 }
                }
                None => break,
            },
            _ = tick.tick() => {}
        }
        if events.len() >= BATCH
            || (!events.is_empty() && tick.period() == FLUSH_EVERY)
            || !hits.is_empty()
        {
            flush(&client, &mut events, &mut hits, &written).await;
        }
    }
    flush(&client, &mut events, &mut hits, &written).await;
}

async fn flush(
    client: &MeiliClient,
    events: &mut Vec<Event>,
    hits: &mut HashMap<String, (u32, u32)>,
    written: &AtomicU64,
) {
    if !events.is_empty() {
        match client.resolve(EVENTS).await {
            Ok(index) => match client.add_documents(&index, events).await {
                Ok(_) => {
                    written.fetch_add(events.len() as u64, Ordering::Relaxed);
                }
                Err(e) => {
                    tracing::warn!(error = %e, n = events.len(), "event write failed; events dropped")
                }
            },
            Err(e) => tracing::warn!(error = %e, "events index unresolved; events dropped"),
        }
        events.clear();
    }
    if !hits.is_empty() {
        // Counters need the current value: one filtered read for the batch, then merged updates.
        let ids: Vec<String> = hits.keys().cloned().collect();
        let index = client
            .resolve(xustive_search::settings::DOCUMENTS)
            .await
            .ok();
        if let Some(index) = index {
            let quoted: Vec<String> = ids.iter().map(|i| format!("\"{i}\"")).collect();
            let q = xustive_search::Query::new("")
                .filter(format!("id IN [{}]", quoted.join(", ")))
                .limit(ids.len().max(1));
            let current: HashMap<String, (u64, u64)> =
                match client.search::<Value>(&index, &q).await {
                    Ok(r) => r
                        .hits
                        .iter()
                        .filter_map(|h| {
                            let id = h.get("id")?.as_str()?.to_string();
                            let o = h
                                .pointer("/hits/opens")
                                .and_then(Value::as_u64)
                                .unwrap_or(0);
                            let r = h
                                .pointer("/hits/reports")
                                .and_then(Value::as_u64)
                                .unwrap_or(0);
                            Some((id, (o, r)))
                        })
                        .collect(),
                    Err(e) => {
                        tracing::warn!(error = %e, "hit counters: read failed; batch dropped");
                        hits.clear();
                        return;
                    }
                };
            let at = now();
            let updates: Vec<Value> = hits
                .iter()
                .filter(|(id, _)| current.contains_key(*id))
                .map(|(id, (o, r))| {
                    let (co, cr) = current.get(id).copied().unwrap_or((0, 0));
                    let mut h = json!({ "opens": co + *o as u64, "reports": cr + *r as u64 });
                    if *o > 0 {
                        h["last_opened_at"] = json!(at);
                    }
                    json!({ "id": id, "hits": h })
                })
                .collect();
            if !updates.is_empty() {
                if let Err(e) = client.update_documents(&index, &updates).await {
                    tracing::warn!(error = %e, "hit counters: write failed");
                }
            }
        }
        hits.clear();
    }
}

// ---------------------------------------------------------------------------------------------
// The report beacon

#[derive(Deserialize)]
pub struct ReportBeacon {
    /// The search→click token from the search response.
    pub t: String,
    /// The document reported.
    pub d: String,
    /// The reason; only `irrelevant` exists (M11-T03).
    #[serde(default)]
    pub r: String,
}

/// `POST /api/v1/report` — a reader says a result is not relevant. Always 204.
pub async fn report(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Option<Json<ReportBeacon>>,
) -> StatusCode {
    let Some(Json(b)) = body else {
        return StatusCode::NO_CONTENT;
    };
    let Some(sink) = state.events_if_on().cloned() else {
        return StatusCode::NO_CONTENT;
    };
    if b.d.is_empty() || b.r != "irrelevant" {
        return StatusCode::NO_CONTENT;
    }
    if let Some((query, rank)) = state.token_context(&b.t, &b.d) {
        sink.hit(&headers, &query, &b.d, rank, Some("irrelevant"));
    }
    StatusCode::NO_CONTENT
}

// ---------------------------------------------------------------------------------------------
// Admin: the overview an operator acts on

#[derive(Deserialize)]
pub struct OverviewParams {
    #[serde(default)]
    pub days: Option<u32>,
}

/// `GET /api/v1/admin/events/overview?days=7`.
pub async fn overview(
    State(state): State<AppState>,
    crate::admin::Peer(peer): crate::admin::Peer,
    headers: HeaderMap,
    axum::extract::Query(p): axum::extract::Query<OverviewParams>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;
    let Some(sink) = state.events_if_on() else {
        return Ok(Json(json!({ "enabled": false })));
    };
    let days = p.days.unwrap_or(7).clamp(1, 365);
    let since = now() - days as i64 * 86_400;
    let index = state.search.resolve(EVENTS).await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": e.to_string() })),
        )
    })?;

    // One scan of the window, in pages, aggregated in memory: the window is bounded by
    // retention and the page cap, and an operator's overview is not a per-request path.
    let mut searches: Vec<Event> = Vec::new();
    let mut clicks: Vec<Event> = Vec::new();
    let mut reports: Vec<Event> = Vec::new();
    let mut offset = 0usize;
    const PAGE: usize = 1000;
    const MAX: usize = 50_000;
    loop {
        let q = xustive_search::Query::new("")
            .filter(format!("at >= {since}"))
            .sort(&["at:desc"])
            .limit(PAGE)
            .offset(offset);
        let r = state
            .search
            .search::<Event>(&index, &q)
            .await
            .map_err(|e| {
                (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({ "error": e.to_string() })),
                )
            })?;
        let n = r.hits.len();
        for e in r.hits {
            match e.kind.as_str() {
                "search" => searches.push(e),
                "click" => clicks.push(e),
                "report" => reports.push(e),
                _ => {}
            }
        }
        offset += n;
        if n < PAGE || offset >= MAX {
            break;
        }
    }

    // Per query: searches, results, clicks.
    let mut by_query: HashMap<String, (u32, u64, u32)> = HashMap::new();
    for s in &searches {
        let e = by_query.entry(s.normalized.clone()).or_insert((0, 0, 0));
        e.0 += 1;
        e.1 = e.1.max(s.total_hits.unwrap_or(0));
    }
    for c in &clicks {
        let key = xustive_text::normalize(&c.query);
        if let Some(e) = by_query.get_mut(&key) {
            e.2 += 1;
        }
    }
    let mut zero: Vec<(&String, u32)> = by_query
        .iter()
        .filter(|(_, v)| v.1 == 0)
        .map(|(k, v)| (k, v.0))
        .collect();
    zero.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let mut unopened: Vec<(&String, u32, u64)> = by_query
        .iter()
        .filter(|(_, v)| v.1 > 0 && v.2 == 0 && v.0 >= 2)
        .map(|(k, v)| (k, v.0, v.1))
        .collect();
    unopened.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let mut top: Vec<(&String, u32, u64, u32)> =
        by_query.iter().map(|(k, v)| (k, v.0, v.1, v.2)).collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

    // Per document: opens and reports, with the queries that led there.
    let mut opened: HashMap<String, (u32, u32, HashMap<String, u32>)> = HashMap::new();
    for c in &clicks {
        if let Some(d) = &c.doc {
            let e = opened.entry(d.clone()).or_insert((0, 0, HashMap::new()));
            e.0 += 1;
            *e.2.entry(c.query.clone()).or_default() += 1;
        }
    }
    for r in &reports {
        if let Some(d) = &r.doc {
            let e = opened.entry(d.clone()).or_insert((0, 0, HashMap::new()));
            e.1 += 1;
            *e.2.entry(r.query.clone()).or_default() += 1;
        }
    }
    let mut most_opened: Vec<(&String, &(u32, u32, HashMap<String, u32>))> =
        opened.iter().filter(|(_, v)| v.0 > 0).collect();
    most_opened.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
    let mut reported: Vec<(&String, &(u32, u32, HashMap<String, u32>))> =
        opened.iter().filter(|(_, v)| v.1 > 0).collect();
    reported.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));

    let ids: Vec<String> = most_opened
        .iter()
        .take(30)
        .chain(reported.iter().take(50))
        .map(|(d, _)| (*d).clone())
        .collect();
    let titles = crate::admin::resolve_doc_titles(&state, &ids).await;
    let doc_row = |d: &String, v: &(u32, u32, HashMap<String, u32>)| {
        let mut qs: Vec<(&String, &u32)> = v.2.iter().collect();
        qs.sort_by(|a, b| b.1.cmp(a.1));
        let t = titles.get(d);
        json!({
            "doc": d,
            "title": t.map(|x| x.0.clone()),
            "url": t.map(|x| x.1.clone()),
            "opens": v.0,
            "reports": v.1,
            "queries": qs.iter().take(3).map(|(q, n)| json!({ "query": q, "count": n })).collect::<Vec<_>>(),
        })
    };

    let clicked_searches = searches
        .iter()
        .filter(|s| {
            s.shown.iter().any(|d| {
                clicks
                    .iter()
                    .any(|c| c.doc.as_deref() == Some(d) && c.query == s.query)
            })
        })
        .count();
    let mut recent: Vec<&Event> = searches
        .iter()
        .chain(clicks.iter())
        .chain(reports.iter())
        .collect();
    recent.sort_by(|a, b| b.at.cmp(&a.at));

    Ok(Json(json!({
        "enabled": true,
        "days": days,
        "retention_days": state.config.collection.retention_days,
        "written": sink.written.load(Ordering::Relaxed),
        "dropped": sink.dropped.load(Ordering::Relaxed),
        "totals": {
            "searches": searches.len(),
            "clicks": clicks.len(),
            "reports": reports.len(),
            "distinct_queries": by_query.len(),
            "zero_result_searches": searches.iter().filter(|s| s.total_hits == Some(0)).count(),
            "searches_with_a_click": clicked_searches,
            "visitors": searches.iter().filter_map(|s| s.visitor.as_ref()).collect::<std::collections::HashSet<_>>().len(),
        },
        "zero_results": zero.iter().take(50).map(|(q, n)| json!({ "query": q, "count": n })).collect::<Vec<_>>(),
        "unopened": unopened.iter().take(50).map(|(q, n, h)| json!({ "query": q, "count": n, "results": h })).collect::<Vec<_>>(),
        "top_queries": top.iter().take(50).map(|(q, n, h, c)| json!({ "query": q, "count": n, "results": h, "clicks": c })).collect::<Vec<_>>(),
        "most_opened": most_opened.iter().take(30).map(|(d, v)| doc_row(d, v)).collect::<Vec<_>>(),
        "reported": reported.iter().take(50).map(|(d, v)| doc_row(d, v)).collect::<Vec<_>>(),
        "recent": recent.iter().take(100).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
pub struct VisitorParams {
    pub visitor: String,
}

/// `GET /api/v1/admin/events/visitor?visitor=` — every event of one visitor id.
pub async fn visitor(
    State(state): State<AppState>,
    crate::admin::Peer(peer): crate::admin::Peer,
    headers: HeaderMap,
    axum::extract::Query(p): axum::extract::Query<VisitorParams>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;
    if !is_token(&p.visitor) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "not a visitor id" })),
        ));
    }
    let index = state.search.resolve(EVENTS).await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": e.to_string() })),
        )
    })?;
    let q = xustive_search::Query::new("")
        .filter(format!("visitor = \"{}\"", p.visitor))
        .sort(&["at:desc"])
        .limit(500);
    let r = state
        .search
        .search::<Event>(&index, &q)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": e.to_string() })),
            )
        })?;
    Ok(Json(json!({ "visitor": p.visitor, "events": r.hits })))
}

/// `POST /api/v1/admin/events/forget` `{visitor}` — the right to be forgotten, operable.
pub async fn forget(
    State(state): State<AppState>,
    crate::admin::Peer(peer): crate::admin::Peer,
    headers: HeaderMap,
    Json(p): Json<VisitorParams>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;
    if !is_token(&p.visitor) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "not a visitor id" })),
        ));
    }
    let n = forget_visitor(&state.search, &p.visitor)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))))?;
    tracing::info!(deleted = n, "visitor events forgotten");
    Ok(Json(json!({ "visitor": p.visitor, "deleted": n })))
}

pub use xustive_search::events::{forget_visitor, sweep};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cookies_are_read_and_anything_else_is_ignored() {
        let mut h = HeaderMap::new();
        h.insert(
            axum::http::header::COOKIE,
            "theme=dark; xv=01ARZ3NDEKTSV4RRFFQ69G5FAV; xs=01ARZ3NDEKTSV4RRFFQ69G5FAW; other=1"
                .parse()
                .unwrap(),
        );
        let (v, s) = cookie_ids(&h);
        assert_eq!(v.as_deref(), Some("01ARZ3NDEKTSV4RRFFQ69G5FAV"));
        assert_eq!(s.as_deref(), Some("01ARZ3NDEKTSV4RRFFQ69G5FAW"));
        let mut bad = HeaderMap::new();
        bad.insert(
            axum::http::header::COOKIE,
            "xv=<script>; xs=short".parse().unwrap(),
        );
        assert_eq!(cookie_ids(&bad), (None, None));
    }

    #[test]
    fn an_event_carries_no_address_and_only_its_kinds_fields() {
        let mut e = Event::new("click", "wach rak");
        e.doc = Some("d1".into());
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], "click");
        assert_eq!(v["doc"], "d1");
        assert!(v.get("shown").is_none());
        assert!(v.get("ip").is_none() && v.get("user_agent").is_none());
        let back: Event = serde_json::from_value(v).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn the_report_beacon_takes_a_token_a_doc_and_a_reason_and_nothing_else() {
        let b: ReportBeacon =
            serde_json::from_str(r#"{"t":"x","d":"y","r":"irrelevant","query":"smuggled"}"#)
                .unwrap();
        assert_eq!(
            (b.t.as_str(), b.d.as_str(), b.r.as_str()),
            ("x", "y", "irrelevant")
        );
    }
}
