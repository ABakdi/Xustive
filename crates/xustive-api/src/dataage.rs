//! `data_age_seconds` — how old the cached tool data is.
//!
//! # Why this is sampled rather than recorded on use
//!
//! The failure this exists to catch is a fetcher that **stops silently**. `toold` dying leaves
//! the last values in Redis, every card keeps rendering, and the only symptom is that the ages
//! creep upwards. Nothing errors, so nothing else would notice.
//!
//! Setting the gauge when a weather query happens to arrive would therefore miss exactly the case
//! it is built for: the fetcher fails at 03:00, nobody searches for weather until 07:00, and the
//! metric is four hours behind reality at the moment it matters most. Worse, the gauge would
//! freeze at its last value rather than climbing, so a dashboard would show a healthy number for
//! a dead fetcher.
//!
//! So a background task samples the cache on a fixed interval regardless of traffic. The gauge
//! then describes the data, not the request pattern.
//!
//! # Why the maximum, not the mean
//!
//! Weather is 58 separate entries. If Tamanrasset alone stopped updating, an average across all
//! 58 would move by about two per cent and no threshold would ever fire. The gauge reports the
//! **oldest** entry, so one stuck wilaya is as visible as fifty-eight.

use std::time::Duration;

use xustive_toold::weather::{key, Forecast, Weather};
use xustive_toold::Dataset;

use crate::state::AppState;

/// Metric name and help, used by both the sampler and its tests.
pub const METRIC: &str = "xustive_data_age_seconds";
pub const HELP: &str = "Age of the oldest cached entry for a tool dataset, in seconds";

/// How often to sample.
///
/// Well below the shortest staleness limit, so the gauge crosses an alert threshold within one
/// interval of the data actually crossing it. Sampling as slowly as the limit itself would make
/// the alert's own latency comparable to the thing it is measuring.
const INTERVAL: Duration = Duration::from_secs(60);

/// Sample every dataset once and publish the gauges.
///
/// Returns the ages it recorded, so a test can assert on them without a Prometheus scrape.
pub async fn sample(state: &AppState) -> Vec<(&'static str, u64)> {
    let Some(cache) = state.tool_cache.as_ref() else {
        // No cache configured at all. Publishing zero here would read as "perfectly fresh", which
        // is the opposite of the truth, so nothing is published.
        return Vec::new();
    };
    let now = xustive_core::now_unix();
    let mut out = Vec::new();

    // Weather: 58 wilaya entries in the Redis tool cache.
    let mut oldest: Option<u64> = None;
    let mut present = 0usize;
    for wilaya in xustive_toold::weather::targets() {
        if let Ok(Some(cached)) = cache
            .get::<Forecast>(&key(Weather.key_prefix(), wilaya.code))
            .await
        {
            present += 1;
            let age = cached.age(now).as_secs();
            oldest = Some(oldest.map_or(age, |o: u64| o.max(age)));
        }
    }
    if let Some(age) = oldest {
        publish(state, "weather", age, present);
        out.push(("weather", age));
    }

    // Knowledge: entities in the search index rather than the Redis cache, so it is sampled
    // differently — sorted by `updated_at` ascending, one hit, which asks the engine for the
    // oldest entity instead of dragging the whole index across to compute it here.
    if let Some((age, count)) = knowledge_age(state, now).await {
        publish(state, "knowledge", age, count);
        out.push(("knowledge", age));
    }

    out
}

/// Publish one dataset's gauges.
///
/// A dataset with nothing cached publishes no age at all. There is no honest number for "how old
/// is data that does not exist", and a zero would silence the alert precisely when the store has
/// been wiped — absence is caught by a separate `absent()` rule.
fn publish(state: &AppState, dataset: &'static str, age: u64, entries: usize) {
    state
        .metrics
        .set_labelled_gauge(METRIC, HELP, &[("dataset", dataset)], age);
    state.metrics.set_labelled_gauge(
        "xustive_data_entries",
        "Number of cached entries for a tool dataset",
        &[("dataset", dataset)],
        entries as u64,
    );
}

/// The age of the least recently harvested entity, and how many entities exist.
///
/// The same "oldest, not mean" reasoning as weather: one entity stuck at a year old is invisible
/// in an average over thousands, and the failure this catches is a harvester that quietly stopped.
async fn knowledge_age(state: &AppState, now: i64) -> Option<(u64, usize)> {
    use xustive_knowledge::index;
    let query = xustive_search::Query::new("")
        .limit(1)
        .sort(&[&format!("{}:asc", index::F_UPDATED_AT)]);
    let response = state
        .search
        .search::<serde_json::Value>(index::INDEX, &query)
        .await
        .ok()?;
    let oldest = response
        .hits
        .first()?
        .get(index::F_UPDATED_AT)
        .and_then(|v| v.as_i64())?;
    Some((
        now.saturating_sub(oldest).max(0) as u64,
        response.estimated_total_hits,
    ))
}

/// Run the sampler until the process ends.
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(INTERVAL);
        // The default `Burst` behaviour would fire back-to-back to catch up after any delay,
        // which turns a slow Redis into a burst of scans against that same Redis.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let ages = sample(&state).await;
            for (dataset, age) in ages {
                tracing::debug!(dataset, age_seconds = age, "sampled data age");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sampling_interval_is_well_below_the_staleness_limit() {
        // Sampling as slowly as the limit would make the alert's own latency comparable to the
        // thing it measures. The gauge must cross a threshold within roughly one interval of the
        // data crossing it.
        assert!(
            INTERVAL * 10 <= Weather.staleness_limit(),
            "sampling every {:?} is too slow for a {:?} staleness limit",
            INTERVAL,
            Weather.staleness_limit()
        );
    }

    #[test]
    fn the_critical_alert_threshold_matches_the_staleness_limit() {
        // The threshold lives in YAML and the limit lives in Rust, so they will drift unless
        // something compares them. Past the limit the serving plane withholds the card, which is
        // exactly when the alert should be critical — a threshold above it would page after users
        // had already lost the feature, and one below it would page while everything still worked.
        let rules = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../deploy/prometheus/alerts.yml"),
        )
        .expect("alerts.yml must exist next to the metric it alerts on");

        let limit = Weather.staleness_limit().as_secs();
        assert!(
            rules.contains(&format!("xustive_data_age_seconds > {limit}")),
            "no critical rule at the {limit}s staleness limit; alerts.yml and Weather::staleness_limit have drifted"
        );

        // And the warning must fire strictly earlier, or there is no window to act in.
        let warning: u64 = rules
            .lines()
            .filter_map(|l| l.trim().strip_prefix("expr: xustive_data_age_seconds > "))
            .filter_map(|v| v.trim().parse().ok())
            .filter(|v| *v < limit)
            .max()
            .expect("no warning rule below the staleness limit");
        assert!(warning < limit, "warning at {warning} must precede {limit}");
    }

    #[test]
    fn absence_is_alerted_on_separately_from_staleness() {
        // The gauge is deliberately not published for an empty dataset, so a value-based rule
        // cannot catch a flushed cache — it would simply see no series and stay silent.
        let rules = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../deploy/prometheus/alerts.yml"),
        )
        .expect("alerts.yml");
        assert!(
            rules.contains("absent(xustive_data_age_seconds"),
            "a flushed cache would publish no series and silently pass every threshold rule"
        );
    }

    #[test]
    fn the_metric_name_is_prometheus_shaped() {
        assert!(METRIC.starts_with("xustive_"));
        assert!(
            METRIC.ends_with("_seconds"),
            "base unit must be in the name"
        );
        assert!(!METRIC.contains('-') && !METRIC.contains(' '));
    }
}
