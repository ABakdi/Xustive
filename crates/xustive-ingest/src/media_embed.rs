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
use xustive_media::ocr;
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

/// Embed a document's images and upsert them, and stamp each image's perceptual hash. Every failure
/// is swallowed — a document is indexed for text regardless of whether its images could be embedded.
///
/// Takes `&mut Document` so it can fill `media[].phash` (dHash) from the bytes it already fetched —
/// the fingerprint that dedup and the future reuse-skip key on, computed once here rather than by a
/// second fetch elsewhere.
pub async fn embed_and_store(document: &mut Document, cfg: &ImageEmbed) {
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
        let url = document.media[i].url.clone();
        let Some(bytes) = cfg.fetcher.fetch(&url, cfg.settings.max_bytes).await else {
            continue; // a failed image is skipped, never fatal
        };
        // Fingerprint from the fetched bytes, if not already set. Cheap next to the embed, and it
        // travels with the document into the index for dedup ([[Deduplication Service]] §4.4).
        if document.media[i].phash.is_none() {
            document.media[i].phash = xustive_media::phash::dhash(&bytes, ocr::MAX_PIXELS);
        }
        let vector = match cfg.embedder.embed(bytes).await {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(url = %url, error = %e, "image embed skipped");
                continue;
            }
        };
        points.push(Point {
            id: point_id(&url),
            vector,
            payload: Payload {
                document_id: document.id.clone(),
                media_url: url.clone(),
                source_type: Some(document.source_type.as_str().to_string()),
                published_at: (document.published_at > 0).then_some(document.published_at),
                is_nsfw: document.is_nsfw,
                phash: document.media[i].phash.clone(),
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
