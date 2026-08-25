//! The Brave Search API connector (M2-T16.6).
//!
//! Query-driven discovery ([[weak_coverage]]) finds the searches the corpus cannot answer. This
//! turns one of those terms into URLs: it asks Brave's Search API and returns the result links,
//! which are then seeded into the ordinary frontier at a discovered-tier trust and fetched under the
//! ordinary rules. Brave's copy of a page is never served — the API answer is a list of URLs worth
//! fetching ourselves.
//!
//! It is the **paid** rung of the discovery ladder, tried before direct SERP collection because its
//! terms permit this use. So it is **off by default**, needs a key, and every run is capped by a
//! query budget — Brave charges per query, and an unbounded loop is a bill, not a bug report.
//!
//! Only the URL list is taken. The result titles and snippets Brave returns are ignored: they are
//! Brave's text about a page, not the page, and indexing them would be both lower quality and a
//! licensing question we have no need to open.

use serde::Deserialize;

/// One web result from Brave — only the URL, which is all discovery needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BraveResult {
    pub url: String,
}

/// The slice of Brave's response we read: `web.results[].url`.
#[derive(Debug, Deserialize)]
struct BraveResponse {
    #[serde(default)]
    web: Option<BraveWeb>,
}

#[derive(Debug, Deserialize)]
struct BraveWeb {
    #[serde(default)]
    results: Vec<BraveWebResult>,
}

#[derive(Debug, Deserialize)]
struct BraveWebResult {
    #[serde(default)]
    url: String,
}

/// Parse Brave's JSON response into result URLs, dropping blanks. Pure, so the shape-handling is
/// tested without a key or the network — the part most likely to drift when Brave changes its
/// response.
pub fn parse_results(body: &str) -> Vec<BraveResult> {
    let Ok(resp) = serde_json::from_str::<BraveResponse>(body) else {
        return Vec::new();
    };
    resp.web
        .map(|w| w.results)
        .unwrap_or_default()
        .into_iter()
        .filter(|r| !r.url.trim().is_empty())
        .map(|r| BraveResult { url: r.url })
        .collect()
}

/// Why a Brave request failed.
#[derive(Debug, thiserror::Error)]
pub enum BraveError {
    #[error("brave request failed: {0}")]
    Http(reqwest::Error),
    #[error("brave api {status}")]
    Status { status: u16 },
    #[error("no api key configured")]
    NoKey,
}

/// The URL is stripped off every wrapped transport error (BUG-033): the Brave request carries the
/// search term as `?q=`, reqwest's error Display embeds the request URL, and the discover loop
/// logs `error = %e` — an unscrubbed error printed weak-coverage terms into the logs whenever
/// Brave was unreachable. Scrubbed once here so no call site has to remember.
impl From<reqwest::Error> for BraveError {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e.without_url())
    }
}

/// A client for the Brave Search API. Holds the subscription token and nothing else.
#[derive(Clone)]
pub struct BraveClient {
    http: reqwest::Client,
    api_key: String,
    base: String,
    count: usize,
}

impl BraveClient {
    /// Build a client. Returns `None` for an empty key — the connector is inert without one rather
    /// than erroring, so a run with Brave off simply does nothing.
    pub fn new(api_key: &str, results_per_query: usize) -> Option<Self> {
        let key = api_key.trim();
        if key.is_empty() {
            return None;
        }
        Some(Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .ok()?,
            api_key: key.to_string(),
            base: "https://api.search.brave.com/res/v1/web/search".to_string(),
            count: results_per_query.clamp(1, 20),
        })
    }

    /// Point at a different endpoint (a test stub or a mirror).
    pub fn with_base(mut self, base: &str) -> Self {
        self.base = base.to_string();
        self
    }

    /// Search one query and return the result URLs. `country=AL` and `search_lang` are not set —
    /// the query text already carries the intent, and over-constraining loses Darija and mixed-script
    /// queries the corpus most needs help with.
    pub async fn search(&self, query: &str) -> Result<Vec<BraveResult>, BraveError> {
        let resp = self
            .http
            .get(&self.base)
            .header("X-Subscription-Token", &self.api_key)
            .header("Accept", "application/json")
            .query(&[("q", query), ("count", &self.count.to_string())])
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(BraveError::Status {
                status: status.as_u16(),
            });
        }
        Ok(parse_results(&resp.text().await?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_result_urls_and_drops_blanks() {
        let body = r#"{
            "web": { "results": [
                { "url": "https://www.aps.dz/a", "title": "x" },
                { "url": "", "title": "blank" },
                { "url": "https://elkhabar.com/b" }
            ]}
        }"#;
        let urls: Vec<String> = parse_results(body).into_iter().map(|r| r.url).collect();
        assert_eq!(urls, vec!["https://www.aps.dz/a", "https://elkhabar.com/b"]);
    }

    #[test]
    fn a_response_with_no_web_block_yields_nothing() {
        // An error payload, or a query Brave had nothing for, must be an empty list, not a panic.
        assert!(parse_results(r#"{"type":"ErrorResponse"}"#).is_empty());
        assert!(parse_results("not json").is_empty());
        assert!(parse_results(r#"{"web":{"results":[]}}"#).is_empty());
    }

    #[test]
    fn an_empty_key_makes_an_inert_client() {
        assert!(BraveClient::new("", 10).is_none());
        assert!(BraveClient::new("   ", 10).is_none());
        assert!(BraveClient::new("real-key", 10).is_some());
    }
}
