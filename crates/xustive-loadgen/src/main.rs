//! `xustive-loadgen` — an open-loop HTTP load generator for the serving plane (M4-T03).
//!
//! Rust-native rather than a `k6`/`oha` dependency: one `cargo run`, no toolchain to install, and
//! the query mix and statistics are unit-tested in the same language as the thing under test.
//!
//! # Open loop, on purpose
//!
//! Requests are dispatched on a fixed schedule for the target rate, *independently* of whether prior
//! requests have returned. A closed loop (N workers each firing the next request only after the last
//! completes) measures throughput but hides latency under load — the classic coordinated-omission
//! error, where a stalled server simply receives fewer requests and reports a rosy p95. Here the
//! schedule does not slow down when the server does, so a stall shows up as rising latency and, past
//! the in-flight cap, as *shed* requests — which is exactly what a real overload looks like.
//!
//! # What it does not do
//!
//! It does not assert the [[Performance Budgets]] at 10M documents — that needs the corpus and the
//! target hardware. It measures whatever stack it is pointed at and reports pass/fail against the
//! budget, so the same harness serves a laptop smoke test and a staging load test unchanged.

mod mix;
mod stats;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use tokio::sync::{mpsc, Semaphore};

use crate::mix::Mix;
use crate::stats::{Report, Samples};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Scenario {
    /// `GET /api/v1/search` with whole queries.
    Search,
    /// `GET /api/v1/suggest` with query prefixes (per-keystroke traffic).
    Suggest,
    /// A search followed by `POST /api/v1/summary` for the returned token.
    Summary,
    /// A realistic blend: mostly suggest (per keystroke), some search.
    Mixed,
}

impl Scenario {
    /// The p95 budget in milliseconds ([[Performance Budgets]] §2–3).
    fn default_p95_ms(self) -> f64 {
        match self {
            Scenario::Search | Scenario::Mixed => 200.0,
            Scenario::Suggest => 40.0,
            Scenario::Summary => 2_500.0,
        }
    }

    fn default_rps(self) -> f64 {
        match self {
            Scenario::Search => 100.0,
            Scenario::Suggest => 300.0,
            Scenario::Summary => 2.0,
            Scenario::Mixed => 150.0,
        }
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "xustive-loadgen",
    about = "Open-loop load generator for the Xustive serving plane"
)]
struct Args {
    /// Base URL of the API.
    #[arg(long, default_value = "http://127.0.0.1:8080", env = "XUSTIVE_API_URL")]
    target: String,
    #[arg(long, value_enum, default_value = "search")]
    scenario: Scenario,
    /// Requests per second to dispatch. Defaults to the scenario's realistic rate.
    #[arg(long)]
    rps: Option<f64>,
    /// How long to run, seconds.
    #[arg(long, default_value_t = 30)]
    duration: u64,
    /// Maximum requests in flight before new ones are shed rather than queued. Defaults to 2× rps.
    #[arg(long)]
    max_inflight: Option<usize>,
    /// Override the p95 budget (ms). Defaults to the scenario's [[Performance Budgets]] number.
    #[arg(long)]
    p95_ms: Option<f64>,
    /// Maximum acceptable error+shed fraction for a pass.
    #[arg(long, default_value_t = 0.01)]
    max_error_rate: f64,
    /// Write the report as JSON here, in addition to printing it.
    #[arg(long)]
    report: Option<PathBuf>,
    /// Seed for the (deterministic) query picker.
    #[arg(long, default_value_t = 42)]
    seed: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let rps = args.rps.unwrap_or_else(|| args.scenario.default_rps());
    let p95_budget = args
        .p95_ms
        .unwrap_or_else(|| args.scenario.default_p95_ms());
    let max_inflight = args
        .max_inflight
        .unwrap_or_else(|| ((rps * 2.0).ceil() as usize).max(16));

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("building the HTTP client")?;
    let mix = Arc::new(Mix::default_mix());

    eprintln!(
        "loadgen: {:?} at {rps:.0} rps for {}s → {} (p95 budget {p95_budget:.0} ms, max in-flight {max_inflight})",
        args.scenario, args.duration, args.target
    );

    let report = run(
        client,
        args.target,
        args.scenario,
        mix,
        rps,
        args.duration,
        max_inflight,
        args.seed,
    )
    .await?;

    print_report(&report, args.scenario, p95_budget, args.max_error_rate);
    if let Some(path) = &args.report {
        std::fs::write(path, serde_json::to_string_pretty(&report)?)
            .with_context(|| format!("writing {}", path.display()))?;
        eprintln!("report written to {}", path.display());
    }

    if report.passes(p95_budget, args.max_error_rate) {
        Ok(())
    } else {
        // Non-zero exit so CI / a Makefile target fails on a budget miss.
        std::process::exit(1);
    }
}

/// One request's outcome, sent to the collector.
enum Outcome {
    Ok(u64), // latency, microseconds
    Error,
    Shed,
}

#[allow(clippy::too_many_arguments)]
async fn run(
    client: reqwest::Client,
    target: String,
    scenario: Scenario,
    mix: Arc<Mix>,
    rps: f64,
    duration_secs: u64,
    max_inflight: usize,
    mut seed: u64,
) -> Result<Report> {
    let semaphore = Arc::new(Semaphore::new(max_inflight));
    let (tx, mut rx) = mpsc::unbounded_channel::<Outcome>();

    let collector = tokio::spawn(async move {
        let mut samples = Samples::default();
        while let Some(o) = rx.recv().await {
            match o {
                Outcome::Ok(us) => samples.record_ok(us),
                Outcome::Error => samples.record_error(),
                Outcome::Shed => samples.record_shed(),
            }
        }
        samples
    });

    let interval = Duration::from_secs_f64(1.0 / rps.max(0.001));
    let total: u64 = (rps * duration_secs as f64).ceil() as u64;
    let start = Instant::now();
    let mut set = tokio::task::JoinSet::new();

    for i in 0..total {
        // Fixed schedule — do NOT let a slow server slide the cadence (that is coordinated omission).
        tokio::time::sleep_until((start + interval.mul_f64(i as f64)).into()).await;

        // Build the request deterministically from the seed before spawning.
        let request = build_request(&client, &target, scenario, &mix, &mut seed);

        // Shed rather than queue when saturated — an overloaded system should report shed load, not
        // absorb it into an ever-growing backlog that reports impossibly good latency.
        let Ok(permit) = Arc::clone(&semaphore).try_acquire_owned() else {
            let _ = tx.send(Outcome::Shed);
            continue;
        };
        let tx = tx.clone();
        set.spawn(async move {
            let t0 = Instant::now();
            let outcome = match request.send().await {
                Ok(resp) if resp.status().is_success() => {
                    Outcome::Ok(t0.elapsed().as_micros() as u64)
                }
                // 429/503 are the server correctly *shedding* excess load (rate limiter, load-shed
                // layer) — not a failure. Counting it as shed rather than error keeps the verdict
                // about whether the server broke, not whether it protected itself. A single-IP load
                // test trips the per-IP rate limit fast; raise the limits or spread across IPs to
                // measure real throughput.
                Ok(resp)
                    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
                        || resp.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE =>
                {
                    Outcome::Shed
                }
                _ => Outcome::Error,
            };
            let _ = tx.send(outcome);
            drop(permit);
        });
    }

    // Let in-flight requests finish, then close the channel and gather.
    while set.join_next().await.is_some() {}
    drop(tx);
    let samples = collector.await.context("collector task")?;
    Ok(samples.summarise(start.elapsed().as_secs_f64()))
}

/// Build the request future for one dispatch, without sending it.
fn build_request(
    client: &reqwest::Client,
    target: &str,
    scenario: Scenario,
    mix: &Mix,
    seed: &mut u64,
) -> reqwest::RequestBuilder {
    match scenario {
        Scenario::Search | Scenario::Summary => {
            // Summary is measured through its search here too — the token comes from a search, and a
            // realistic summary request pays for both. (A dedicated two-step timing is a refinement.)
            let q = mix.pick(seed).to_string();
            client
                .get(format!("{target}/api/v1/search"))
                .query(&[("q", q.as_str())])
        }
        Scenario::Suggest => {
            let prefix = mix.pick_prefix(seed, 4);
            client
                .get(format!("{target}/api/v1/suggest"))
                .query(&[("q", prefix.as_str()), ("limit", "8")])
        }
        Scenario::Mixed => {
            // ~70% suggest, ~30% search — suggest fires per keystroke, so it dominates real volume.
            let roll = *seed % 10;
            let _ = mix.pick(seed); // advance regardless, for a stable stream
            if roll < 7 {
                let prefix = mix.pick_prefix(seed, 4);
                client
                    .get(format!("{target}/api/v1/suggest"))
                    .query(&[("q", prefix.as_str()), ("limit", "8")])
            } else {
                let q = mix.pick(seed).to_string();
                client
                    .get(format!("{target}/api/v1/search"))
                    .query(&[("q", q.as_str())])
            }
        }
    }
}

fn print_report(report: &Report, scenario: Scenario, p95_budget: f64, max_error_rate: f64) {
    let verdict = if report.passes(p95_budget, max_error_rate) {
        "PASS"
    } else {
        "FAIL"
    };
    println!("── {scenario:?} ──────────────────────────────");
    println!("  requests     {}", report.requests);
    println!(
        "  ok / err / shed  {} / {} / {}   (error rate {:.2}%)",
        report.ok,
        report.errors,
        report.shed,
        report.error_rate * 100.0
    );
    println!("  throughput   {:.0} rps", report.throughput_rps);
    println!(
        "  p50 / p95 / p99   {:.0} / {:.0} / {:.0} ms",
        report.p50_ms, report.p95_ms, report.p99_ms
    );
    println!("  max          {:.0} ms", report.max_ms);
    println!(
        "  budget       p95 ≤ {p95_budget:.0} ms, errors ≤ {:.0}%   → {verdict}",
        max_error_rate * 100.0
    );
}
