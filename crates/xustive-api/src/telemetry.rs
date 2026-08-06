//! Tracing setup.
//!
//! The privacy constraint is structural: **no query text, transcript, or OCR output may appear in
//! any log line, metric label, or span attribute.** A CI lint greps for those identifiers inside
//! `tracing::` macro arguments. The registry in [`crate::metrics`] enforces the same thing at the
//! type level by only accepting `&'static str` label names.

use xustive_core::config::TelemetryConfig;

/// Initialise the global subscriber. Safe to call once per process.
pub fn init(cfg: &TelemetryConfig) {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let filter = EnvFilter::try_new(&cfg.log_filter).unwrap_or_else(|_| EnvFilter::new("info"));

    let registry = tracing_subscriber::registry().with(filter);

    if cfg.log_json {
        registry
            .with(fmt::layer().json().flatten_event(true))
            .init();
    } else {
        registry.with(fmt::layer().compact()).init();
    }
}

/// Bucket a query length for metrics.
///
/// This is what we record *instead of* the query: enough to notice a shift in usage, useless for
/// reconstructing what anyone searched for.
pub fn query_len_bucket(chars: usize) -> &'static str {
    match chars {
        0 => "0",
        1..=10 => "1-10",
        11..=30 => "11-30",
        31..=80 => "31-80",
        _ => "80+",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_buckets_are_bounded_and_lossy() {
        assert_eq!(query_len_bucket(0), "0");
        assert_eq!(query_len_bucket(7), "1-10");
        assert_eq!(query_len_bucket(25), "11-30");
        assert_eq!(query_len_bucket(500), "80+");
        // Distinct queries of similar length collapse to one bucket — that is the point.
        assert_eq!(query_len_bucket(12), query_len_bucket(29));
    }
}
