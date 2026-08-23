//! The serving-side client for the [[Federation Gateway]] (M7-T05, [[ADR-0017]]).
//!
//! This is the *only* outbound call the serving plane makes, and it goes to one internal address —
//! the `xustive-federator` gateway on the `core` network — never to the open internet. The gateway,
//! dual-homed, is what reaches the self-hosted SearXNG. So the API keeps its no-egress property: it
//! can reach this one gateway and nothing else.
//!
//! Everything here is **fail-open**. Federation is additive — a "from the web" strip beside the real
//! results — so a disabled, slow, or broken gateway yields an empty strip and the search is exactly
//! what it would be without federation. A per-gateway circuit breaker (T04.5) sheds a failing
//! gateway fast instead of spending the budget on it every request. No query text is ever logged.

use std::time::Duration;

use serde::Deserialize;
use xustive_ingest::federation::FederatedHit;

/// The gateway's `/federate` response shape (mirrors `xustive_federator::FederateResponse`).
#[derive(Debug, Deserialize)]
struct FederateReply {
    #[serde(default)]
    hits: Vec<FederatedHit>,
    #[serde(default)]
    partial: bool,
}

/// A client for the Federation Gateway. Present only when the API is configured to federate
/// (`[federation] enabled` *and* a `federator_url`).
#[derive(Clone)]
pub struct FederatorClient {
    http: reqwest::Client,
    /// `{federator_url}/federate`.
    endpoint: String,
    /// `{federator_url}/healthz`.
    health: String,
    default_budget_ms: u64,
    /// Fail fast when the gateway is down instead of waiting out the budget on every request.
    breaker: xustive_core::circuit::SharedBreaker,
}

impl FederatorClient {
    /// Build from `[federation]`, or `None` when there is no gateway URL to call. Built whenever a
    /// URL is configured — **even if federation is currently off** — because the client's existence
    /// is "configured" while the runtime `federation_enabled` switch is "on/off". Gating the client
    /// on `enabled` would mean the admin toggle could never turn federation on without a restart. The
    /// HTTP timeout is a hair above the budget so the budget, not the socket, bounds a call.
    pub fn from_config(cfg: &xustive_core::config::FederationConfig) -> Option<Self> {
        let base = cfg.federator_url.trim().trim_end_matches('/');
        if base.is_empty() {
            return None;
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_millis(cfg.budget_ms + 500))
            .build()
            .ok()?;
        let breaker = xustive_core::circuit::SharedBreaker::new(xustive_core::circuit::Config {
            failure_threshold: 3,
            cooldown: Duration::from_secs(5),
            max_cooldown: Duration::from_secs(60),
        });
        Some(Self {
            http,
            endpoint: format!("{base}/federate"),
            health: format!("{base}/healthz"),
            default_budget_ms: cfg.budget_ms,
            breaker,
        })
    }

    /// The breaker's state, for the admin console (`"closed"`, `"open"`, `"half-open"`).
    pub fn breaker_state(&self) -> &'static str {
        use xustive_core::circuit::State;
        match self.breaker.state() {
            State::Closed => "closed",
            State::Open => "open",
            State::HalfOpen => "half-open",
        }
    }

    /// Liveness probe against the gateway's `/healthz`.
    pub async fn healthy(&self) -> bool {
        matches!(self.http.get(&self.health).send().await, Ok(r) if r.status().is_success())
    }

    /// Fetch federated hits for a query. **Never fails** — any error (breaker open, network, non-2xx,
    /// decode) yields an empty list, because federation must not be able to break or slow a search.
    /// `budget_ms` overrides the configured default when set; it is passed to the gateway *and*
    /// bounds the wait here.
    pub async fn federate(&self, query: &str, budget_ms: Option<u64>) -> Vec<FederatedHit> {
        if query.trim().is_empty() {
            return Vec::new();
        }
        // Fail fast if the gateway has been failing — no request, no wait.
        if !self.breaker.allow() {
            return Vec::new();
        }
        let budget = budget_ms.unwrap_or(self.default_budget_ms);
        match self.try_federate(query, budget).await {
            Ok(hits) => {
                self.breaker.on_success();
                hits
            }
            Err(()) => {
                self.breaker.on_failure();
                Vec::new()
            }
        }
    }

    async fn try_federate(&self, query: &str, budget_ms: u64) -> Result<Vec<FederatedHit>, ()> {
        let call = self
            .http
            .post(&self.endpoint)
            .json(&serde_json::json!({ "query": query, "budget_ms": budget_ms }))
            .send();
        // Bound the wait by the budget even if the gateway misbehaves — federation may never make
        // the local answer wait past this.
        let resp = match tokio::time::timeout(Duration::from_millis(budget_ms), call).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "federation gateway request failed");
                return Err(());
            }
            Err(_) => {
                tracing::warn!(budget_ms, "federation gateway exceeded budget");
                return Err(());
            }
        };
        if !resp.status().is_success() {
            tracing::warn!(status = %resp.status(), "federation gateway returned an error");
            return Err(());
        }
        match resp.json::<FederateReply>().await {
            Ok(reply) => {
                if reply.partial {
                    tracing::debug!("federation returned a partial result");
                }
                Ok(reply.hits)
            }
            Err(e) => {
                tracing::warn!(error = %e, "federation gateway reply decode failed");
                Err(())
            }
        }
    }
}
