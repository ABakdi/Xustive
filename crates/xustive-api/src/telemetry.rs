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

    // llama.cpp is quietened by default. It logs every tensor and graph reservation at INFO,
    // which is two hundred lines on each model load — enough to bury this process's own startup
    // output and make a working server look like a stuck one. An explicit `llama_cpp_2=` in the
    // filter still wins, so `RUST_LOG=llama_cpp_2=info` brings it all back when a load misbehaves.
    // The target is the crate *name* with hyphens, not the Rust module path — `llama-cpp-2`,
    // not `llama_cpp_2`. Getting that wrong silently does nothing, which is exactly how the
    // first attempt at this failed.
    const QUIET_LLAMA: &str = "llama-cpp-2=warn";
    let requested = &cfg.log_filter;
    let filter = if requested.contains("llama") {
        EnvFilter::try_new(requested.clone())
    } else {
        EnvFilter::try_new(format!("{requested},{QUIET_LLAMA}"))
    }
    .unwrap_or_else(|_| EnvFilter::new(format!("info,{QUIET_LLAMA}")));

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
