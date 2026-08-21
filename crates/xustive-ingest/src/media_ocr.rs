//! Index-side image OCR enrichment ([[Enrichment Pipeline]], M3-T07).
//!
//! A crawled page's images (already extracted into `Document.media` by the parser) are fetched and
//! OCR'd so the text *inside* an image becomes searchable. Opt-in, bounded, and failure-isolated:
//! **a failed image never fails its document** (M3-T07.5) — every error is swallowed and the media is
//! simply left without `ocr_text`.
//!
//! Runs in the ingestion plane, after parse. The fetch is SSRF-guarded ([`SafeUrl`]) and size-capped;
//! the OCR itself is CPU-bound and goes to the blocking pool.

use std::time::Duration;

use xustive_core::{BodySource, Document, MediaKind, SafeUrl};
use xustive_media::ocr;

/// Bounded settings, from `[media]`.
#[derive(Debug, Clone)]
pub struct Settings {
    pub tessdata: String,
    pub langs: String,
    /// Cost ceiling: at most this many images are fetched + OCR'd per document.
    pub max_images: usize,
    pub max_bytes: usize,
}

/// A small image fetcher with its own client — it never inherits the crawler's identity, and its
/// only job is to pull image bytes safely.
#[derive(Clone)]
pub struct ImageFetcher {
    http: reqwest::Client,
}

impl ImageFetcher {
    pub fn new() -> Option<Self> {
        let http = reqwest::Client::builder()
            .user_agent("XustiveBot/1.0 (+https://xustive.dz/bot; media OCR)")
            .timeout(Duration::from_secs(15))
            .build()
            .ok()?;
        Some(Self { http })
    }

    /// Fetch image bytes: private addresses refused ([`SafeUrl`]), `image/*` only, size-capped both
    /// by the declared length and by the actual bytes read. Shared by the OCR and embed passes.
    pub async fn fetch(&self, url: &str, max_bytes: usize) -> Option<Vec<u8>> {
        let safe = SafeUrl::parse(url).ok()?;
        let resp = self.http.get(safe.as_str()).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let is_image = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ct| ct.trim_start().starts_with("image/"));
        if !is_image {
            return None;
        }
        if resp
            .content_length()
            .is_some_and(|l| l as usize > max_bytes)
        {
            return None;
        }
        let bytes = resp.bytes().await.ok()?;
        if bytes.len() > max_bytes {
            return None;
        }
        Some(bytes.to_vec())
    }
}

/// OCR a document's images in place, then backfill a thin body from what was read.
pub async fn enrich(document: &mut Document, fetcher: &ImageFetcher, cfg: &Settings) {
    let candidates: Vec<usize> = document
        .media
        .iter()
        .enumerate()
        .filter(|(_, m)| m.kind == MediaKind::Image && m.ocr_text.is_none())
        .map(|(i, _)| i)
        .take(cfg.max_images)
        .collect();
    if candidates.is_empty() {
        return;
    }

    let mut read: Vec<String> = Vec::new();
    for i in candidates {
        let url = document.media[i].url.clone();
        let Some(bytes) = fetcher.fetch(&url, cfg.max_bytes).await else {
            continue; // a failed image is skipped, never fatal
        };
        let (tessdata, langs) = (cfg.tessdata.to_string(), cfg.langs.to_string());
        let ocr = tokio::task::spawn_blocking(move || {
            ocr::recognise(&bytes, &tessdata, &langs, ocr::MAX_PIXELS)
        })
        .await;
        let Ok(Ok(ocr)) = ocr else {
            continue;
        };
        if ocr.usable {
            read.push(ocr.text.clone());
            document.media[i].ocr_text = Some(ocr.text);
            document.media[i].ocr_lang = Some(cfg.langs.to_string());
        }
    }

    // Backfill the body when the page's own text is thin — the image *was* the content (M3-T07.4).
    // A page with real prose keeps its prose; OCR only fills a gap, it never overwrites.
    if !read.is_empty() && document.body.split_whitespace().count() < THIN_WORDS {
        let joined = read.join(" ");
        if document.body.trim().is_empty() {
            document.body = joined;
            document.body_source = BodySource::Ocr;
        } else {
            document.body = format!("{} {}", document.body.trim(), joined);
        }
        document.body_len = document.body.chars().count();
    }
}

/// Below this word count a document's own text is "thin" enough that OCR should backfill it.
const THIN_WORDS: usize = 20;
