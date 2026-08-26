//! The index-queue section of the admin console.
//!
//! Surfaces what `make dlq` shows on the command line — the backlog waiting to be indexed and the
//! dead letters the indexer gave up on — plus the one control that matters: replaying the dead
//! letters once their cause is fixed. Read-mostly; replay is deliberate and confirmed in the UI.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde_json::json;

use crate::admin::Peer;
use crate::state::AppState;

/// Connect to the index queue for a one-off admin read. Low-frequency (an operator page), so a
/// per-request connect is fine — unlike the once-a-second Live stream.
async fn queue(state: &AppState) -> Option<xustive_queue::Queue> {
    xustive_queue::Queue::connect(
        &state.config.queue.url,
        &state.config.queue.index_stream,
        xustive_queue::INDEXER_GROUP,
    )
    .await
    .ok()
}

/// `GET /admin/queue` — backlog depth, dead-letter count, and a peek at the dead letters.
pub async fn status(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;

    let Some(q) = queue(&state).await else {
        return Ok(Json(json!({ "unavailable": true })));
    };

    let depth = q.depth_of(xustive_queue::INDEXER_GROUP).await.unwrap_or(0);
    let dead_count = q.dead_count().await.unwrap_or(0);
    // The capacity alarm (PROB-001): Redis memory against its ceiling, and the frontier's size.
    // The one signal whose absence let the last OOM arrive with no warning anywhere in admin.
    let capacity = match xustive_ingest::frontier::Frontier::connect(&state.config.queue.url) {
        Ok(f) => {
            let memory = f.memory_usage().await;
            let (frontier_waiting, _) = f.depth().await;
            let deferred = f.deferred().await;
            json!({
                "redis_used_bytes": memory.map(|(u, _)| u),
                "redis_max_bytes": memory.map(|(_, m)| m),
                "redis_pct": memory
                    .filter(|(_, m)| *m > 0)
                    .map(|(u, m)| (u as f64 / m as f64 * 100.0).round() as u64),
                "frontier_waiting": frontier_waiting,
                "frontier_deferred": deferred,
            })
        }
        Err(_) => json!(null),
    };
    let dead = q.peek_dead_with_ids(20).await.unwrap_or_default();
    let dead_json: Vec<serde_json::Value> = dead
        .iter()
        .map(|(entry_id, d)| {
            // The payload is a crawled document; show just enough to recognise it, never the body.
            let url = d
                .payload
                .get("document")
                .and_then(|doc| doc.get("url"))
                .and_then(|v| v.as_str())
                .or_else(|| d.payload.get("url").and_then(|v| v.as_str()))
                .unwrap_or("");
            json!({ "entry_id": entry_id, "url": url, "attempts": d.attempts, "reason": d.reason, "failed_at": d.failed_at })
        })
        .collect();

    Ok(Json(json!({
        "unavailable": false,
        "backlog": depth,
        "dead_count": dead_count,
        "dead": dead_json,
        "capacity": capacity,
    })))
}

/// `POST /admin/queue/replay` — re-enqueue the dead letters (after their cause is fixed).
pub async fn replay(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;

    let Some(q) = queue(&state).await else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(
                json!({"error": {"code": "queue_unavailable", "message": "cannot reach the queue"}}),
            ),
        ));
    };
    match q.replay_dead(1000).await {
        Ok(n) => {
            tracing::warn!(replayed = n, "dead letters replayed by operator");
            Ok(Json(json!({ "replayed": n })))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": {"code": "replay_failed", "message": e.to_string()}})),
        )),
    }
}

#[derive(serde::Deserialize)]
pub struct DeadItem {
    pub entry_id: String,
}

/// Per-item dead-letter actions (PROB-003): the all-or-nothing replay is right when one bug killed
/// a batch, wrong when one poisoned job sits among salvageable ones. Both take the stream entry id
/// the status endpoint now returns, and both answer `found: false` for an id that is already gone
/// rather than erroring — the page may be a poll behind reality.
async fn dead_action(
    state: &AppState,
    entry_id: &str,
    act: &str,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if entry_id.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"code": "missing_entry_id", "message": "entry_id is required"}})),
        ));
    }
    let Some(q) = queue(state).await else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(
                json!({"error": {"code": "queue_unavailable", "message": "cannot reach the queue"}}),
            ),
        ));
    };
    let outcome = match act {
        "replay" => q.replay_dead_one(entry_id.trim()).await,
        _ => q.drop_dead(entry_id.trim()).await,
    };
    match outcome {
        Ok(found) => Ok(Json(json!({ "ok": true, "found": found }))),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": {"code": "dead_action_failed", "message": e.to_string()}})),
        )),
    }
}

/// `POST /admin/queue/dead/replay` — put one dead letter back on the queue.
pub async fn replay_one(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
    Json(req): Json<DeadItem>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;
    let out = dead_action(&state, &req.entry_id, "replay").await?;
    tracing::warn!(entry_id = %req.entry_id, "one dead letter replayed by operator");
    Ok(out)
}

/// `POST /admin/queue/dead/drop` — discard one dead letter for good. The only deliberate discard
/// in the queue; the UI confirms it, and the log keeps the trace.
pub async fn drop_one(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
    Json(req): Json<DeadItem>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;
    let out = dead_action(&state, &req.entry_id, "drop").await?;
    tracing::warn!(entry_id = %req.entry_id, "one dead letter dropped by operator");
    Ok(out)
}
