//! Index-time text embedding for semantic search (M7-T02).
//!
//! Mirrors [`crate::media_embed`] for text: embed a document's title + body and upsert the vector to
//! the Qdrant text collection, keyed by the document id so a re-index overwrites rather than
//! duplicates. **Fail-open** — a vector-store or embedder outage logs and moves on; it never fails a
//! document, exactly as image embedding never fails one. The collection is created by the serving
//! side at startup; the crawler only writes.

use xustive_core::Document;
use xustive_vector::{point_id, Payload, Point, Store, TextEmbedder};

/// The index-time text-embedding dependencies, built once and shared across the crawl.
#[derive(Clone)]
pub struct TextEmbed {
    pub embedder: TextEmbedder,
    pub store: Store,
    /// Characters of `title\nbody` to embed. The head carries the topic — what a retrieval embedding
    /// needs — and a bound keeps the per-document embed cost predictable.
    pub max_chars: usize,
}

impl TextEmbed {
    fn text_of(&self, doc: &Document) -> String {
        let mut s = doc.title.clone();
        if !doc.body.is_empty() {
            if !s.is_empty() {
                s.push('\n');
            }
            s.push_str(&doc.body);
        }
        s.chars().take(self.max_chars).collect()
    }
}

/// Embed one document's text and upsert its vector. Fail-open at every step.
pub async fn embed_and_store(document: &Document, cfg: &TextEmbed) {
    let text = cfg.text_of(document);
    if text.trim().is_empty() {
        return;
    }
    let vector = match cfg.embedder.embed_one(&text).await {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!(error = %e, "text embed failed; document indexed without a vector");
            return;
        }
    };
    // Keyed by `point_id(document.id)` so a re-index of the same document overwrites its point. The
    // image payload fields (media_url, phash, is_nsfw) are left empty — unused for a text point.
    let point = Point {
        id: point_id(&document.id),
        vector,
        payload: Payload {
            document_id: document.id.clone(),
            source_type: Some(document.source_type.as_str().to_string()),
            published_at: (document.published_at > 0).then_some(document.published_at),
            ..Default::default()
        },
    };
    if let Err(e) = cfg.store.upsert(std::slice::from_ref(&point)).await {
        tracing::warn!(error = %e, "text vector upsert failed; document is not semantically findable yet");
    }
}
