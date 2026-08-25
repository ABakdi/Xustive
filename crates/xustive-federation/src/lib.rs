//! Query-time federation with a self-hosted SearXNG aggregator ([[ADR-0017]], [[Federation
//! Gateway]], M7-T04).
//!
//! SearXNG is an open-source metasearch engine: give it a query and it returns a ranked,
//! de-duplicated list of results aggregated from many engines. We run our own instance (on the
//! egress network — see [[Federation Gateway]]), so a user query reaches third-party engines only
//! *through our SearXNG*, carrying no client identity, IP, cookie, or session.
//!
//! Unlike the Brave connector (`xustive_ingest::brave`), which takes only URLs for offline discovery, federation keeps the
//! **title and snippet** too: a federated hit is blended into the live answer and its URL is fed to
//! the crawler so the page is indexed and, thereafter, answered locally. The engine name rides along
//! as provenance, so a blended result stays distinguishable in ranking and on the console.
//!
//! This module is the client — the pure response parser and the HTTP call. Reaching SearXNG within
//! a latency budget, blending, the crawl-feed, and the allowlist belong to the [[Federation Gateway]]
//! that calls it; here we keep the part most likely to drift (SearXNG's JSON shape) pure and
//! fixture-tested.

pub mod llm;

use serde::{Deserialize, Serialize};

/// One federated result. Carries what a blended answer needs — the destination, the aggregator's
/// title and snippet, its rank, and which engine surfaced it (provenance).
///
/// Serialisable both ways: the [[Federation Gateway]] serialises these in its `/federate` response,
/// and the serving API deserialises them to blend — one shared shape, no drift between the two.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FederatedHit {
    pub url: String,
    pub title: String,
    pub snippet: String,
    /// The upstream engine SearXNG credits for this result (e.g. `duckduckgo`, `wikipedia`). Empty
    /// when SearXNG did not name one.
    pub engine: String,
    /// 1-based position in SearXNG's returned order.
    pub rank: usize,
}

/// The slice of SearXNG's `format=json` response we read: `results[].{url,title,content,engine}`.
#[derive(Debug, Deserialize)]
struct SearxngResponse {
    #[serde(default)]
    results: Vec<SearxngResult>,
}

#[derive(Debug, Deserialize)]
struct SearxngResult {
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    /// SearXNG calls the snippet `content`.
    #[serde(default)]
    content: String,
    /// Either a single `engine` or a list of `engines`; SearXNG emits both across versions.
    #[serde(default)]
    engine: String,
    #[serde(default)]
    engines: Vec<String>,
}

/// Parse SearXNG's JSON into federated hits, dropping blank URLs and preserving order as rank. Pure,
/// so the shape-handling is tested without a network — the part most likely to drift when SearXNG
/// changes its response between versions.
pub fn parse_results(body: &str) -> Vec<FederatedHit> {
    let Ok(resp) = serde_json::from_str::<SearxngResponse>(body) else {
        return Vec::new();
    };
    resp.results
        .into_iter()
        .filter(|r| !r.url.trim().is_empty())
        .enumerate()
        .map(|(i, r)| {
            let engine = if !r.engine.trim().is_empty() {
                r.engine
            } else {
                r.engines
                    .into_iter()
                    .find(|e| !e.trim().is_empty())
                    .unwrap_or_default()
            };
            FederatedHit {
                url: r.url,
                title: r.title,
                snippet: r.content,
                engine,
                rank: i + 1,
            }
        })
        .collect()
}

/// Why a federation request failed.
#[derive(Debug, thiserror::Error)]
pub enum FederationError {
    #[error("searxng request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("searxng returned {status}")]
    Status { status: u16 },
}

/// A client for a self-hosted SearXNG instance. Holds the endpoint and a per-query hit cap.
#[derive(Clone)]
pub struct SearxngClient {
    http: reqwest::Client,
    base: String,
    max_hits: usize,
}

impl SearxngClient {
    /// Build a client. Returns `None` for an empty endpoint — federation is inert without one rather
    /// than erroring, so a deployment with federation off simply does nothing. `timeout` bounds a
    /// single call; the caller layers its own (tighter) query-time budget on top.
    pub fn new(base: &str, max_hits: usize, timeout: std::time::Duration) -> Option<Self> {
        let base = base.trim().trim_end_matches('/');
        if base.is_empty() {
            return None;
        }
        Some(Self {
            http: reqwest::Client::builder().timeout(timeout).build().ok()?,
            base: base.to_string(),
            max_hits: max_hits.clamp(1, 50),
        })
    }

    /// Search one query and return the federated hits. Asks SearXNG for JSON; `country`/`language`
    /// are left unset — the query text carries the intent, and over-constraining loses the
    /// mixed-script and dialect queries federation most exists to help.
    pub async fn search(&self, query: &str) -> Result<Vec<FederatedHit>, FederationError> {
        let url = format!("{}/search", self.base);
        let resp = self
            .http
            .get(&url)
            .header("Accept", "application/json")
            .query(&[("q", query), ("format", "json")])
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(FederationError::Status {
                status: status.as_u16(),
            });
        }
        let mut hits = parse_results(&resp.text().await?);
        hits.truncate(self.max_hits);
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hits_with_rank_and_provenance() {
        let body = r#"{
            "results": [
                {"url": "https://www.aps.dz/a", "title": "A", "content": "snippet a", "engine": "duckduckgo"},
                {"url": "", "title": "blank"},
                {"url": "https://elkhabar.com/b", "title": "B", "content": "snippet b", "engines": ["bing", "brave"]}
            ]
        }"#;
        let hits = parse_results(body);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://www.aps.dz/a");
        assert_eq!(hits[0].engine, "duckduckgo");
        assert_eq!(hits[0].rank, 1);
        // Blank URL dropped; the next kept hit is rank 2, and `engines[]` fills in for `engine`.
        assert_eq!(hits[1].url, "https://elkhabar.com/b");
        assert_eq!(hits[1].snippet, "snippet b");
        assert_eq!(hits[1].engine, "bing");
        assert_eq!(hits[1].rank, 2);
    }

    #[test]
    fn a_malformed_or_empty_response_yields_nothing() {
        // An error page, a query SearXNG had nothing for, or garbage must be an empty list, not a
        // panic — the fail-open contract starts here.
        assert!(parse_results("not json").is_empty());
        assert!(parse_results(r#"{"results":[]}"#).is_empty());
        assert!(parse_results(r#"{"error":"upstream"}"#).is_empty());
    }

    #[test]
    fn an_empty_endpoint_makes_an_inert_client() {
        let t = std::time::Duration::from_secs(5);
        assert!(SearxngClient::new("", 10, t).is_none());
        assert!(SearxngClient::new("   ", 10, t).is_none());
        assert!(SearxngClient::new("http://xustive-searxng:8080", 10, t).is_some());
        // Trailing slash is normalised so the `/search` join never doubles it.
        assert!(SearxngClient::new("http://xustive-searxng:8080/", 10, t).is_some());
    }
}
