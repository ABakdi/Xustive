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
pub const BUCKETS: &[f64] = &[0.005, 0.01, 0.025, 0.05, 0.1, 0.2, 0.4, 0.8, 1.5, 3.0, 10.0];

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
struct GaugeFamily {
    help: &'static str,
    series: BTreeMap<String, (Labels, AtomicU64)>,
}

#[derive(Default)]
struct Inner {
    counters: BTreeMap<&'static str, CounterFamily>,
    histograms: BTreeMap<&'static str, HistogramFamily>,
    gauges: BTreeMap<&'static str, (&'static str, AtomicU64)>,
    labelled_gauges: BTreeMap<&'static str, GaugeFamily>,
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

    /// Set one series of a labelled gauge.
    ///
    /// Separate from `set_gauge` because a single number cannot answer the question this exists
    /// for. `data_age_seconds` without a `dataset` label says something somewhere is stale, which
    /// is not enough to page anyone: weather going quiet and exchange rates going quiet need
    /// different responses.
    pub fn set_labelled_gauge(
        &self,
        name: &'static str,
        help: &'static str,
        labels: &[(&'static str, &str)],
        value: u64,
    ) {
        let key = label_key(labels);
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let fam = inner
            .labelled_gauges
            .entry(name)
            .or_insert_with(|| GaugeFamily {
                help,
                series: BTreeMap::new(),
            });
        fam.series
            .entry(key)
            .or_insert_with(|| (owned(labels), AtomicU64::new(0)))
            .1
            .store(value, Ordering::Relaxed);
    }

    /// Total across every series of a counter.
    ///
    /// For the admin dashboard, which wants "how many searches" rather than a breakdown by route
    /// and status. Reading the same counters Prometheus exports, so the two cannot disagree — a
    /// separate tally kept for the dashboard is a second number that drifts.
    pub fn counter_total(&self, name: &str) -> u64 {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .counters
            .get(name)
            .map(|fam| {
                fam.series
                    .values()
                    .map(|(_, v)| v.load(Ordering::Relaxed))
                    .sum()
            })
            .unwrap_or(0)
    }

    /// Count and sum of a histogram across all its series, for rates and means.
    pub fn histogram_totals(&self, name: &str) -> (u64, f64) {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .histograms
            .get(name)
            .map(|fam| {
                fam.series
                    .values()
                    .fold((0u64, 0f64), |(c, s), (_, _, count, sum)| {
                        (
                            c + count.load(Ordering::Relaxed),
                            s + *sum.lock().unwrap_or_else(|e| e.into_inner()),
                        )
                    })
            })
            .unwrap_or((0, 0.0))
    }

    /// The cumulative bucket counts of a histogram, summed across series — a snapshot the
    /// console's sampler diffs against the previous one to get a quantile *for the interval*,
    /// which is what a chart of "p95 over the last hour" actually needs.
    pub fn histogram_buckets(&self, name: &str, key: &str, value: &str) -> Vec<u64> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = vec![0u64; BUCKETS.len()];
        if let Some(fam) = inner.histograms.get(name) {
            for (labels, buckets, _, _) in fam.series.values() {
                if !key.is_empty() && !labels.iter().any(|(k, v)| *k == key && v == value) {
                    continue;
                }
                for (i, b) in buckets.iter().enumerate() {
                    out[i] += b.load(Ordering::Relaxed);
                }
            }
        }
        out
    }

    /// Total across series whose labels contain a given key/value pair.
    pub fn counter_where(&self, name: &str, key: &str, value: &str) -> u64 {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .counters
            .get(name)
            .map(|fam| {
                fam.series
                    .values()
                    .filter(|(labels, _)| labels.iter().any(|(k, v)| *k == key && v == value))
                    .map(|(_, v)| v.load(Ordering::Relaxed))
                    .sum()
            })
            .unwrap_or(0)
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

        for (name, fam) in &inner.labelled_gauges {
            let _ = writeln!(out, "# HELP {name} {}", fam.help);
            let _ = writeln!(out, "# TYPE {name} gauge");
            for (labels, value) in fam.series.values() {
                let _ = writeln!(
                    out,
                    "{name}{} {}",
                    render_labels(labels, None),
                    value.load(Ordering::Relaxed)
                );
            }
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
/// The cross-encoder round trip ([[ADR-0032]], M13), seconds.
pub const RERANK_DURATION: &str = "xustive_rerank_duration_seconds";
pub const RERANK_DURATION_HELP: &str = "Cross-encoder reranker round trip, seconds";

/// "Did you mean" corrections shown, by whether the page used them.
pub const SPELLING: &str = "xustive_spelling_total";
pub const SPELLING_HELP: &str = "Spelling corrections offered or applied";

pub const DEGRADED: &str = "xustive_degraded_total";
pub const DEGRADED_HELP: &str =
    "Requests that skipped a stage to stay inside the deadline, by stage";

/// Instant answers served, by tool.
///
/// Bounded cardinality: the label is the tool name, of which there are nine. Never the query.
pub const INSTANT_ANSWERS: &str = "xustive_instant_answers_total";
pub const INSTANT_ANSWERS_HELP: &str = "Instant answers served, by tool";

pub const SUGGEST_TOTAL: &str = "xustive_suggest_total";
pub const SUGGEST_TOTAL_HELP: &str =
    "Suggestion requests, labelled only by whether the result was empty. Never by prefix";

pub const EXPANSION_USED: &str = "xustive_query_expansion_total";
pub const EXPANSION_USED_HELP: &str =
    "Queries that needed a second, expanded retrieval leg, by language";

pub const RATE_LIMITED: &str = "xustive_rate_limited_total";
pub const RATE_LIMITED_HELP: &str = "Requests refused by the rate limiter, by route";

pub const SUMMARY_DURATION: &str = "xustive_summary_duration_seconds";
pub const SUMMARY_DURATION_HELP: &str = "Time to produce a summary, end to end";
pub const SUMMARY_WITHHELD: &str = "xustive_summary_withheld_total";
pub const SUMMARY_WITHHELD_HELP: &str =
    "Summaries not shown, by reason. Refusals are normal; generation failures are not";
pub const SUMMARY_EXTERNAL: &str = "xustive_summary_external_total";
pub const SUMMARY_EXTERNAL_HELP: &str =
    "External-summariser attempts by outcome (M7-T08); every non-ok falls back to the local model";
pub const FEDERATION_DURATION: &str = "xustive_federation_duration_seconds";
pub const FEDERATION_DURATION_HELP: &str =
    "Detached federation fetch duration, spawn to hits (M7-T09.2) — off the response's critical path";
pub const FEDERATION_BLEND: &str = "xustive_federation_blend_cards_total";
pub const FEDERATION_BLEND_HELP: &str =
    "Result cards served on federation-on first pages, by source \
    (web|local). The web share falling over time is the convergence measure: the index catching up";

pub const SEARCH_DURATION: &str = "xustive_search_duration_seconds";
pub const SEARCH_DURATION_HELP: &str = "Search pipeline duration by stage";
pub const SEARCH_RESULTS: &str = "xustive_search_results_total";
pub const SEARCH_RESULTS_HELP: &str = "Searches by result-count bucket and language";
pub const SEARCH_ZERO: &str = "xustive_search_zero_results_total";
pub const SEARCH_ZERO_HELP: &str = "Searches returning no results, by language";
pub const LANG_DETECTED: &str = "xustive_lang_detected_total";
pub const LANG_DETECTED_HELP: &str =
    "Detected query language and script. The share of `ary` is itself a product metric: if it is near zero, either detection is broken or the audience assumption is wrong.";
pub const BUILD_INFO: &str = "xustive_build_info";
pub const BUILD_INFO_HELP: &str = "Always 1; presence indicates the process is up";

pub const FEDERATION_SEARCHES: &str = "xustive_federation_searches_total";
pub const FEDERATION_SEARCHES_HELP: &str =
    "Searches that consulted federation, by outcome: `hits` (the gateway returned results), `empty` (on, but nothing came back — a miss, a timeout, or the gateway down). The ratio is federation's live contribution, expected to fall as the crawl-feed fills the index.";
pub const FEDERATION_FED: &str = "xustive_federation_urls_fed_total";
pub const FEDERATION_FED_HELP: &str =
    "Federated URLs queued for crawling — the crawl-feed that converges the index toward answering locally. Each becomes a real result once the crawler reaches it.";
pub const SEMANTIC_FUSED: &str = "xustive_semantic_fused_total";
pub const SEMANTIC_FUSED_HELP: &str =
    "Searches where the dense (semantic) leg contributed candidates, by kind: `recall` (it added documents the lexical leg missed) or `reinforce` (its top ids were all already lexical hits). The `recall` share is semantic search earning its keep.";

pub const QUEUE_DEPTH: &str = "xustive_queue_depth";
pub const QUEUE_DEPTH_HELP: &str =
    "Documents waiting to be indexed. Consumer-group lag, not stream length: the stream is capped and trimmed, so its length stops rising long before the backlog does and would read as a healthy queue during exactly the incident this is meant to catch.";
pub const QUEUE_PENDING: &str = "xustive_queue_pending";
pub const QUEUE_PENDING_HELP: &str =
    "Claimed but not acknowledged. Rising while the depth falls means workers are taking work and dying with it.";
pub const QUEUE_DEAD: &str = "xustive_queue_dead_letters";
pub const QUEUE_DEAD_HELP: &str =
    "Documents the indexer gave up on. Any sustained rise is data loss; replay is deliberate and manual, so nothing clears this on its own.";

pub const CRAWL_FETCHED: &str = "xustive_crawl_fetched_total";
pub const CRAWL_FETCHED_HELP: &str = "Pages fetched by the crawler since it started.";
pub const CRAWL_REVISITED: &str = "xustive_crawl_revisited_total";
pub const CRAWL_REVISITED_HELP: &str =
    "Of the fetches, how many were revisits of pages already held. fetched minus this is fresh discovery — the two halves of the crawl budget, separated so freshness and coverage cannot starve each other unseen.";

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
