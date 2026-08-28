//! The cross-encoder client ([[ADR-0032]], M13-T04): the top of the page to the reranker
//! sidecar, one score per candidate back, under a hard timeout. The fusion itself is pure and
//! lives in `xustive_search::rank::fuse_rrf`; this module only fetches the model's opinion and
//! never lets a slow or absent sidecar touch the page — `None` means "rank as before".

use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use xustive_core::config::RerankerConfig;

#[derive(Clone)]
pub struct Reranker {
    http: reqwest::Client,
    endpoint: String,
    pub top_n: usize,
}

#[derive(Deserialize)]
struct Scores {
    scores: Vec<f32>,
}

impl Reranker {
    pub fn new(cfg: &RerankerConfig) -> Option<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_millis(cfg.timeout_ms.max(50)))
            .build()
            .ok()?;
        Some(Self {
            http,
            endpoint: cfg.endpoint.clone(),
            top_n: cfg.top_n.clamp(1, 50),
        })
    }

    /// The model's score for each of `documents`, in order — or `None` on timeout, error, or a
    /// reply of the wrong length. Nothing about the request is logged but the outcome.
    pub async fn scores(&self, query: &str, documents: &[String]) -> Option<Vec<f32>> {
        if documents.is_empty() {
            return Some(Vec::new());
        }
        let resp = self
            .http
            .post(&self.endpoint)
            .json(&json!({ "query": query, "documents": documents }))
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let out: Scores = resp.json().await.ok()?;
        (out.scores.len() == documents.len()).then_some(out.scores)
    }
}

/// What the model reads for one candidate: the title and the excerpt the page shows, never the
/// body — the judgement should be on what the reader will see.
pub fn passage_of(hit: &Value) -> String {
    let title = hit
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let excerpt = hit
        .get("excerpt")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    match (title.is_empty(), excerpt.is_empty()) {
        (false, false) => format!("{title} — {excerpt}"),
        (false, true) => title.to_string(),
        _ => excerpt.to_string(),
    }
}
