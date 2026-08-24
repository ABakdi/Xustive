//! Text embedding for semantic search (M7-T02).
//!
//! A client for the text-embed sidecar (`services/text-embed`): a batch of strings in, one
//! L2-normalised vector out per string. The sidecar runs a multilingual sentence model (bge-m3 by
//! default) on the internal `core` network. Both planes use it — the ingestion side embeds each
//! document's text so it is findable by meaning, the query side embeds the search query.
//!
//! Unlike [`crate::SidecarEmbedder`] (image bytes → one vector), this speaks a JSON batch contract,
//! so a whole crawl batch or a page of candidates is embedded in one round trip.

use crate::{l2_normalise, VectorError};

#[derive(serde::Serialize)]
struct EmbedRequest<'a> {
    texts: &'a [String],
}

#[derive(serde::Deserialize)]
struct EmbedReply {
    vectors: Vec<Vec<f32>>,
}

/// A client for the text-embed sidecar. Holds the endpoint and a circuit breaker, mirroring
/// [`crate::SidecarEmbedder`].
#[derive(Clone)]
pub struct TextEmbedder {
    http: reqwest::Client,
    endpoint: String,
    breaker: xustive_core::circuit::SharedBreaker,
}

impl TextEmbedder {
    /// Build a client. Does not connect — the first request does.
    pub fn new(
        endpoint: impl Into<String>,
        timeout: std::time::Duration,
    ) -> Result<Self, VectorError> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| VectorError::Unreachable(e.to_string()))?;
        let breaker = xustive_core::circuit::SharedBreaker::new(xustive_core::circuit::Config {
            failure_threshold: 3,
            cooldown: std::time::Duration::from_secs(10),
            max_cooldown: std::time::Duration::from_secs(120),
        });
        Ok(Self {
            http,
            endpoint: endpoint.into(),
            breaker,
        })
    }

    /// Liveness probe against the sidecar's `/health`.
    pub async fn healthy(&self) -> bool {
        let base = self
            .endpoint
            .trim_end_matches("/embed")
            .trim_end_matches('/');
        matches!(self.http.get(format!("{base}/health")).send().await, Ok(r) if r.status().is_success())
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

    /// Embed a batch of strings, returning one L2-normalised vector per input, in order. Empty input
    /// makes no call. Breaker-guarded; a tripped breaker returns immediately so a dead sidecar does
    /// not cost the caller its timeout on every request.
    pub async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, VectorError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        if !self.breaker.allow() {
            return Err(VectorError::Unreachable("circuit open".into()));
        }
        match self.try_embed(texts).await {
            Ok(v) => {
                self.breaker.on_success();
                Ok(v)
            }
            Err(e) => {
                self.breaker.on_failure();
                Err(e)
            }
        }
    }

    /// Embed one string — the query path.
    pub async fn embed_one(&self, text: &str) -> Result<Vec<f32>, VectorError> {
        let one = [text.to_string()];
        let mut v = self.embed_batch(&one).await?;
        v.pop()
            .ok_or_else(|| VectorError::Decode("empty embedding response".into()))
    }

    async fn try_embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, VectorError> {
        let resp = self
            .http
            .post(&self.endpoint)
            .json(&EmbedRequest { texts })
            .send()
            .await
            .map_err(|e| VectorError::Unreachable(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(VectorError::Backend {
                status: status.as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let reply: EmbedReply = resp
            .json()
            .await
            .map_err(|e| VectorError::Decode(e.to_string()))?;
        // One vector per input, aligned by position — the caller pairs them with document ids by
        // index, so a length mismatch would silently mis-key vectors. Refuse it.
        if reply.vectors.len() != texts.len() {
            return Err(VectorError::Decode(format!(
                "expected {} vectors, got {}",
                texts.len(),
                reply.vectors.len()
            )));
        }
        Ok(reply.vectors.into_iter().map(l2_normalise).collect())
    }
}
