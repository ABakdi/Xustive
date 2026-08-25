//! External LLM client for the gateway's summarise route (M7-T08).
//!
//! Speaks the OpenAI-compatible `/chat/completions` shape — the de-facto lingua franca that
//! DeepSeek, Qwen (DashScope), OpenRouter, and Parallel-AI-class services all answer — so one
//! client covers every provider an operator might point it at. Which provider is a config choice,
//! not a code change.
//!
//! Lives in this leaf crate for the same reason the SearXNG client does: the gateway binary must
//! stay free of heavyweight workspace dependencies, and the response parsing is pure enough to
//! unit-test without a server. The API key is taken by value (from the gateway's environment) and
//! never logged; neither is the prompt, which contains query text.

use std::time::Duration;

use serde_json::Value;

use crate::FederationError;

/// A client for one OpenAI-compatible chat-completions endpoint.
pub struct ExternalLlm {
    http: reqwest::Client,
    /// Full URL of the completions route, e.g. `https://api.deepseek.com/chat/completions`.
    endpoint: String,
    model: String,
    key: String,
}

impl ExternalLlm {
    /// `None` when no endpoint is configured — the caller stays inert rather than erroring, the
    /// same contract as the SearXNG client.
    pub fn new(endpoint: &str, model: &str, key: &str, timeout: Duration) -> Option<Self> {
        let endpoint = endpoint.trim();
        if endpoint.is_empty() {
            return None;
        }
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .user_agent("xustive-federator")
            .build()
            .ok()?;
        Some(Self {
            http,
            endpoint: endpoint.to_string(),
            model: model.trim().to_string(),
            key: key.trim().to_string(),
        })
    }

    /// One completion. `max_tokens` bounds the answer (a summary is a paragraph, not an essay).
    pub async fn complete(&self, prompt: &str, max_tokens: u32) -> Result<String, FederationError> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{ "role": "user", "content": prompt }],
            "max_tokens": max_tokens,
            "temperature": 0.3,
        });
        let mut req = self.http.post(&self.endpoint).json(&body);
        if !self.key.is_empty() {
            req = req.bearer_auth(&self.key);
        }
        let resp = req.send().await.map_err(FederationError::from)?;
        let status = resp.status();
        if !status.is_success() {
            return Err(FederationError::Status {
                status: status.as_u16(),
            });
        }
        let text = resp.text().await.map_err(FederationError::from)?;
        parse_completion(&text).ok_or(FederationError::Status { status: 502 })
    }
}

/// Pull the assistant text out of an OpenAI-compatible completions response. Pure, so the shape
/// assumptions are testable without a provider.
pub fn parse_completion(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    let content = v
        .get("choices")?
        .get(0)?
        .get("message")?
        .get("content")?
        .as_str()?
        .trim();
    (!content.is_empty()).then(|| content.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_standard_completion_parses() {
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"ملخص قصير. [1]"}}]}"#;
        assert_eq!(parse_completion(body).as_deref(), Some("ملخص قصير. [1]"));
    }

    #[test]
    fn empty_content_and_malformed_bodies_are_none() {
        assert_eq!(
            parse_completion(r#"{"choices":[{"message":{"content":"  "}}]}"#),
            None
        );
        assert_eq!(parse_completion(r#"{"error":{"message":"quota"}}"#), None);
        assert_eq!(parse_completion("not json"), None);
        assert_eq!(parse_completion(r#"{"choices":[]}"#), None);
    }

    #[test]
    fn an_unconfigured_endpoint_yields_no_client() {
        assert!(ExternalLlm::new("", "m", "k", Duration::from_secs(5)).is_none());
        assert!(ExternalLlm::new(
            "https://api.deepseek.com/chat/completions",
            "deepseek-chat",
            "",
            Duration::from_secs(5)
        )
        .is_some());
    }
}
