//! A short memory of how the system has been doing (M12): the numbers the console charts.
//!
//! Prometheus and Grafana exist in `deploy/`, but the operator's page must not depend on a
//! second service being up to show a line. So the API samples its own counters every
//! [`STEP`] seconds into a ring of [`KEEP`] points — 24 hours — and serves it as
//! `GET /api/v1/admin/timeseries`. Each point is what happened *in that interval*: searches per
//! minute, the p95 of the interval's searches (from the histogram buckets' difference, not the
//! lifetime cumulative), the crawler's fetches and indexings, the queue's depth, the events sink's
//! throughput. Restarting the process forgets the ring; that is the price of no dependency, and
//! Prometheus keeps the long memory for those who run it.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Serialize;
use serde_json::{json, Value};

use crate::metrics::{self, BUCKETS};
use crate::state::AppState;

pub const STEP: Duration = Duration::from_secs(30);
/// 24 h at 30 s.
pub const KEEP: usize = 2880;

#[derive(Debug, Clone, Serialize, Default)]
pub struct Point {
    pub at: i64,
    pub searches: u64,
    pub zero_results: u64,
    pub rate_limited: u64,
    pub degraded: u64,
    /// p95 of the interval's search retrieval stage, in ms; null when nothing happened.
    pub search_p95_ms: Option<f64>,
    pub summaries: u64,
    pub summary_p95_ms: Option<f64>,
    pub fetched: u64,
    pub indexed: u64,
    pub failed: u64,
    pub frontier_waiting: u64,
    pub inflight: u64,
    pub events_written: u64,
    pub events_dropped: u64,
}

/// The lifetime values the sampler diffs against.
#[derive(Default)]
struct Prev {
    searches: u64,
    zero: u64,
    rate_limited: u64,
    degraded: u64,
    search_buckets: Vec<u64>,
    summaries: u64,
    summary_buckets: Vec<u64>,
    fetched: u64,
    indexed: u64,
    failed: u64,
    events_written: u64,
    events_dropped: u64,
}

#[derive(Clone, Default)]
pub struct Ring {
    inner: Arc<Mutex<(VecDeque<Point>, Prev)>>,
}

impl Ring {
    pub fn points(&self, hours: u32) -> Vec<Point> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let n = ((hours as u64 * 3600) / STEP.as_secs()) as usize;
        inner.0.iter().rev().take(n.max(1)).rev().cloned().collect()
    }
}

/// p95 of the interval from two cumulative bucket snapshots (Prometheus' `histogram_quantile`
/// over `increase()`, in twelve lines).
fn p95_ms(now: &[u64], prev: &[u64]) -> Option<f64> {
    let d: Vec<u64> = now
        .iter()
        .zip(prev.iter().chain(std::iter::repeat(&0)))
        .map(|(a, b)| a.saturating_sub(*b))
        .collect();
    let total = *d.last()?;
    if total == 0 {
        return None;
    }
    let target = (total as f64 * 0.95).ceil() as u64;
    let mut lower = 0.0;
    let mut lower_count = 0u64;
    for (i, &c) in d.iter().enumerate() {
        if c >= target {
            let upper = BUCKETS[i];
            let span = c - lower_count;
            let frac = if span == 0 {
                1.0
            } else {
                (target - lower_count) as f64 / span as f64
            };
            return Some(((lower + (upper - lower) * frac) * 1000.0 * 10.0).round() / 10.0);
        }
        lower = BUCKETS[i];
        lower_count = c;
    }
    Some(BUCKETS[BUCKETS.len() - 1] * 1000.0)
}

/// The sampler task. Spawned once at startup; never fails the process.
pub fn start(state: AppState) -> Ring {
    let ring = Ring::default();
    let r = ring.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(STEP);
        tick.tick().await; // the first tick fires immediately; the first point needs an interval
        loop {
            tick.tick().await;
            let m = &state.metrics;
            let searches = m.counter_total(metrics::SEARCH_RESULTS);
            let zero = m.counter_total(metrics::SEARCH_ZERO);
            let rate_limited = m.counter_total(metrics::RATE_LIMITED);
            let degraded = m.counter_total(metrics::DEGRADED);
            let search_buckets = m.histogram_buckets(metrics::SEARCH_DURATION, "stage", "retrieve");
            let (summaries, _) = m.histogram_totals(metrics::SUMMARY_DURATION);
            let summary_buckets = m.histogram_buckets(metrics::SUMMARY_DURATION, "", "");
            let snap = crate::admin_crawler::snapshot(&state).await;
            let (ew, ed) = state
                .events
                .as_ref()
                .map(|s| {
                    (
                        s.written.load(std::sync::atomic::Ordering::Relaxed),
                        s.dropped.load(std::sync::atomic::Ordering::Relaxed),
                    )
                })
                .unwrap_or((0, 0));
            let at = crate::events::now();
            let mut inner = r.inner.lock().unwrap_or_else(|e| e.into_inner());
            let (ring, prev) = &mut *inner;
            let point = Point {
                at,
                searches: searches.saturating_sub(prev.searches),
                zero_results: zero.saturating_sub(prev.zero),
                rate_limited: rate_limited.saturating_sub(prev.rate_limited),
                degraded: degraded.saturating_sub(prev.degraded),
                search_p95_ms: p95_ms(&search_buckets, &prev.search_buckets),
                summaries: summaries.saturating_sub(prev.summaries),
                summary_p95_ms: p95_ms(&summary_buckets, &prev.summary_buckets),
                fetched: snap.fetched.saturating_sub(prev.fetched),
                indexed: snap.indexed.saturating_sub(prev.indexed),
                failed: snap.failed.saturating_sub(prev.failed),
                frontier_waiting: snap.waiting as u64,
                inflight: snap.inflight as u64,
                events_written: ew.saturating_sub(prev.events_written),
                events_dropped: ed.saturating_sub(prev.events_dropped),
            };
            *prev = Prev {
                searches,
                zero,
                rate_limited,
                degraded,
                search_buckets,
                summaries,
                summary_buckets,
                fetched: snap.fetched,
                indexed: snap.indexed,
                failed: snap.failed,
                events_written: ew,
                events_dropped: ed,
            };
            ring.push_back(point);
            while ring.len() > KEEP {
                ring.pop_front();
            }
        }
    });
    ring
}

#[derive(serde::Deserialize)]
pub struct Params {
    #[serde(default)]
    pub hours: Option<u32>,
}

/// `GET /api/v1/admin/timeseries?hours=6` — the ring, oldest first, plus the step.
pub async fn handler(
    State(state): State<AppState>,
    crate::admin::Peer(peer): crate::admin::Peer,
    headers: HeaderMap,
    axum::extract::Query(p): axum::extract::Query<Params>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;
    let hours = p.hours.unwrap_or(6).clamp(1, 24);
    let points = state
        .timeseries
        .as_ref()
        .map(|r| r.points(hours))
        .unwrap_or_default();
    Ok(Json(
        json!({ "step_seconds": STEP.as_secs(), "hours": hours, "points": points }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interval_p95_comes_from_the_difference_not_the_lifetime() {
        // Lifetime: 100 fast requests. Interval: 20 more, all in the 0.4–0.8 s bucket.
        let prev: Vec<u64> = BUCKETS
            .iter()
            .map(|&b| if b >= 0.05 { 100 } else { 0 })
            .collect();
        let now: Vec<u64> = BUCKETS
            .iter()
            .map(|&b| {
                if b >= 0.8 {
                    120
                } else if b >= 0.05 {
                    100
                } else {
                    0
                }
            })
            .collect();
        let p = p95_ms(&now, &prev).unwrap();
        assert!(
            p > 400.0 && p <= 800.0,
            "p95 of the interval is in the slow bucket: {p}"
        );
        assert_eq!(p95_ms(&prev, &prev), None);
    }
}
