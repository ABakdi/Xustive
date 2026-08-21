//! Index-side image embedding ([[Vector Index]], M3-T05, the write path).
//!
//! For each of a crawled document's images, produce a CLIP embedding (via the embed sidecar) and
//! upsert it into Qdrant so the image becomes findable by visual similarity. Runs in the ingestion
//! plane, alongside — and structurally like — [`crate::media_ocr`]: opt-in, bounded per document,
//! and **failure-isolated**, so a failed embed never fails its document.
//!
//! One point per media item, keyed by [`xustive_vector::point_id`] on the media URL, so re-crawling
//! the same image overwrites its point rather than duplicating it. The payload carries exactly the
//! filterable fields the query path needs (document id, source type, date, NSFW flag, phash).

use xustive_core::{Document, MediaKind};
use xustive_vector::{point_id, Embedder, Payload, Point, Store};

use crate::media_ocr::ImageFetcher;

/// Bounded settings, from `[vector]` / `[media]`.
#[derive(Debug, Clone)]
pub struct Settings {
    /// At most this many images embedded per document — the cost ceiling per page.
    pub max_images: usize,
    pub max_bytes: usize,
}

/// Everything the embed pass needs, cloned per worker.
#[derive(Clone)]
pub struct ImageEmbed {
    pub fetcher: ImageFetcher,
    pub embedder: std::sync::Arc<dyn Embedder>,
    pub store: Store,
    pub settings: Settings,
}

/// Embed a document's images and upsert them. Every failure is swallowed — a document is indexed
/// for text regardless of whether its images could be embedded.
pub async fn embed_and_store(document: &Document, cfg: &ImageEmbed) {
    let candidates: Vec<usize> = document
        .media
        .iter()
        .enumerate()
        .filter(|(_, m)| m.kind == MediaKind::Image)
        .map(|(i, _)| i)
        .take(cfg.settings.max_images)
        .collect();
    if candidates.is_empty() {
        return;
    }

    let mut points: Vec<Point> = Vec::new();
    for i in candidates {
        let media = &document.media[i];
        let Some(bytes) = cfg.fetcher.fetch(&media.url, cfg.settings.max_bytes).await else {
            continue; // a failed image is skipped, never fatal
        };
        let vector = match cfg.embedder.embed(bytes).await {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(url = %media.url, error = %e, "image embed skipped");
                continue;
            }
        };
        points.push(Point {
            id: point_id(&media.url),
            vector,
            payload: Payload {
                document_id: document.id.clone(),
                media_url: media.url.clone(),
                source_type: Some(document.source_type.as_str().to_string()),
                published_at: (document.published_at > 0).then_some(document.published_at),
                is_nsfw: document.is_nsfw,
                phash: media.phash.clone(),
            },
        });
    }

    if points.is_empty() {
        return;
    }
    if let Err(e) = cfg.store.upsert(&points).await {
        // A vector-store outage must not fail ingestion — the document is already text-indexed, and
        // its embeddings are re-derivable on the next crawl ([[Vector Index]] §7).
        tracing::warn!(doc = %document.id, error = %e, "image embedding upsert failed");
    }
}
