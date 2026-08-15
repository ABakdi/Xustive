//! `crawld` — the crawler as a long-running process.
//!
//! The difference between this and `xustive-cli crawl` is not the pipeline, which is shared. It is
//! that this one does not finish: it seeds the frontier once, then claims, fetches and queues in a
//! loop for as long as it is running, and picks up where it left off when restarted because the
//! frontier is in Redis rather than in its memory.
//!
//! # It produces to the queue, it does not index
//!
//! Documents go onto `q:index` and the indexer worker drains them. Two reasons, and the second is
//! the one that matters:
//!
//! 1. Meilisearch prefers batches; the worker already batches, bisects a failing batch, and
//!    dead-letters what will never succeed.
//! 2. **A crawler that indexes directly stops crawling whenever the index is unavailable.** With a
//!    queue between them, an index outage costs a growing backlog rather than a stopped crawl and
//!    a set of pages that were fetched, politely, and then thrown away.
//!
//! # Shutdown
//!
//! `SIGTERM` and Ctrl-C drain: the in-flight fetch finishes, its document is queued, and the loop
//! exits. Nothing is lost and the frontier is left intact, so a restart resumes rather than
//! re-discovering. A crawler that dropped work on shutdown would make restarting expensive for the
//! sites, which is the wrong party to charge.

use std::time::Duration;

use anyhow::{Context, Result};
use xustive_core::Config;
use xustive_ingest::crawl_stats::CrawlStats;
use xustive_ingest::fetch::{FetchConfig, Fetcher};
use xustive_ingest::frontier::Frontier;
use xustive_ingest::orchestrator::{Orchestrator, OrchestratorConfig, Outcome};
use xustive_ingest::robots_cache::RobotsCache;
use xustive_queue::indexer::IndexJob;
use xustive_queue::Queue;

use crate::crawl::parse_seeds;

/// How often progress is logged when nothing else is happening.
///
/// An unattended process that logs nothing is indistinguishable from a stopped one, and the first
/// thing anybody does is check whether it is still alive.
const HEARTBEAT: Duration = Duration::from_secs(60);

/// Queue depth above which the crawler slows down.
///
/// The crawler now outpaces the indexer by a wide margin — sixteen concurrent fetchers fill the
/// queue far faster than a single Meilisearch writer drains it, and the backlog reached
/// twenty-three thousand documents. Nothing was broken, and everything crawled was hours from
/// being searchable.
///
/// Crawling faster than we can index is not throughput, it is a longer queue. Worse, it spends
/// other sites' bandwidth to produce documents that sit in Redis — the one cost in this system
/// paid by somebody else.
const BACKPRESSURE_AT: usize = 5_000;

/// How long to pause when the queue is deep.
///
/// Long enough for the indexer to make real progress, short enough that the crawler resumes
/// promptly once it does. Paused rather than stopped: the frontier and the in-flight claims are
/// untouched, so this costs nothing but time.
const BACKPRESSURE_PAUSE: Duration = Duration::from_secs(10);

/// Concurrent fetch workers.
///
/// **This is the throughput lever, and it costs no politeness at all.** Crawl-delay is per host,
/// and the frontier hands each worker a different host — so twenty workers means twenty hosts in
/// flight while each individual site sees exactly the same one-request-at-a-time pacing it saw
/// before.
///
/// The bottleneck is network latency, not CPU: a fetch is a few hundred milliseconds of waiting
/// and a few milliseconds of parsing. Sequentially, one slow host stalls the entire crawl. There
/// is nothing here a GPU could help with.
///
/// Bounded by hosts, not by cores. Past the number of distinct hosts that are due, extra workers
/// find nothing to claim and idle — so this is set near the seed count and rises with it.
pub const DEFAULT_WORKERS: usize = 16;

pub struct Options {
    pub workers: usize,
    pub seeds_path: String,
    pub max_documents: Option<usize>,
    pub discover_new_hosts: bool,
    /// Start from an empty frontier rather than resuming.
    pub reset: bool,
}

pub async fn run(config: &Config, opts: &Options) -> Result<()> {
    let frontier = Frontier::connect(&config.queue.url)
        .with_context(|| format!("no Redis at {}", config.queue.url))?;
    if opts.reset {
        tracing::warn!("clearing the frontier; this discards discovery and re-fetches from seeds");
        frontier.clear().await;
    }

    let queue = Queue::connect(&config.queue.url, &config.queue.index_stream, "crawld")
        .await
        .context("could not reach the index queue")?;

    let fetcher = Fetcher::new(FetchConfig {
        ignore_politeness: config.crawl.ignore_politeness,
        ..FetchConfig::default()
    })?;
    let fetcher = match RobotsCache::connect(&config.queue.url) {
        Some(cache) => fetcher.with_shared_cache(cache),
        None => fetcher,
    };

    let shared = CrawlStats::connect(&config.queue.url);
    let mut orchestrator = Orchestrator::new(
        fetcher,
        frontier,
        OrchestratorConfig {
            max_documents: opts.max_documents,
            discover_new_hosts: opts.discover_new_hosts,
            // The daemon runs indefinitely, so it is the one that keeps the corpus fresh.
            // `crawl --max N` stays bounded and does not schedule anything.
            revisit: true,
            ..OrchestratorConfig::default()
        },
    );
    if let Some(s) = shared.clone() {
        s.set_state("running").await;
        orchestrator = orchestrator.with_shared_stats(s);
    }
    if let Some(v) = xustive_ingest::revisit::Visits::connect_in(&config.queue.url, "frontier") {
        orchestrator = orchestrator.with_visits(v);
    }

    // Seeded every start, not only the first. `add` is idempotent, so a seed already known is a
    // no-op — and a seed added to the file since the last start would otherwise never be picked up
    // without a reset, which is far too blunt an instrument for "I added a newspaper".
    let tsv = std::fs::read_to_string(&opts.seeds_path)
        .with_context(|| format!("cannot read seeds from {}", opts.seeds_path))?;
    let seeds = parse_seeds(&tsv);
    let mut added = 0usize;
    for seed in &seeds {
        // Trust maps to 0–100 for the priority function, which knows nothing about tiers.
        let trust = match seed.trust {
            xustive_core::TrustTier::A => 100,
            xustive_core::TrustTier::B => 60,
            xustive_core::TrustTier::C => 30,
        };
        if orchestrator.seed(&seed.url, &seed.source_id, trust).await {
            added += 1;
        }
    }
    let (waiting, inflight) = orchestrator.frontier().depth().await;
    tracing::info!(
        seeds = seeds.len(),
        newly_queued = added,
        waiting,
        inflight,
        discover_new_hosts = opts.discover_new_hosts,
        "crawler starting"
    );

    // One orchestrator per worker, sharing the frontier. Claiming is atomic, so they coordinate
    // through Redis rather than through a lock here — which is also what lets workers in separate
    // processes join the same crawl.
    let workers = opts.workers.max(1);
    let mut tasks = tokio::task::JoinSet::new();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let produced = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    for id in 0..workers {
        let frontier = orchestrator.frontier().clone();
        let queue = queue.clone();
        let stop = stop.clone();
        let produced = produced.clone();
        let shared = shared.clone();
        let max = opts.max_documents;
        let discover = opts.discover_new_hosts;
        let ignore_politeness = config.crawl.ignore_politeness;
        let redis_url = config.queue.url.clone();

        tasks.spawn(async move {
            let Ok(fetcher) = Fetcher::new(FetchConfig {
                ignore_politeness,
                ..FetchConfig::default()
            }) else {
                return;
            };
            let fetcher = match RobotsCache::connect(&redis_url) {
                Some(cache) => fetcher.with_shared_cache(cache),
                None => fetcher,
            };
            let mut orch = Orchestrator::new(
                fetcher,
                frontier,
                OrchestratorConfig {
                    // The budget is global, so it is enforced against the shared counter rather
                    // than per worker — sixteen workers each stopping at `max` would collect
                    // sixteen times what was asked for.
                    max_documents: None,
                    discover_new_hosts: discover,
                    revisit: true,
                    ..OrchestratorConfig::default()
                },
            );
            if let Some(s) = shared.clone() {
                orch = orch.with_shared_stats(s);
            }
            if let Some(v) = xustive_ingest::revisit::Visits::connect_in(&redis_url, "frontier") {
                orch = orch.with_visits(v);
            }

            loop {
                if stop.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                // Back off when the indexer is behind.
                //
                // Checked per worker rather than centrally: all sixteen pause together, and it
                // needs no extra shared state. One Redis call against a ten-second pause.
                //
                // Fails **open** — a crawler that stops on a Redis blip is worse than one that
                // briefly runs ahead — but the failure is logged, because a backpressure check
                // that silently never fires is indistinguishable from one that is not there.
                match queue.depth().await {
                    Ok(backlog) if backlog >= BACKPRESSURE_AT => {
                        tracing::debug!(worker = id, backlog, "indexer behind; pausing");
                        tokio::time::sleep(BACKPRESSURE_PAUSE).await;
                        continue;
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!(
                        worker = id,
                        error = %e,
                        "cannot read the queue depth; not pausing"
                    ),
                }

                if let Some(max) = max {
                    if produced.load(std::sync::atomic::Ordering::Relaxed) >= max {
                        return;
                    }
                }

                match orch.step(now_ms()).await {
                    Outcome::Document(parsed) => {
                        let Ok(document) = serde_json::to_value(&parsed.document) else {
                            continue;
                        };
                        if let Err(e) = queue
                            .produce(&IndexJob {
                                document,
                                index: None,
                            })
                            .await
                        {
                            tracing::warn!(worker = id, error = %e, "could not queue a document");
                        } else {
                            produced.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            if let Some(s) = &shared {
                                s.incr("indexed", 1).await;
                            }
                        }
                    }
                    Outcome::Idle => tokio::time::sleep(orch.idle_sleep()).await,
                    Outcome::Finished => return,
                }
            }
        });
    }

    tracing::info!(workers, "workers started");

    // The main task watches for shutdown and reports; the workers do the crawling.
    let mut shutdown = std::pin::pin!(signal());
    let mut ticker = tokio::time::interval(HEARTBEAT);
    ticker.tick().await;
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => {
                tracing::info!("shutting down; in-flight fetches will finish");
                stop.store(true, std::sync::atomic::Ordering::Relaxed);
                break;
            }
            _ = ticker.tick() => {
                let (waiting, inflight) = orchestrator.frontier().depth().await;
                let backlog = queue.depth().await.unwrap_or(0);
                tracing::info!(
                    queued = produced.load(std::sync::atomic::Ordering::Relaxed),
                    waiting,
                    inflight,
                    backlog,
                    workers,
                    "crawling"
                );
                if opts.max_documents.is_some_and(|m| {
                    produced.load(std::sync::atomic::Ordering::Relaxed) >= m
                }) {
                    tracing::info!("document budget reached");
                    stop.store(true, std::sync::atomic::Ordering::Relaxed);
                    break;
                }
            }
            Some(_) = tasks.join_next() => {
                if tasks.is_empty() {
                    break;
                }
            }
        }
    }

    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    // Drain rather than abort: an in-flight fetch finishes and its document is queued, so a
    // restart resumes instead of re-fetching pages we already paid a site for.
    while tasks.join_next().await.is_some() {}

    let produced = produced.load(std::sync::atomic::Ordering::Relaxed);

    // Read from the shared counters rather than any one worker's, since the work was spread.
    let (waiting, inflight) = orchestrator.frontier().depth().await;
    tracing::info!(queued = produced, waiting, inflight, "crawler stopped");
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Ctrl-C or `SIGTERM`.
///
/// `SIGTERM` matters as much as Ctrl-C: it is what a container runtime sends, and a crawler that
/// only handled Ctrl-C would be killed uncleanly on every ordinary deploy.
async fn signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
