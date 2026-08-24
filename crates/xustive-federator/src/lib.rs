//! The Federation Gateway ([[Federation Gateway]] C31, [[ADR-0017]], M7-T04).
//!
//! The one narrow, allowlisted egress hop between the serving plane and the open web. The serving
//! API — which has no route to the internet — reaches this over the internal `core` network; this,
//! dual-homed onto the egress network, reaches a self-hosted SearXNG. So "the serving plane cannot
//! reach the internet" survives as "the serving plane can reach one internal gateway, which holds
//! the only outbound client."
//!
//! It is deliberately tiny and stateless: accept a query, ask SearXNG within a budget, return the
//! normalised hits. No index, no durable store, no ranking — the caller blends and caps. Fail-open
//! is the contract: a slow, dead, or unconfigured SearXNG returns an empty (possibly `partial`)
//! answer, never an error, so federation can never break or slow the local result.
//!
//! No query text is ever logged (the [[Observability|telemetry lint]] keeps `query` a forbidden
//! field name); the only outbound target is SearXNG.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use xustive_federation::{FederatedHit, SearxngClient};

/// Shared gateway state: the SearXNG client (absent when unconfigured) and the default budget.
#[derive(Clone)]
pub struct AppState {
    /// `None` when no endpoint is configured — the gateway then answers empty, staying inert exactly
    /// as the client does, rather than erroring.
    pub client: Option<Arc<SearxngClient>>,
    /// Applied when a request does not carry its own `budget_ms`.
    pub default_budget: Duration,
}

/// `POST /federate` request. `budget_ms` lets the caller (the serving pipeline) impose a tighter
/// bound than the gateway's default — it is the caller's latency budget, enforced here too so a slow
/// SearXNG cannot outlast it.
#[derive(Debug, Deserialize)]
pub struct FederateRequest {
    pub query: String,
    #[serde(default)]
    pub budget_ms: Option<u64>,
}

/// `POST /federate` response. `partial` is true when the budget cut the call short or SearXNG
/// errored — the caller ships what it has (index-only when empty) and never waits further.
#[derive(Debug, Serialize)]
pub struct FederateResponse {
    pub hits: Vec<FederatedHit>,
    pub partial: bool,
}

/// The router: a health check and the one federation route.
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/federate", post(federate))
        .with_state(state)
}

async fn healthz() -> &'static str {
    "ok"
}

async fn federate(
    State(state): State<AppState>,
    Json(req): Json<FederateRequest>,
) -> Json<FederateResponse> {
    Json(federate_inner(&state, req).await)
}

/// The gateway logic, separated from the axum extractors so it is unit-testable without a server.
pub async fn federate_inner(state: &AppState, req: FederateRequest) -> FederateResponse {
    let query = req.query.trim();
    if query.is_empty() {
        return FederateResponse {
            hits: Vec::new(),
            partial: false,
        };
    }
    // Unconfigured is inert, not an error — the same fail-open the client itself takes.
    let Some(client) = state.client.clone() else {
        return FederateResponse {
            hits: Vec::new(),
            partial: false,
        };
    };
    let budget = req
        .budget_ms
        .map(Duration::from_millis)
        .unwrap_or(state.default_budget);

    match tokio::time::timeout(budget, client.search(query)).await {
        Ok(Ok(hits)) => FederateResponse {
            hits,
            partial: false,
        },
        Ok(Err(e)) => {
            // Note: `e` is a FederationError — it carries a status or reqwest error, never the query.
            tracing::warn!(error = %e, "searxng call failed; returning empty");
            FederateResponse {
                hits: Vec::new(),
                partial: true,
            }
        }
        Err(_) => {
            tracing::warn!(
                budget_ms = budget.as_millis() as u64,
                "searxng call exceeded budget; returning empty"
            );
            FederateResponse {
                hits: Vec::new(),
                partial: true,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unconfigured() -> AppState {
        AppState {
            client: None,
            default_budget: Duration::from_millis(250),
        }
    }

    #[tokio::test]
    async fn an_empty_query_returns_empty_and_not_partial() {
        let r = federate_inner(
            &unconfigured(),
            FederateRequest {
                query: "   ".into(),
                budget_ms: None,
            },
        )
        .await;
        assert!(r.hits.is_empty());
        assert!(!r.partial); // nothing was attempted, so this is not a truncated answer
    }

    #[tokio::test]
    async fn an_unconfigured_gateway_is_inert_not_partial() {
        // No endpoint configured is a deployment choice, not a failed call — empty and complete.
        let r = federate_inner(
            &unconfigured(),
            FederateRequest {
                query: "قسنطينة".into(),
                budget_ms: Some(100),
            },
        )
        .await;
        assert!(r.hits.is_empty());
        assert!(!r.partial);
    }

    #[test]
    fn the_response_serialises_hits_and_partial() {
        let resp = FederateResponse {
            hits: vec![FederatedHit {
                url: "https://example.dz/a".into(),
                title: "A".into(),
                snippet: "s".into(),
                engine: "duckduckgo".into(),
                rank: 1,
            }],
            partial: true,
        };
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["partial"], true);
        assert_eq!(v["hits"][0]["url"], "https://example.dz/a");
        assert_eq!(v["hits"][0]["engine"], "duckduckgo");
    }
}
