//! Core types shared across the Xustive workspace.
//!
//! Contains the canonical entities ([`model`]), the error taxonomy the retry layer switches on
//! ([`error`]), the SSRF guard every fetch must pass ([`safe_url`]), deduplication hashing
//! ([`hash`]), and layered configuration ([`config`]).
//!
//! This crate has no knowledge of HTTP, the index, or the queue. It is the vocabulary, not the
//! machinery.

pub mod circuit;
pub mod config;
pub mod error;
pub mod hash;
pub mod model;
pub mod registry;
pub mod safe_url;

pub use config::Config;
pub use error::{Classify, ErrorClass};
pub use model::{
    Author, BodySource, Comment, CrawlFrequency, CrawlPolicy, DatePrecision, DiscoveryChannel,
    Document, Engagement, EnrichmentLevel, Geo, Lang, LegalBasis, Lifecycle, Media, MediaKind,
    Script, Sentiment, SentimentLabel, Source, SourceType, TrustTier, SCHEMA_VERSION,
};
pub use registry::{Registry, RegistryError};
pub use safe_url::{SafeUrl, UrlError};

/// Generate a new time-sortable identifier.
pub fn new_id() -> String {
    ulid::Ulid::new().to_string()
}

/// A deterministic document id derived from a URL, so the same URL always maps to the same document.
///
/// Used where a document must be **idempotent by URL** rather than unique per fetch: the eager
/// federation index (M7) and the crawl of the same URL both compute this, so the full-page crawl
/// overwrites the thin eager document instead of creating a duplicate. Meilisearch ids allow only
/// `[a-zA-Z0-9_-]`, so this is a hex digest with a prefix — never the `b3:` form `hash` uses, whose
/// colon Meilisearch rejects. The caller passes an already-canonicalised URL, so two spellings of
/// the same address do not become two ids.
pub fn id_for_url(url: &str) -> String {
    format!("u-{}", blake3::hash(url.trim().as_bytes()).to_hex())
}

/// Current unix timestamp in seconds.
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_sortable() {
        let a = new_id();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = new_id();
        assert_ne!(a, b);
        // ULIDs are lexicographically ordered by time.
        assert!(a < b, "{a} should sort before {b}");
    }

    #[test]
    fn id_for_url_is_stable_and_meili_safe() {
        // Same URL → same id, so an eager doc and its later crawl converge on one document.
        assert_eq!(
            id_for_url("https://example.dz/a"),
            id_for_url("https://example.dz/a")
        );
        assert_ne!(
            id_for_url("https://example.dz/a"),
            id_for_url("https://example.dz/b")
        );
        // Meilisearch ids allow only [a-zA-Z0-9_-]; the `b3:` colon form would be rejected.
        let id = id_for_url("https://example.dz/a");
        assert!(
            id.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "id has an invalid character: {id}"
        );
    }

    #[test]
    fn now_is_plausible() {
        let t = now_unix();
        // After 2020 and before 2100 — catches a clock that is catastrophically wrong.
        assert!(t > 1_577_836_800, "clock looks wrong: {t}");
        assert!(t < 4_102_444_800, "clock looks wrong: {t}");
    }
}
