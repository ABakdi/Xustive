//! Image similarity search — the visual half of "search by image" ([[Vector Index]], M3-T05).
//!
//! An uploaded photo is embedded (CLIP, via the embed sidecar), the resulting vector is searched
//! against Qdrant, and the matching *documents* are resolved from the lexical index for display.
//! One point per media item, so several images can point at one document — results are collapsed by
//! `document_id`, keeping each document's best-scoring image.
//!
//! # Isolation
//!
//! Vector search being unavailable must never affect text search ([[Vector Index]] §7). This whole
//! feature is an `Option` on the state: absent when `[vector] enabled = false` or the services are
//! unreachable, in which case the endpoint returns a clean 503 and nothing else is touched.
//!
//! # Privacy
//!
//! The uploaded image is embedded in memory and never stored; the vector is transient. As with OCR,
//! the bytes arrive as a raw POST body so they never reach a URL or an access log.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::Json;
use serde::Serialize;
use serde_json::Value;

use xustive_vector::{Embedder, SearchFilter, Store};

use crate::error::ApiError;
use crate::search::{to_card, ResultCard};
use crate::state::AppState;

/// The built image-similarity engine: a vector store and an embedder. Present only when enabled.
#[derive(Clone)]
pub struct ImageSearch {
    pub store: Store,
    pub embedder: Arc<dyn Embedder>,
    pub search_limit: usize,
    pub ef_search: usize,
    pub score_threshold: f32,
}

impl ImageSearch {
    /// Build from `[vector]` config, or `None` when disabled. Constructing the clients cannot reach
    /// the network yet, so this never blocks startup; `ensure_collection` is called separately.
    pub fn from_config(cfg: &xustive_core::config::VectorConfig) -> Option<Self> {
        if !cfg.enabled {
            return None;
        }
        let timeout = std::time::Duration::from_millis(cfg.timeout_ms);
        let key = (!cfg.qdrant_key.is_empty()).then(|| cfg.qdrant_key.clone());
        let store = Store::new(&cfg.qdrant_url, key, cfg.collection.clone(), timeout).ok()?;
        let embedder =
            xustive_vector::SidecarEmbedder::new(&cfg.embedder_endpoint, timeout).ok()?;
        Some(Self {
            store,
            embedder: Arc::new(embedder),
            search_limit: cfg.search_limit,
            ef_search: cfg.ef_search,
            score_threshold: cfg.score_threshold(),
        })
    }
}

#[derive(Serialize)]
pub struct ImageSearchResponse {
    pub results: Vec<ResultCard>,
    /// How many similar images were found before collapsing by document — lets the UI say
    /// "12 similar images across 5 pages" honestly.
    pub matched_images: usize,
}

/// `POST /api/v1/search/image` — find documents whose images look like the uploaded one.
pub async fn handler(
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Json<ImageSearchResponse>, ApiError> {
    let Some(engine) = state.image_search.clone() else {
        // Feature off or services down: a clean, specific unavailable — never a 500, and never a
        // side effect on text search.
        return Err(ApiError::model_unavailable("image_search_unavailable"));
    };
    if body.is_empty() {
        return Err(ApiError::BadImage {
            code: "empty_image",
        });
    }
    if body.len() > state.config.media.max_image_bytes {
        return Err(ApiError::BadImage {
            code: "image_too_large",
        });
    }

    // Embed the upload. An embedder failure (sidecar down, undecodable image) is not a text-search
    // problem — surface it as image search being unavailable.
    let vector = engine.embedder.embed(body.to_vec()).await.map_err(|e| {
        tracing::warn!(error = %e, "CLIP embed failed");
        ApiError::model_unavailable("image_search_unavailable")
    })?;

    let hits = engine
        .store
        .search(
            &vector,
            engine.search_limit,
            engine.ef_search,
            engine.score_threshold,
            &SearchFilter::safe(),
        )
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "vector search failed");
            ApiError::model_unavailable("image_search_unavailable")
        })?;

    let matched_images = hits.len();

    // Collapse by document, keeping each document's best score and first-seen order.
    let mut order: Vec<String> = Vec::new();
    let mut best: std::collections::HashMap<String, f32> = std::collections::HashMap::new();
    for h in &hits {
        let e = best
            .entry(h.payload.document_id.clone())
            .or_insert_with(|| {
                order.push(h.payload.document_id.clone());
                f32::MIN
            });
        *e = e.max(h.score);
    }
    if order.is_empty() {
        return Ok(Json(ImageSearchResponse {
            results: Vec::new(),
            matched_images: 0,
        }));
    }

    // Resolve the documents from the lexical index in one filtered query (`id IN [...]`), then
    // re-order to match similarity and attach each document's best image score.
    let cards = resolve_documents(&state, &order, &best).await;

    Ok(Json(ImageSearchResponse {
        results: cards,
        matched_images,
    }))
}

/// Fetch the documents by id and return them in similarity order with scores attached.
async fn resolve_documents(
    state: &AppState,
    order: &[String],
    best: &std::collections::HashMap<String, f32>,
) -> Vec<ResultCard> {
    let quoted: Vec<String> = order.iter().map(|id| format!("\"{id}\"")).collect();
    let filter = format!("id IN [{}]", quoted.join(", "));
    let query = xustive_search::Query::new("")
        .filter(filter)
        .limit(order.len());

    let index = state.documents_index();
    let hits = match state.search.search::<Value>(&index, &query).await {
        Ok(h) => h.hits,
        Err(e) => {
            tracing::warn!(error = %e, "resolving image-search documents failed");
            return Vec::new();
        }
    };

    // Index the resolved hits by id so they can be emitted in similarity order.
    let mut by_id: std::collections::HashMap<String, Value> = hits
        .into_iter()
        .filter_map(|h| {
            let id = h.get("id").and_then(Value::as_str)?.to_string();
            Some((id, h))
        })
        .collect();

    order
        .iter()
        .filter_map(|id| {
            let hit = by_id.remove(id)?;
            let mut card = to_card(&hit);
            // Cosine score is already 0–1 for normalised vectors; surface it for a qualitative label.
            card.score = best.get(id).copied().unwrap_or(0.0);
            Some(card)
        })
        .collect()
}

// A test-only helper to keep the JSON shape honest without a live Qdrant/CLIP.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_serialises_with_snake_case_fields() {
        let resp = ImageSearchResponse {
            results: Vec::new(),
            matched_images: 3,
        };
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["matched_images"], 3);
        assert!(v["results"].is_array());
    }

    #[test]
    fn disabled_config_builds_no_engine() {
        let cfg = xustive_core::config::VectorConfig::default(); // enabled = false
        assert!(ImageSearch::from_config(&cfg).is_none());
    }

    #[test]
    fn enabled_config_builds_an_engine() {
        let cfg = xustive_core::config::VectorConfig {
            enabled: true,
            ..Default::default()
        };
        let engine = ImageSearch::from_config(&cfg).expect("engine builds when enabled");
        assert_eq!(engine.ef_search, 64);
        assert!((engine.score_threshold - 0.75).abs() < 1e-6);
    }
}
