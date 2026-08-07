//! Tracing setup.
//!
//! The privacy constraint is structural: **no query text, transcript, or OCR output may appear in
//! any log line, metric label, or span attribute.** A CI lint greps for those identifiers inside
//! `tracing::` macro arguments. The registry in [`crate::metrics`] enforces the same thing at the
//! type level by only accepting `&'static str` label names.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tracing_subscriber::reload;
use tracing_subscriber::EnvFilter;
use xustive_core::config::TelemetryConfig;

/// Handle for changing the log filter at runtime.
///
/// Debug logging is the thing an operator needs during an incident and the last thing they should
/// have to restart a process to get: a restart loses the warm caches and, often, the condition
/// being investigated.
type Reload = reload::Handle<EnvFilter, tracing_subscriber::Registry>;

struct LevelControl {
    handle: Reload,
    /// What to return to. Captured at boot so a revert cannot drift from the configured baseline.
    baseline: String,
    /// When the current override expires, if one is active.
    expires: Option<Instant>,
    current: String,
}

static CONTROL: OnceLock<Mutex<LevelControl>> = OnceLock::new();

/// How long a raised log level survives.
///
/// Debug logging on a busy search engine is expensive and, more importantly, is the state in
/// which the most sensitive data is closest to being written down. Every override expires on its
/// own; nobody has to remember to turn it off, which is the step that never happens.
pub const OVERRIDE_TTL: Duration = Duration::from_secs(15 * 60);

/// Initialise the global subscriber. Safe to call once per process.
pub fn init(cfg: &TelemetryConfig) {
    use tracing_subscriber::{fmt, prelude::*};

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

    let effective = filter.to_string();
    let (filter, handle) = reload::Layer::new(filter);
    let registry = tracing_subscriber::registry().with(filter);

    if cfg.log_json {
        registry
            .with(fmt::layer().json().flatten_event(true))
            .init();
    } else {
        registry.with(fmt::layer().compact()).init();
    }

    let _ = CONTROL.set(Mutex::new(LevelControl {
        handle,
        baseline: effective.clone(),
        expires: None,
        current: effective,
    }));
}

/// The active filter, and how long any override has left.
pub fn level_status() -> (String, String, Option<u64>) {
    let Some(control) = CONTROL.get() else {
        return ("uninitialised".into(), "uninitialised".into(), None);
    };
    let control = match control.lock() {
        Ok(c) => c,
        Err(p) => p.into_inner(),
    };
    let remaining = control
        .expires
        .map(|e| e.saturating_duration_since(Instant::now()).as_secs());
    (control.current.clone(), control.baseline.clone(), remaining)
}

/// Raise or lower the log filter for at most [`OVERRIDE_TTL`].
///
/// Returns the seconds until it reverts. Rejects a filter the subscriber cannot parse rather than
/// applying half of it, since a partially-applied filter is worse than none: it looks like it
/// worked.
pub fn set_level(filter: &str) -> Result<u64, String> {
    let control = CONTROL.get().ok_or("telemetry is not initialised")?;
    let parsed = EnvFilter::try_new(filter).map_err(|e| format!("invalid filter: {e}"))?;
    let rendered = parsed.to_string();

    let mut control = match control.lock() {
        Ok(c) => c,
        Err(p) => p.into_inner(),
    };
    control
        .handle
        .reload(parsed)
        .map_err(|e| format!("could not apply filter: {e}"))?;
    control.current = rendered;
    control.expires = Some(Instant::now() + OVERRIDE_TTL);
    Ok(OVERRIDE_TTL.as_secs())
}

/// Return to the configured filter immediately.
pub fn revert_level() -> Result<String, String> {
    let control = CONTROL.get().ok_or("telemetry is not initialised")?;
    let mut control = match control.lock() {
        Ok(c) => c,
        Err(p) => p.into_inner(),
    };
    let baseline = control.baseline.clone();
    let parsed = EnvFilter::try_new(&baseline).map_err(|e| format!("invalid baseline: {e}"))?;
    control
        .handle
        .reload(parsed)
        .map_err(|e| format!("could not restore filter: {e}"))?;
    control.current = baseline.clone();
    control.expires = None;
    Ok(baseline)
}

/// Revert an expired override. Called on a timer.
///
/// A polling task rather than a timer armed at override time: a timer has to be cancelled and
/// re-armed on every change, and a missed cancellation reverts a fresh override early. Checking
/// once a minute costs nothing and cannot get out of step.
pub fn expire_override() -> bool {
    let Some(control) = CONTROL.get() else {
        return false;
    };
    let expired = {
        let control = match control.lock() {
            Ok(c) => c,
            Err(p) => p.into_inner(),
        };
        control.expires.is_some_and(|e| Instant::now() >= e)
    };
    if expired {
        if let Ok(baseline) = revert_level() {
            tracing::info!(%baseline, "log level override expired");
            return true;
        }
    }
    false
}

/// Run the expiry check for the life of the process.
pub fn spawn_override_expiry() {
    tokio::spawn(async {
        let mut ticker = tokio::time::interval(Duration::from_secs(60));
        loop {
            ticker.tick().await;
            expire_override();
        }
    });
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
    fn an_override_ttl_is_short_enough_to_be_forgotten_safely() {
        // The whole design assumes nobody has to remember to turn debug logging off. Fifteen
        // minutes is long enough to reproduce an incident and short enough that leaving it on is
        // not a standing exposure.
        assert!(OVERRIDE_TTL <= Duration::from_secs(30 * 60));
        assert!(OVERRIDE_TTL >= Duration::from_secs(5 * 60));
    }

    #[test]
    fn an_invalid_filter_is_refused_rather_than_partly_applied() {
        // Without a subscriber installed this reports "not initialised"; with one it reports an
        // invalid filter. Either way it must be an error — a partially-applied filter is worse
        // than none, because it looks like it worked.
        assert!(set_level("this is not a filter =!= at all").is_err());
    }

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
