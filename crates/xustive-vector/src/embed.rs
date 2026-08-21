//! CLIP image embedding, behind a trait.
//!
//! Turns image bytes into a 512-d CLIP ViT-B/32 vector, L2-normalised, ready to upsert or query
//! against the [`crate::Store`]. The embedding itself runs in a **sidecar** (`services/clip-embed`)
//! rather than in-process: it keeps the model (and its Python/ML runtime) out of the Rust build, and
//! the same endpoint serves both planes — the ingestion side embedding crawled images and the query
//! side embedding an uploaded photo.
//!
//! Unlike the OCR sidecar ([[ADR-0016 - Two OCR Engines with an Optional Unlimited-OCR Sidecar]]),
//! CLIP ViT-B/32 is ~150M parameters and runs comfortably CPU-only, so image similarity is not
//! gated on a GPU — it is opt-in only because it needs its model and index provisioned.
//!
//! # Normalisation
//!
//! Vectors are L2-normalised here, on the way out, so both write and query paths store/compare
//! normalised vectors and Qdrant's cosine reduces to a dot product ([[Vector Index]] §4). Whether
//! the sidecar already normalised is irrelevant — normalising an already-unit vector is a no-op.

use async_trait::async_trait;

use crate::{VectorError, DIM};

/// Anything that turns image bytes into a CLIP vector.
#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, image: Vec<u8>) -> Result<Vec<f32>, VectorError>;
    /// Whether the embedder is ready, for the admin console. Default: true (an in-process embedder
    /// is always ready); the sidecar probes its `/health`.
    async fn healthy(&self) -> bool {
        true
    }
}

/// HTTP client for the CLIP embed sidecar.
///
/// Wire contract: `POST {endpoint}` with raw image bytes, reply `{"embedding": [f32; 512]}`.
#[derive(Clone)]
pub struct SidecarEmbedder {
    http: reqwest::Client,
    endpoint: String,
}

#[derive(serde::Deserialize)]
struct EmbedReply {
    embedding: Vec<f32>,
}

impl SidecarEmbedder {
    pub fn new(
        endpoint: impl Into<String>,
        timeout: std::time::Duration,
    ) -> Result<Self, VectorError> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| VectorError::Unreachable(e.to_string()))?;
        Ok(Self {
            http,
            endpoint: endpoint.into(),
        })
    }
}

#[async_trait]
impl Embedder for SidecarEmbedder {
    async fn embed(&self, image: Vec<u8>) -> Result<Vec<f32>, VectorError> {
        let resp = self
            .http
            .post(&self.endpoint)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(image)
            .send()
            .await
            .map_err(|e| VectorError::Unreachable(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(VectorError::Backend {
                status: resp.status().as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let reply: EmbedReply = resp
            .json()
            .await
            .map_err(|e| VectorError::Decode(e.to_string()))?;
        if reply.embedding.len() != DIM {
            return Err(VectorError::Decode(format!(
                "expected {DIM}-d embedding, got {}",
                reply.embedding.len()
            )));
        }
        Ok(l2_normalise(reply.embedding))
    }

    /// Liveness probe against the sidecar's `/health` (the endpoint is the `/embed` path).
    async fn healthy(&self) -> bool {
        let base = self
            .endpoint
            .trim_end_matches("/embed")
            .trim_end_matches('/');
        matches!(
            self.http.get(format!("{base}/health")).send().await,
            Ok(r) if r.status().is_success()
        )
    }
}

/// L2-normalise in place and return. A zero vector is returned unchanged (there is no unit
/// direction for it) rather than producing NaNs.
pub fn l2_normalise(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_makes_a_unit_vector() {
        let v = l2_normalise(vec![3.0, 4.0]);
        let norm = (v[0] * v[0] + v[1] * v[1]).sqrt();
        assert!((norm - 1.0).abs() < 1e-6);
        assert!((v[0] - 0.6).abs() < 1e-6);
        assert!((v[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn normalise_leaves_a_zero_vector_alone() {
        let v = l2_normalise(vec![0.0, 0.0, 0.0]);
        assert_eq!(v, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn embed_reply_decodes() {
        let r: EmbedReply = serde_json::from_str(r#"{"embedding":[0.1,0.2,0.3]}"#).unwrap();
        assert_eq!(r.embedding.len(), 3);
    }
}
