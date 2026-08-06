//! A small Prometheus registry.
//!
//! Hand-rolled rather than pulled from a crate because the requirements are narrow and one
//! constraint is unusual: **label values must be bounded and must never carry user content**.
//! Every recording site here takes `&'static str` labels, which makes "log the query as a label"
//! not expressible rather than merely discouraged.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Latency buckets in seconds, chosen around the search budget (p95 ≤ 200 ms).
const BUCKETS: &[f64] = &[0.005, 0.01, 0.025, 0.05, 0.1, 0.2, 0.4, 0.8, 1.5, 3.0, 10.0];

type Labels = Vec<(&'static str, String)>;

#[derive(Default)]
struct CounterFamily {
    help: &'static str,
    series: BTreeMap<String, (Labels, AtomicU64)>,
}

#[derive(Default)]
struct HistogramFamily {
    help: &'static str,
    series: BTreeMap<String, (Labels, Vec<AtomicU64>, AtomicU64, Mutex<f64>)>,
}

#[derive(Default)]
struct Inner {
    counters: BTreeMap<&'static str, CounterFamily>,
    histograms: BTreeMap<&'static str, HistogramFamily>,
    gauges: BTreeMap<&'static str, (&'static str, AtomicU64)>,
}

/// Process-wide metric registry.
#[derive(Clone, Default)]
pub struct Metrics {
    inner: Arc<Mutex<Inner>>,
}

impl Metrics {
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment a counter by one.
    ///
    /// Labels are `&'static str` keys with short, enumerable values (a route, a status class,
    /// an error code). Never a query, a URL, or an identifier.
    pub fn incr(&self, name: &'static str, help: &'static str, labels: &[(&'static str, &str)]) {
        self.incr_by(name, help, labels, 1);
    }

    pub fn incr_by(
        &self,
        name: &'static str,
        help: &'static str,
        labels: &[(&'static str, &str)],
        n: u64,
    ) {
        let key = label_key(labels);
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let fam = inner.counters.entry(name).or_insert_with(|| CounterFamily {
            help,
            series: BTreeMap::new(),
        });
        fam.series
            .entry(key)
            .or_insert_with(|| (owned(labels), AtomicU64::new(0)))
            .1
            .fetch_add(n, Ordering::Relaxed);
    }

    /// Record a duration observation.
    pub fn observe(
        &self,
        name: &'static str,
        help: &'static str,
        labels: &[(&'static str, &str)],
        seconds: f64,
    ) {
        let key = label_key(labels);
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let fam = inner
            .histograms
            .entry(name)
            .or_insert_with(|| HistogramFamily {
                help,
                series: BTreeMap::new(),
            });
        let entry = fam.series.entry(key).or_insert_with(|| {
            (
                owned(labels),
                BUCKETS.iter().map(|_| AtomicU64::new(0)).collect(),
                AtomicU64::new(0),
                Mutex::new(0.0),
            )
        });
        for (i, &b) in BUCKETS.iter().enumerate() {
            if seconds <= b {
                entry.1[i].fetch_add(1, Ordering::Relaxed);
            }
        }
        entry.2.fetch_add(1, Ordering::Relaxed);
        *entry.3.lock().unwrap_or_else(|e| e.into_inner()) += seconds;
    }

    /// Set a gauge to an absolute value.
    pub fn set_gauge(&self, name: &'static str, help: &'static str, value: u64) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .gauges
            .entry(name)
            .or_insert_with(|| (help, AtomicU64::new(0)))
            .1
            .store(value, Ordering::Relaxed);
    }

    /// Render the Prometheus text exposition format.
    pub fn render(&self) -> String {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = String::with_capacity(4096);

        for (name, fam) in &inner.counters {
            let _ = writeln!(out, "# HELP {name} {}", fam.help);
            let _ = writeln!(out, "# TYPE {name} counter");
            for (labels, value) in fam.series.values() {
                let _ = writeln!(
                    out,
                    "{name}{} {}",
                    render_labels(labels, None),
                    value.load(Ordering::Relaxed)
                );
            }
        }

        for (name, fam) in &inner.histograms {
            let _ = writeln!(out, "# HELP {name} {}", fam.help);
            let _ = writeln!(out, "# TYPE {name} histogram");
            for (labels, buckets, count, sum) in fam.series.values() {
                for (i, &b) in BUCKETS.iter().enumerate() {
                    let _ = writeln!(
                        out,
                        "{name}_bucket{} {}",
                        render_labels(labels, Some(&format!("{b}"))),
                        buckets[i].load(Ordering::Relaxed)
                    );
                }
                let total = count.load(Ordering::Relaxed);
                let _ = writeln!(
                    out,
                    "{name}_bucket{} {total}",
                    render_labels(labels, Some("+Inf"))
                );
                let s = *sum.lock().unwrap_or_else(|e| e.into_inner());
                let _ = writeln!(out, "{name}_sum{} {s}", render_labels(labels, None));
                let _ = writeln!(out, "{name}_count{} {total}", render_labels(labels, None));
            }
        }

        for (name, (help, value)) in &inner.gauges {
            let _ = writeln!(out, "# HELP {name} {help}");
            let _ = writeln!(out, "# TYPE {name} gauge");
            let _ = writeln!(out, "{name} {}", value.load(Ordering::Relaxed));
        }

        out
    }
}

fn owned(labels: &[(&'static str, &str)]) -> Labels {
    labels.iter().map(|(k, v)| (*k, (*v).to_string())).collect()
}

fn label_key(labels: &[(&'static str, &str)]) -> String {
    let mut sorted: Vec<_> = labels.to_vec();
    sorted.sort_by_key(|(k, _)| *k);
    sorted
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn render_labels(labels: &Labels, le: Option<&str>) -> String {
    if labels.is_empty() && le.is_none() {
        return String::new();
    }
    let mut parts: Vec<String> = labels
        .iter()
        .map(|(k, v)| format!("{k}=\"{}\"", escape(v)))
        .collect();
    if let Some(le) = le {
        parts.push(format!("le=\"{le}\""));
    }
    format!("{{{}}}", parts.join(","))
}

fn escape(v: &str) -> String {
    v.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

// --- metric names, so call sites cannot typo them apart ---

pub const HTTP_REQUESTS: &str = "xustive_http_requests_total";
pub const HTTP_REQUESTS_HELP: &str = "Total HTTP requests by route and status";
pub const HTTP_DURATION: &str = "xustive_http_duration_seconds";
pub const HTTP_DURATION_HELP: &str = "HTTP request duration by route";
pub const SEARCH_DURATION: &str = "xustive_search_duration_seconds";
pub const SEARCH_DURATION_HELP: &str = "Search pipeline duration by stage";
pub const SEARCH_RESULTS: &str = "xustive_search_results_total";
pub const SEARCH_RESULTS_HELP: &str = "Searches by result-count bucket and language";
pub const SEARCH_ZERO: &str = "xustive_search_zero_results_total";
pub const SEARCH_ZERO_HELP: &str = "Searches returning no results, by language";
pub const BUILD_INFO: &str = "xustive_build_info";
pub const BUILD_INFO_HELP: &str = "Always 1; presence indicates the process is up";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_accumulates_per_label_set() {
        let m = Metrics::new();
        m.incr(
            HTTP_REQUESTS,
            HTTP_REQUESTS_HELP,
            &[("route", "/search"), ("status", "200")],
        );
        m.incr(
            HTTP_REQUESTS,
            HTTP_REQUESTS_HELP,
            &[("route", "/search"), ("status", "200")],
        );
        m.incr(
            HTTP_REQUESTS,
            HTTP_REQUESTS_HELP,
            &[("route", "/search"), ("status", "503")],
        );

        let out = m.render();
        assert!(out.contains(r#"xustive_http_requests_total{route="/search",status="200"} 2"#));
        assert!(out.contains(r#"xustive_http_requests_total{route="/search",status="503"} 1"#));
    }

    #[test]
    fn label_order_does_not_split_a_series() {
        let m = Metrics::new();
        m.incr(
            HTTP_REQUESTS,
            HTTP_REQUESTS_HELP,
            &[("route", "/x"), ("status", "200")],
        );
        m.incr(
            HTTP_REQUESTS,
            HTTP_REQUESTS_HELP,
            &[("status", "200"), ("route", "/x")],
        );
        let out = m.render();
        // Two increments, one series.
        assert_eq!(out.matches("xustive_http_requests_total{").count(), 1);
        assert!(out.contains("} 2"));
    }

    #[test]
    fn histogram_buckets_are_cumulative() {
        let m = Metrics::new();
        for s in [0.001, 0.03, 0.15, 5.0] {
            m.observe(
                HTTP_DURATION,
                HTTP_DURATION_HELP,
                &[("route", "/search")],
                s,
            );
        }
        let out = m.render();
        // 0.001 only
        assert!(out.contains(r#"le="0.005"} 1"#));
        // 0.001 + 0.03
        assert!(out.contains(r#"le="0.05"} 2"#));
        // + 0.15
        assert!(out.contains(r#"le="0.2"} 3"#));
        // everything
        assert!(out.contains(r#"le="+Inf"} 4"#));
        assert!(out.contains("xustive_http_duration_seconds_count{route=\"/search\"} 4"));
    }

    #[test]
    fn histogram_sum_is_recorded() {
        let m = Metrics::new();
        m.observe(HTTP_DURATION, HTTP_DURATION_HELP, &[], 0.5);
        m.observe(HTTP_DURATION, HTTP_DURATION_HELP, &[], 0.25);
        assert!(m
            .render()
            .contains("xustive_http_duration_seconds_sum 0.75"));
    }

    #[test]
    fn gauge_is_absolute_not_cumulative() {
        let m = Metrics::new();
        m.set_gauge("xustive_test_gauge", "help", 5);
        m.set_gauge("xustive_test_gauge", "help", 3);
        assert!(m.render().contains("xustive_test_gauge 3"));
    }

    #[test]
    fn exposition_includes_help_and_type() {
        let m = Metrics::new();
        m.incr(SEARCH_ZERO, SEARCH_ZERO_HELP, &[("lang", "ary")]);
        let out = m.render();
        assert!(out.contains(&format!("# HELP {SEARCH_ZERO} {SEARCH_ZERO_HELP}")));
        assert!(out.contains(&format!("# TYPE {SEARCH_ZERO} counter")));
    }

    #[test]
    fn label_values_are_escaped() {
        let m = Metrics::new();
        m.incr(HTTP_REQUESTS, HTTP_REQUESTS_HELP, &[("route", r#"a"b\c"#)]);
        let out = m.render();
        assert!(out.contains(r#"route="a\"b\\c""#), "got {out}");
    }

    #[test]
    fn empty_registry_renders_empty() {
        assert_eq!(Metrics::new().render(), "");
    }

    #[test]
    fn concurrent_increments_do_not_lose_counts() {
        let m = Metrics::new();
        std::thread::scope(|s| {
            for _ in 0..8 {
                let m = m.clone();
                s.spawn(move || {
                    for _ in 0..100 {
                        m.incr(HTTP_REQUESTS, HTTP_REQUESTS_HELP, &[("route", "/x")]);
                    }
                });
            }
        });
        assert!(m.render().contains("} 800"));
    }
}
