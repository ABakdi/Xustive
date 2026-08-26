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
    /// The external summariser (M7-T08), `None` when unconfigured — the second and only other
    /// outbound client this gateway may hold. The API key lives here, on the egress plane, and the
    /// serving API never sees it.
    pub llm: Option<Arc<xustive_federation::llm::ExternalLlm>>,
}

/// `POST /federate` request. `budget_ms` lets the caller (the serving pipeline) impose a tighter
/// bound than the gateway's default — it is the caller's latency budget, enforced here too so a slow
/// SearXNG cannot outlast it.
#[derive(Debug, Deserialize)]
pub struct FederateRequest {
    pub query: String,
    #[serde(default)]
    pub budget_ms: Option<u64>,
    /// `web` (default), `images` or `videos` (M9-T06). Absent on the wire from an older API.
    #[serde(default)]
    pub category: xustive_federation::Category,
}

/// `POST /federate` response. `partial` is true when the budget cut the call short or SearXNG
/// errored — the caller ships what it has (index-only when empty) and never waits further.
#[derive(Debug, Serialize)]
pub struct FederateResponse {
    pub hits: Vec<FederatedHit>,
    pub partial: bool,
}

/// `POST /summarise` request (M7-T08): the serving API builds the full grounded prompt (its own
/// passages, citation rules and language instruction) and this gateway only carries it out — so the
/// prompt policy lives in one place whichever backend answers.
#[derive(Debug, Deserialize)]
pub struct SummariseRequest {
    pub prompt: String,
    #[serde(default)]
    pub budget_ms: Option<u64>,
}

/// `POST /summarise` response. `text: None` is the fail-open answer — provider down, over budget,
/// unconfigured — and the caller falls back to the local model without treating it as an error.
#[derive(Debug, Serialize)]
pub struct SummariseResponse {
    pub text: Option<String>,
}

/// Token ceiling for one external summary — a cited paragraph, and a cost bound per call.
const SUMMARY_MAX_TOKENS: u32 = 512;

/// Request-body ceiling (BUG-008). The gateway is unauthenticated on `core` — its callers are
/// trusted by network position, not identity — so the bodies it accepts are the abuse surface: a
/// compromised core container could otherwise pump ~2MB prompts (axum's default) through the
/// operator's paid LLM key. A real query is bytes and a real grounded prompt is a few tens of KB;
/// 64KB covers both with room, and caps what any caller can spend per request.
const MAX_BODY_BYTES: usize = 64 * 1024;

/// The router: a health check, the federation route, and the external-summary route.
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/federate", post(federate))
        .route("/summarise", post(summarise))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_BYTES))
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

async fn summarise(
    State(state): State<AppState>,
    Json(req): Json<SummariseRequest>,
) -> Json<SummariseResponse> {
    Json(summarise_inner(&state, req).await)
}

/// The summarise logic, separated from the axum extractors so it is unit-testable without a server.
/// Fail-open throughout: every failure is `text: None`, never an error status — an external summary
/// is an improvement on the local one, never a precondition for anything.
pub async fn summarise_inner(state: &AppState, req: SummariseRequest) -> SummariseResponse {
    let prompt = req.prompt.trim();
    if prompt.is_empty() {
        return SummariseResponse { text: None };
    }
    let Some(llm) = state.llm.clone() else {
        return SummariseResponse { text: None };
    };
    let budget = req
        .budget_ms
        .map(Duration::from_millis)
        .unwrap_or(state.default_budget);

    match tokio::time::timeout(budget, llm.complete(prompt, SUMMARY_MAX_TOKENS)).await {
        Ok(Ok(text)) => SummariseResponse { text: Some(text) },
        Ok(Err(e)) => {
            // `e` carries a status or transport error, never the prompt (which holds query text).
            tracing::warn!(error = %e, "external summariser call failed; answering empty");
            SummariseResponse { text: None }
        }
        Err(_) => {
            tracing::warn!(
                budget_ms = budget.as_millis() as u64,
                "external summariser exceeded budget; answering empty"
            );
            SummariseResponse { text: None }
        }
    }
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

    match tokio::time::timeout(budget, client.search_in(query, req.category)).await {
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
            llm: None,
        }
    }

    #[tokio::test]
    async fn summarise_is_inert_without_an_llm_and_on_an_empty_prompt() {
        // Both fail-open shapes: unconfigured, and nothing to summarise. Neither is an error.
        let r = summarise_inner(
            &unconfigured(),
            SummariseRequest {
                prompt: "summarise this".into(),
                budget_ms: Some(100),
            },
        )
        .await;
        assert!(r.text.is_none());
        let r = summarise_inner(
            &unconfigured(),
            SummariseRequest {
                prompt: "   ".into(),
                budget_ms: None,
            },
        )
        .await;
        assert!(r.text.is_none());
    }

    #[tokio::test]
    async fn an_empty_query_returns_empty_and_not_partial() {
        let r = federate_inner(
            &unconfigured(),
            FederateRequest {
                query: "   ".into(),
                budget_ms: None,
                category: Default::default(),
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
                category: Default::default(),
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
                media: None,
            }],
            partial: true,
        };
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["partial"], true);
        assert_eq!(v["hits"][0]["url"], "https://example.dz/a");
        assert_eq!(v["hits"][0]["engine"], "duckduckgo");
    }
}
