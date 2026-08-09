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

pub struct Options {
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
            ..OrchestratorConfig::default()
        },
    );
    if let Some(s) = shared.clone() {
        s.set_state("running").await;
        orchestrator = orchestrator.with_shared_stats(s);
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

    let mut shutdown = std::pin::pin!(signal());
    let mut last_beat = std::time::Instant::now();
    let mut produced = 0usize;

    loop {
        let outcome = tokio::select! {
            // Biased so a shutdown signal is seen even while work is always available, which is
            // the normal state of a healthy crawler.
            biased;
            _ = &mut shutdown => {
                tracing::info!("shutting down; the in-flight fetch will finish");
                break;
            }
            outcome = orchestrator.step(now_ms()) => outcome,
        };

        match outcome {
            Outcome::Document(parsed) => {
                if let Some(s) = &shared {
                    s.incr("indexed", 1).await;
                }
                let document = serde_json::to_value(&parsed.document)?;
                // A failure to queue is not a reason to stop. The document is lost, which is a
                // shame; stopping would lose every document after it too.
                if let Err(e) = queue
                    .produce(&IndexJob {
                        document,
                        index: None,
                    })
                    .await
                {
                    tracing::warn!(error = %e, "could not queue a document");
                } else {
                    produced += 1;
                }
            }
            Outcome::Idle => {
                tokio::time::sleep(orchestrator.idle_sleep()).await;
            }
            Outcome::Finished => {
                tracing::info!("document budget reached");
                break;
            }
        }

        if last_beat.elapsed() > HEARTBEAT {
            let (waiting, inflight) = orchestrator.frontier().depth().await;
            let s = orchestrator.stats();
            tracing::info!(
                queued = produced,
                fetched = s.fetched,
                parsed = s.parsed,
                discovered = s.discovered,
                failed = s.failed,
                waiting,
                inflight,
                "crawling"
            );
            last_beat = std::time::Instant::now();
        }
    }

    // Recorded before the summary, so the console reflects reality even if this process is killed
    // between the two.
    if let Some(s) = &shared {
        s.set_state("stopped").await;
    }

    let s = orchestrator.stats();
    let mut skips: Vec<(&&str, &usize)> = s.skipped.iter().collect();
    skips.sort_by(|a, b| b.1.cmp(a.1));
    tracing::info!(
        queued = produced,
        fetched = s.fetched,
        parsed = s.parsed,
        discovered = s.discovered,
        failed = s.failed,
        skipped = ?skips,
        "crawler stopped"
    );
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
