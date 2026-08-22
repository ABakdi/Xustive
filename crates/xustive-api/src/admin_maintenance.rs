//! Destructive maintenance actions, from the console (M4-T09.3).
//!
//! Right now: **takedown** — remove every already-indexed document for a domain across the lexical
//! index, the image vectors, and the raw stored bodies. It mirrors `xustive-cli takedown` so the
//! operator has one auditable action instead of three manual store edits. Guarded: the caller must
//! echo the exact domain as `confirm`, the same way the CLI requires `--yes`.
//!
//! It does not stop *future* crawling — pair it with disabling the source. That is stated in the
//! response and in the UI.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::admin::Peer;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct TakedownRequest {
    pub domain: String,
    /// Must equal `domain` — a typed confirmation, so a stray click cannot delete content.
    pub confirm: String,
    /// When false (default), report what *would* be removed without deleting.
    #[serde(default)]
    pub execute: bool,
}

/// `POST /admin/takedown` — preview or execute a domain takedown.
pub async fn takedown(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
    Json(req): Json<TakedownRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;

    let domain = req.domain.trim();
    if domain.is_empty() {
        return Err(bad("empty_domain", "a domain is required"));
    }
    if req.execute && req.confirm.trim() != domain {
        return Err(bad(
            "confirm_mismatch",
            "type the exact domain to confirm the takedown",
        ));
    }

    // Find every document for this domain — one filtered query, paged to the end.
    let filter = format!("domain = \"{}\"", domain.replace('"', "\\\""));
    let index = state.documents_index();
    let mut targets: Vec<(String, String)> = Vec::new();
    let mut offset = 0usize;
    const PAGE: usize = 1000;
    loop {
        let query = xustive_search::Query::new("")
            .filter(filter.clone())
            .offset(offset)
            .limit(PAGE);
        let hits = match state.search.search::<Value>(&index, &query).await {
            Ok(h) => h,
            Err(e) => return Err(bad("search_failed", &e.to_string())),
        };
        if hits.hits.is_empty() {
            break;
        }
        for h in &hits.hits {
            if let Some(id) = h.get("id").and_then(Value::as_str) {
                let url = h
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                targets.push((id.to_string(), url));
            }
        }
        offset += PAGE;
    }

    if !req.execute {
        return Ok(Json(json!({
            "domain": domain,
            "matched": targets.len(),
            "executed": false,
            "note": "preview only — resubmit with execute:true and confirm:<domain> to delete",
        })));
    }

    // Delete across every store. Optional stores absent is fine — a document simply had none there.
    let raw = xustive_ingest::raw_store::RawStore::connect_in(
        &state.config.queue.url,
        "frontier",
        std::time::Duration::from_secs(1),
    );
    let (mut docs, mut vecs, mut bodies) = (0u64, 0u64, 0u64);
    for (id, url) in &targets {
        if state.search.delete_document(&index, id).await.is_ok() {
            docs += 1;
        }
        if let Some(engine) = &state.image_search {
            if engine.store.delete_by_document(id).await.is_ok() {
                vecs += 1;
            }
        }
        if let (Some(rs), false) = (&raw, url.is_empty()) {
            rs.forget(url).await;
            bodies += 1;
        }
    }
    tracing::warn!(%domain, docs, vecs, bodies, "domain takedown executed by operator");

    Ok(Json(json!({
        "domain": domain,
        "executed": true,
        "documents_removed": docs,
        "vector_groups_removed": vecs,
        "raw_bodies_removed": bodies,
        "note": "future crawling is NOT blocked — also disable the source to prevent re-indexing",
    })))
}

fn bad(code: &str, message: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error": {"code": code, "message": message}})),
    )
}
