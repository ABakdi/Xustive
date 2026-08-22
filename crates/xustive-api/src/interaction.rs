//! The interaction (click) endpoint ([[Interaction Signals]], M6-T03).
//!
//! The browser beacons `{t, d}` here when a result link is clicked: `t` is the opaque token minted
//! in the search response, `d` is the clicked document id. The handler resolves the token to the
//! query's hash **in memory** and records the click — so the click request carries **no query
//! text**, and the server holds no per-person record. It always answers `204`, revealing nothing
//! about whether the token was valid (a probe learns nothing).
//!
//! # Privacy invariants
//!
//! - The request body has no query, no identifier — only an opaque token and a document id.
//! - `token` is a forbidden telemetry field name, so it can never be logged (the telemetry lint
//!   enforces this).
//! - When interaction signals are disabled, the endpoint is a silent no-op that still returns 204.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use crate::state::AppState;

#[derive(Deserialize)]
pub struct ClickBeacon {
    /// The opaque search→click token (`interaction_token` from the search response).
    pub t: String,
    /// The clicked document id.
    pub d: String,
}

/// `POST /api/v1/interaction` — record an anonymous click. Always 204.
pub async fn handler(State(state): State<AppState>, body: Option<Json<ClickBeacon>>) -> StatusCode {
    // A malformed or missing body is not worth distinguishing from a valid one — 204 regardless, so
    // the endpoint leaks nothing about what it accepted.
    let Some(Json(beacon)) = body else {
        return StatusCode::NO_CONTENT;
    };
    let Some(store) = state.interactions() else {
        return StatusCode::NO_CONTENT;
    };

    // Resolve the token to a query hash in memory. Unknown or expired → nothing recorded, still 204.
    let qhash = state
        .interaction_tokens
        .read()
        .ok()
        .and_then(|m| m.get(&beacon.t).map(|(qh, _)| qh.clone()));

    if let Some(qh) = qhash {
        // A document id is a bounded, non-identifying value; guard only against an empty one.
        if !beacon.d.is_empty() {
            store.click_by_qhash(&qh, &beacon.d).await;
        }
    }
    StatusCode::NO_CONTENT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beacon_decodes_the_minimal_shape() {
        let b: ClickBeacon = serde_json::from_str(r#"{"t":"01ABC","d":"doc1"}"#).unwrap();
        assert_eq!(b.t, "01ABC");
        assert_eq!(b.d, "doc1");
    }

    #[test]
    fn beacon_has_no_query_field() {
        // The shape is deliberately query-free. A body carrying a query is simply ignored (serde
        // drops unknown fields), so a client cannot smuggle query text through this endpoint.
        let b: ClickBeacon = serde_json::from_str(r#"{"t":"x","d":"y","query":"secret"}"#).unwrap();
        assert_eq!(b.t, "x");
    }
}
