//! Semantic (dense) text retrieval for search (M7-T02).
//!
//! The query-side half of semantic search: embed the query with the text-embed sidecar, k-NN the
//! Qdrant text collection, and return the matching document ids in similarity order. The search
//! handler fuses these with the lexical candidates (reciprocal-rank fusion) before re-ranking, so a
//! query worded differently from a document can still find it.
//!
//! Everything is **fail-open**: a dead sidecar or Qdrant yields no dense candidates and search falls
//! back to lexical-only, exactly as it behaves today with semantic search off. The embedder is
//! reached on the internal `core` network — not internet egress.

use std::time::Duration;

use xustive_vector::{SearchFilter, Store, TextEmbedder};

/// The query-time semantic engine: the Qdrant text collection plus the query embedder.
#[derive(Clone)]
pub struct TextSearch {
    pub store: Store,
    pub embedder: TextEmbedder,
    search_limit: usize,
    ef_search: usize,
}

impl TextSearch {
    /// Build from `[vector]`, or `None` when semantic search is off or a client cannot be built.
    /// Never touches the network.
    pub fn from_config(cfg: &xustive_core::config::VectorConfig) -> Option<Self> {
        if !cfg.text_enabled {
            return None;
        }
        let timeout = Duration::from_millis(cfg.timeout_ms);
        let key = (!cfg.qdrant_key.is_empty()).then(|| cfg.qdrant_key.clone());
        let store = Store::with_dim(
            &cfg.qdrant_url,
            key,
            cfg.text_collection.clone(),
            cfg.text_dim,
            timeout,
        )
        .ok()?;
        let embedder = TextEmbedder::new(&cfg.text_embedder_endpoint, timeout).ok()?;
        Some(Self {
            store,
            embedder,
            search_limit: cfg.text_search_limit,
            ef_search: cfg.ef_search,
        })
    }

    /// Liveness of the embedder sidecar, for the admin console.
    pub async fn healthy(&self) -> bool {
        self.embedder.healthy().await
    }

    /// Dense-retrieve document ids for a query, most-similar first. **Fail-open**: any error (embed,
    /// Qdrant, breaker-open) returns an empty list, and search proceeds lexical-only.
    pub async fn candidates(&self, query: &str) -> Vec<String> {
        let vector = match self.embedder.embed_one(query).await {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(error = %e, "query embed failed; no dense candidates");
                return Vec::new();
            }
        };
        // No score threshold: RRF fuses by rank, so a weak-but-present neighbour is simply ranked
        // low, not dropped. The filter is permissive — text points carry no NSFW flag.
        let hits = match self
            .store
            .search(
                &vector,
                self.search_limit,
                self.ef_search,
                0.0,
                &SearchFilter::default(),
            )
            .await
        {
            Ok(h) => h,
            Err(e) => {
                tracing::debug!(error = %e, "dense search failed");
                return Vec::new();
            }
        };
        // Unique document ids, best-similarity order preserved.
        let mut seen = std::collections::HashSet::new();
        let mut ids = Vec::new();
        for h in hits {
            let id = h.payload.document_id;
            if !id.is_empty() && seen.insert(id.clone()) {
                ids.push(id);
            }
        }
        ids
    }
}

/// Fuse a lexical and a dense candidate list by **reciprocal-rank fusion** (M7-T02.3).
///
/// RRF needs no score calibration between the two very different scales (BM25-ish vs cosine): each
/// list contributes `1/(K + rank)` to a document's score, and the union is sorted by the sum. `K`
/// (60, the standard) softens the weight of top ranks so neither list dominates. The result is a
/// reordered candidate pool — dense recall pulls in documents lexical missed, and lexical precision
/// keeps exact matches on top — capped to `pool`, ready for the re-ranker.
///
/// `lexical` is the Meili candidate `Vec<Value>` (each carries `id`); `dense_ids` is the dense order;
/// `dense_docs` supplies the documents for dense ids not already in `lexical`.
pub fn rrf_fuse(
    lexical: Vec<serde_json::Value>,
    dense_ids: &[String],
    dense_docs: Vec<serde_json::Value>,
    pool: usize,
) -> Vec<serde_json::Value> {
    const K: f32 = 60.0;
    use std::collections::HashMap;

    let id_of = |v: &serde_json::Value| -> Option<String> {
        v.get("id").and_then(|x| x.as_str()).map(String::from)
    };

    // One copy of each document's Value, keyed by id (lexical wins ties — same content either way).
    let mut docs: HashMap<String, serde_json::Value> = HashMap::new();
    let mut scores: HashMap<String, f32> = HashMap::new();

    for (rank, v) in lexical.into_iter().enumerate() {
        if let Some(id) = id_of(&v) {
            *scores.entry(id.clone()).or_insert(0.0) += 1.0 / (K + rank as f32);
            docs.entry(id).or_insert(v);
        }
    }
    for v in dense_docs {
        if let Some(id) = id_of(&v) {
            docs.entry(id).or_insert(v);
        }
    }
    for (rank, id) in dense_ids.iter().enumerate() {
        // Only credit dense ids we actually have a document for (in lexical or fetched).
        if docs.contains_key(id) {
            *scores.entry(id.clone()).or_insert(0.0) += 1.0 / (K + rank as f32);
        }
    }

    let mut ranked: Vec<(String, f32)> = scores.into_iter().collect();
    // Sort by fused score desc; ties broken by id for a deterministic order.
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    ranked
        .into_iter()
        .filter_map(|(id, _)| docs.remove(&id))
        .take(pool)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(id: &str) -> serde_json::Value {
        json!({ "id": id, "title": id })
    }

    #[test]
    fn fusion_unions_both_lists_and_surfaces_agreement() {
        // Lexical: [a, b]. Dense: [b, c]. `b` is in both → should rank first; `c` (dense-only,
        // supplied via dense_docs) must appear; nothing is lost.
        let lexical = vec![doc("a"), doc("b")];
        let dense_ids = vec!["b".to_string(), "c".to_string()];
        let dense_docs = vec![doc("c")];
        let fused = rrf_fuse(lexical, &dense_ids, dense_docs, 10);
        let ids: Vec<&str> = fused.iter().filter_map(|v| v["id"].as_str()).collect();
        assert_eq!(ids[0], "b", "the document both legs agree on ranks first");
        assert!(
            ids.contains(&"a") && ids.contains(&"c"),
            "no candidate is dropped: {ids:?}"
        );
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn a_dense_id_with_no_document_is_ignored() {
        // Dense returns an id we could not resolve to a document — it must not appear as a null.
        let fused = rrf_fuse(vec![doc("a")], &["ghost".to_string()], vec![], 10);
        let ids: Vec<&str> = fused.iter().filter_map(|v| v["id"].as_str()).collect();
        assert_eq!(ids, vec!["a"]);
    }

    #[test]
    fn the_pool_cap_is_honoured() {
        let lexical = vec![doc("a"), doc("b"), doc("c")];
        let fused = rrf_fuse(lexical, &[], vec![], 2);
        assert_eq!(fused.len(), 2);
    }
}
