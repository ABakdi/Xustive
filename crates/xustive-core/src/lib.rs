//! Core types shared across the Xustive workspace.
//!
//! Contains the canonical entities ([`model`]), the error taxonomy the retry layer switches on
//! ([`error`]), the SSRF guard every fetch must pass ([`safe_url`]), deduplication hashing
//! ([`hash`]), and layered configuration ([`config`]).
//!
//! This crate has no knowledge of HTTP, the index, or the queue. It is the vocabulary, not the
//! machinery.

pub mod config;
pub mod error;
pub mod hash;
pub mod model;
pub mod safe_url;

pub use config::Config;
pub use error::{Classify, ErrorClass};
pub use model::{
    Author, BodySource, Comment, CrawlFrequency, CrawlPolicy, DatePrecision, Document, Engagement,
    Geo, Lang, LegalBasis, Media, MediaKind, Script, Sentiment, SentimentLabel, Source, SourceType,
    TrustTier, SCHEMA_VERSION,
};
pub use safe_url::{SafeUrl, UrlError};

/// Generate a new time-sortable identifier.
pub fn new_id() -> String {
    ulid::Ulid::new().to_string()
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
    fn now_is_plausible() {
        let t = now_unix();
        // After 2020 and before 2100 — catches a clock that is catastrophically wrong.
        assert!(t > 1_577_836_800, "clock looks wrong: {t}");
        assert!(t < 4_102_444_800, "clock looks wrong: {t}");
    }
}
