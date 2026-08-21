//! Latency statistics and the pass/fail verdict against a budget.
//!
//! Percentiles from raw samples (microseconds), computed by the nearest-rank method on a sorted
//! copy. A load test collects at most a few hundred thousand samples, so an exact sort is both
//! simplest and correct — no approximate histogram, no dependency. p95 and p99 are what the
//! [[Performance Budgets]] state, so those are what the verdict keys on.

use serde::Serialize;

/// Collected outcomes of a run.
#[derive(Default)]
pub struct Samples {
    /// Successful-request latencies, microseconds.
    latencies_us: Vec<u64>,
    ok: u64,
    errors: u64,
    /// Requests shed because the in-flight cap was hit — overload, not a server error, but counted.
    shed: u64,
}

impl Samples {
    pub fn record_ok(&mut self, latency_us: u64) {
        self.latencies_us.push(latency_us);
        self.ok += 1;
    }

    pub fn record_error(&mut self) {
        self.errors += 1;
    }

    pub fn record_shed(&mut self) {
        self.shed += 1;
    }

    #[allow(dead_code)] // used in tests; part of the collector API for future multi-runner merges
    pub fn merge(&mut self, mut other: Samples) {
        self.latencies_us.append(&mut other.latencies_us);
        self.ok += other.ok;
        self.errors += other.errors;
        self.shed += other.shed;
    }

    /// Summarise into a report over `elapsed_secs` of wall time.
    pub fn summarise(mut self, elapsed_secs: f64) -> Report {
        self.latencies_us.sort_unstable();
        let sorted = &self.latencies_us;
        let total = self.ok + self.errors + self.shed;
        Report {
            requests: total,
            ok: self.ok,
            errors: self.errors,
            shed: self.shed,
            error_rate: if total == 0 {
                0.0
            } else {
                (self.errors + self.shed) as f64 / total as f64
            },
            throughput_rps: if elapsed_secs > 0.0 {
                total as f64 / elapsed_secs
            } else {
                0.0
            },
            p50_ms: percentile_ms(sorted, 50.0),
            p95_ms: percentile_ms(sorted, 95.0),
            p99_ms: percentile_ms(sorted, 99.0),
            max_ms: sorted.last().map(|u| *u as f64 / 1000.0).unwrap_or(0.0),
        }
    }
}

/// The nearest-rank percentile of a sorted microsecond slice, in milliseconds.
fn percentile_ms(sorted: &[u64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    // Nearest-rank: the ceil(p/100 * n)-th value, 1-indexed.
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[idx] as f64 / 1000.0
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub requests: u64,
    pub ok: u64,
    pub errors: u64,
    pub shed: u64,
    pub error_rate: f64,
    pub throughput_rps: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

impl Report {
    /// Does this run meet a p95 budget (milliseconds) with an acceptable error rate?
    pub fn passes(&self, p95_budget_ms: f64, max_error_rate: f64) -> bool {
        self.p95_ms <= p95_budget_ms && self.error_rate <= max_error_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_on_a_known_distribution() {
        // 1..=100 ms. Nearest-rank p50 = 50th value = 50 ms, p95 = 95 ms, p99 = 99 ms.
        let sorted: Vec<u64> = (1..=100).map(|ms| ms * 1000).collect();
        assert_eq!(percentile_ms(&sorted, 50.0), 50.0);
        assert_eq!(percentile_ms(&sorted, 95.0), 95.0);
        assert_eq!(percentile_ms(&sorted, 99.0), 99.0);
    }

    #[test]
    fn an_empty_sample_set_is_zero_not_a_panic() {
        assert_eq!(percentile_ms(&[], 95.0), 0.0);
        let r = Samples::default().summarise(1.0);
        assert_eq!(r.requests, 0);
        assert_eq!(r.p95_ms, 0.0);
    }

    #[test]
    fn error_rate_counts_errors_and_shed() {
        let mut s = Samples::default();
        for _ in 0..8 {
            s.record_ok(1000);
        }
        s.record_error();
        s.record_shed();
        let r = s.summarise(1.0);
        assert_eq!(r.requests, 10);
        assert_eq!(r.ok, 8);
        assert!((r.error_rate - 0.2).abs() < 1e-9);
    }

    #[test]
    fn passes_checks_both_latency_and_errors() {
        let mut s = Samples::default();
        for _ in 0..100 {
            s.record_ok(150_000); // 150 ms
        }
        let r = s.summarise(1.0);
        assert!(
            r.passes(200.0, 0.01),
            "150 ms p95 should pass a 200 ms budget"
        );
        assert!(
            !r.passes(100.0, 0.01),
            "150 ms p95 should fail a 100 ms budget"
        );
    }

    #[test]
    fn throughput_is_requests_over_time() {
        let mut s = Samples::default();
        for _ in 0..500 {
            s.record_ok(1000);
        }
        let r = s.summarise(10.0);
        assert!((r.throughput_rps - 50.0).abs() < 1e-9);
    }

    #[test]
    fn merge_combines_two_collectors() {
        let mut a = Samples::default();
        a.record_ok(1000);
        let mut b = Samples::default();
        b.record_ok(2000);
        b.record_error();
        a.merge(b);
        let r = a.summarise(1.0);
        assert_eq!(r.requests, 3);
        assert_eq!(r.ok, 2);
        assert_eq!(r.errors, 1);
    }
}
