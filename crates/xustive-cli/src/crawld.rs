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

/// How often each host's sitemap is polled for freshness (M2-T15.6).
///
/// Hours, not minutes. A sitemap's `lastmod` changes at the pace of publishing, and polling it
/// faster than that spends requests to learn nothing — the same waste the whole freshness design
/// avoids. Six hours keeps a news site's new articles found within a fraction of a day while
/// costing one request per host per poll.
const SITEMAP_POLL_EVERY: Duration = Duration::from_secs(6 * 3600);
/// How often the hot-document re-crawl pulls frequently-clicked pages forward (M6-T06.1).
const HOT_RECRAWL_EVERY: Duration = Duration::from_secs(30 * 60);
/// Cap per pass, so popularity can pull pages forward without owning the frontier.
const HOT_RECRAWL_LIMIT: usize = 200;
/// How often the background query-driven discovery run fires (M2-T16.4).
const DISCOVER_EVERY: Duration = Duration::from_secs(5 * 60);

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

/// Redis memory fraction above which the crawl pauses (PROB-001) — the universal backstop. Every
/// other bound (stream cap, frontier ceiling, host budgets) keeps normal operation far below this;
/// the high-water pause is what makes the OOM wall structurally unreachable by the crawler's own
/// writes even if something new starts growing. Well under 1.0 on purpose: at the wall itself
/// every Redis write already fails and the pause could no longer help.
const MEMORY_HIGH_WATER: f64 = 0.85;

/// How often each worker re-probes the guards (indexer lag, memory). Between probes the last
/// verdict stands — the values change on the scale of seconds, and probing per claim added two
/// Redis round trips to every fetch across 64 workers (PROB-002).
const PROBE_EVERY: Duration = Duration::from_secs(2);

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
/// find nothing to claim and idle — so this rises with the breadth of the frontier. With a
/// frontier spanning hundreds of hosts (steady state, once discovery has run), the old value of
/// 16 was the ceiling: 16 hosts in flight against a 1.5s per-host delay caps throughput near
/// ~9 fetches/sec no matter how much work is queued. Sixty-four lifts that ceiling roughly
/// fourfold while every individual host still sees the same one-at-a-time, delay-respecting
/// pacing — the extra concurrency is spent entirely on *more distinct hosts*, never on hitting
/// one host harder. Each idle worker is a parked async task (a few KB), so overshooting the
/// due-host count costs almost nothing.
pub const DEFAULT_WORKERS: usize = 64;

pub struct Options {
    pub workers: usize,
    pub seeds_path: String,
    /// The data sources registry. Its approved, active sources are seeded alongside the TSV — this
    /// is what makes `registry activate <id>` take effect on the next start. Absent or missing file
    /// is not an error: the registry is optional next to the always-present dev seed list.
    pub registry_path: Option<String>,
    pub max_documents: Option<usize>,
    pub discover_new_hosts: bool,
    /// Start from an empty frontier rather than resuming.
    pub reset: bool,
}

pub async fn run(config: &Config, opts: &Options) -> Result<()> {
    let frontier = Frontier::connect(&config.queue.url)
        .with_context(|| format!("no Redis at {}", config.queue.url))?
        .with_limits(xustive_ingest::frontier::FrontierLimits::from_config(
            &config.crawl,
        ));
    if opts.reset {
        tracing::warn!("clearing the frontier; this discards discovery and re-fetches from seeds");
        frontier.clear().await;
    }

    let queue = Queue::connect_producer(&config.queue.url, &config.queue.index_stream)
        .await
        .context("could not reach the index queue")?
        .with_max_len(config.queue.index_stream_max_len);

    let fetcher = Fetcher::new(FetchConfig {
        ignore_politeness: config.crawl.ignore_politeness,
        ..FetchConfig::default()
    })?;
    let fetcher = match RobotsCache::connect(&config.queue.url) {
        Some(cache) => fetcher.with_shared_cache(cache),
        None => fetcher,
    };

    let shared = CrawlStats::connect(&config.queue.url).await;
    let mut orchestrator = Orchestrator::new(
        fetcher,
        frontier,
        OrchestratorConfig {
            max_documents: opts.max_documents,
            discover_new_hosts: opts.discover_new_hosts,
            // The daemon runs indefinitely, so it is the one that keeps the corpus fresh.
            // `crawl --max N` stays bounded and does not schedule anything.
            revisit: true,
            max_outlinks_per_page: config.crawl.max_outlinks_per_page,
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
    if config.crawl.raw_ttl_days > 0 {
        if let Some(r) = xustive_ingest::raw_store::RawStore::connect_in(
            &config.queue.url,
            "frontier",
            std::time::Duration::from_secs(config.crawl.raw_ttl_days * 86_400),
        ) {
            orchestrator = orchestrator.with_raw_store(r);
        }
    }
    if let Some(g) = xustive_ingest::link_graph::LinkGraphStore::connect(&config.queue.url) {
        orchestrator = orchestrator.with_link_graph(g);
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

    // Also seed the registry's approved, active sources. `is_crawlable` is the gate: a `proposed`
    // or unapproved source is skipped, so nothing crawls until a human ran `registry activate`.
    let mut registry_seeded = 0usize;
    if let Some(reg_path) = &opts.registry_path {
        match xustive_core::Registry::load(reg_path) {
            Ok(reg) => {
                for s in reg.crawlable() {
                    let trust = match s.trust_tier {
                        xustive_core::TrustTier::A => 100,
                        xustive_core::TrustTier::B => 60,
                        xustive_core::TrustTier::C => 30,
                    };
                    for url in &s.entry_points {
                        if orchestrator.seed(url, &s.id, trust).await {
                            added += 1;
                            registry_seeded += 1;
                        }
                    }
                }
            }
            // A missing registry file is fine (dev may run on the TSV alone); a malformed one is
            // worth surfacing, but must not stop a crawl that the TSV can seed on its own.
            Err(e) => tracing::warn!(path = %reg_path, error = %e, "skipping registry seeding"),
        }
    }

    let (waiting, inflight) = orchestrator.frontier().depth().await;
    tracing::info!(
        seeds = seeds.len(),
        registry_seeded,
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

    // Image OCR enrichment, built once and shared (cloned) into every worker. Off unless enabled.
    let media_ocr = if config.media.image_ocr_enabled {
        xustive_ingest::media_ocr::ImageFetcher::new().map(|fetcher| {
            (
                fetcher,
                xustive_ingest::media_ocr::Settings {
                    tessdata: config.media.tessdata_dir.clone(),
                    langs: config.media.ocr_langs.clone(),
                    max_images: config.media.max_images_per_doc,
                    max_bytes: config.media.max_image_bytes,
                },
            )
        })
    } else {
        None
    };

    // Image embedding into the vector index, built once and cloned per worker. Off unless `[vector]`
    // is enabled AND both the embedder and Qdrant clients construct. CLIP embedding runs CPU-only,
    // so this is not GPU-gated — but it still fetches + embeds per image, hence opt-in.
    let image_embed = if config.vector.enabled {
        build_image_embed(config).await
    } else {
        None
    };

    // Text embedding into the vector index for semantic search (M7-T02). Off unless `[vector]
    // text_enabled` AND both the text-embed sidecar and Qdrant clients construct. One embed call per
    // document, so opt-in; fail-open, so a sidecar outage never stalls the crawl.
    let text_embed = if config.vector.text_enabled {
        build_text_embed(config)
    } else {
        None
    };

    for id in 0..workers {
        let frontier = orchestrator.frontier().clone();
        let queue = queue.clone();
        let stop = stop.clone();
        let produced = produced.clone();
        let shared = shared.clone();
        let max = opts.max_documents;
        let discover = opts.discover_new_hosts;
        let ignore_politeness = config.crawl.ignore_politeness;
        let max_outlinks_per_page = config.crawl.max_outlinks_per_page;
        let redis_url = config.queue.url.clone();
        let raw_ttl_days = config.crawl.raw_ttl_days;
        let media_ocr = media_ocr.clone();
        let image_embed = image_embed.clone();
        let text_embed = text_embed.clone();

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
            // A handle of our own for the memory backstop below — `frontier` moves into the
            // orchestrator on the next line.
            let mem_frontier = frontier.clone();
            // Guard-probe state (PROB-002): start expired so the first step probes immediately.
            let mut last_probe = std::time::Instant::now() - PROBE_EVERY;
            let mut probe_paused = false;
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
                    max_outlinks_per_page,
                    ..OrchestratorConfig::default()
                },
            );
            if let Some(s) = shared.clone() {
                orch = orch.with_shared_stats(s);
            }
            if let Some(v) = xustive_ingest::revisit::Visits::connect_in(&redis_url, "frontier") {
                orch = orch.with_visits(v);
            }
            if raw_ttl_days > 0 {
                if let Some(r) = xustive_ingest::raw_store::RawStore::connect_in(
                    &redis_url,
                    "frontier",
                    std::time::Duration::from_secs(raw_ttl_days * 86_400),
                ) {
                    orch = orch.with_raw_store(r);
                }
            }
            if let Some(g) = xustive_ingest::link_graph::LinkGraphStore::connect(&redis_url) {
                orch = orch.with_link_graph(g);
            }
            if let Some((fetcher, settings)) = media_ocr {
                orch = orch.with_media_ocr(fetcher, settings);
            }
            if let Some(embed) = image_embed {
                orch = orch.with_image_embed(embed);
            }
            if let Some(embed) = text_embed {
                orch = orch.with_text_embed(embed);
            }
            let dedup = xustive_ingest::dedup::Dedup::connect_in(&redis_url, "frontier");

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
                // The guard probes (indexer lag + the PROB-001 memory backstop) run at most every
                // couple of seconds per worker, not per step (PROB-002): 64 workers probing on
                // every claim added two Redis round trips per fetch for a value that changes on
                // the scale of seconds. Between probes, the last verdict stands.
                if last_probe.elapsed() >= PROBE_EVERY {
                    last_probe = std::time::Instant::now();
                    probe_paused = false;
                    match queue.depth_of(xustive_queue::INDEXER_GROUP).await {
                        Ok(backlog) if backlog >= BACKPRESSURE_AT => {
                            tracing::debug!(worker = id, backlog, "indexer behind; pausing");
                            probe_paused = true;
                        }
                        Ok(_) => {}
                        Err(e) => tracing::warn!(
                            worker = id,
                            error = %e,
                            "cannot read the queue depth; not pausing"
                        ),
                    }
                    // The operator pause (PROB-003): held deliberately from the console.
                    if !probe_paused {
                        if let Some(st) = &shared {
                            if st.is_paused().await {
                                tracing::debug!(worker = id, "crawl paused by operator");
                                probe_paused = true;
                            }
                        }
                    }
                    // The memory backstop (PROB-001): above the high-water mark, stop producing.
                    // Loud — the old failure mode was every write silently failing at the wall
                    // while the crawler looked merely idle.
                    if !probe_paused {
                        if let Some((used, max)) = mem_frontier.memory_usage().await {
                            if max > 0 && (used as f64 / max as f64) > MEMORY_HIGH_WATER {
                                tracing::warn!(
                                    worker = id,
                                    used_mb = used / 1_048_576,
                                    max_mb = max / 1_048_576,
                                    "redis memory past high water; crawl paused until it drains"
                                );
                                probe_paused = true;
                            }
                        }
                    }
                }
                if probe_paused {
                    tokio::time::sleep(BACKPRESSURE_PAUSE).await;
                    // Re-probe immediately after a pause so recovery is noticed promptly.
                    last_probe = std::time::Instant::now() - PROBE_EVERY;
                    continue;
                }

                if let Some(max) = max {
                    if produced.load(std::sync::atomic::Ordering::Relaxed) >= max {
                        return;
                    }
                }

                match orch.step(now_ms()).await {
                    Outcome::Document(parsed) => {
                        // Cross-run dedup by content hash: the same article reached from a homepage
                        // and a sitemap, or one wire story syndicated to two sites, is queued once.
                        // Fails open — an unreachable dedup store lets the document through rather
                        // than dropping it, since a duplicate write is a no-op and a lost document
                        // is gone.
                        if let Some(d) = &dedup {
                            if !d.is_new(&parsed.document.content_hash).await {
                                if let Some(s) = &shared {
                                    s.incr_skip("duplicate").await;
                                    s.incr_source(&parsed.document.source_id, "duplicate", 1)
                                        .await;
                                    s.incr_channel(
                                        parsed.document.discovery.token(),
                                        "duplicate",
                                        1,
                                    )
                                    .await;
                                }
                                continue;
                            }
                        }
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
                                let doc = &parsed.document;
                                s.incr("indexed", 1).await;
                                s.incr_source(&doc.source_id, "indexed", 1).await;
                                s.incr_channel(doc.discovery.token(), "indexed", 1).await;
                                // spam is 0.0..=1.0; store ×1000 so the integer counter keeps a mean.
                                s.incr_source(
                                    &doc.source_id,
                                    "spam_sum",
                                    (doc.spam_score.clamp(0.0, 1.0) * 1000.0) as u64,
                                )
                                .await;
                                if !doc.is_date_trustworthy() {
                                    s.incr_source(&doc.source_id, "date_unknown", 1).await;
                                }
                            }
                        }
                    }
                    Outcome::Idle => tokio::time::sleep(orch.idle_sleep()).await,
                    Outcome::Finished => return,
                }
            }
        });
    }

    // A sitemap poller runs alongside the workers (M2-T15.6).
    //
    // One sitemap fetch reports on hundreds of URLs, so it is the cheapest freshness signal there
    // is: a page a sitemap says is unchanged need not be revisited at all, and one it says changed
    // is pulled forward. Separate task rather than folded into a worker, because its cadence is
    // hours, not the sub-second loop of a fetch worker, and it must not hold a worker slot idle
    // between polls.
    {
        let frontier = orchestrator.frontier().clone();
        let stop = stop.clone();
        let redis_url = config.queue.url.clone();
        let ignore_politeness = config.crawl.ignore_politeness;
        // Sitemap URL and trust per host, derived from the seeds. The convention `/sitemap.xml`
        // covers most publishers; robots-declared sitemaps are a refinement for later.
        let targets: Vec<(String, u8)> = seeds
            .iter()
            .filter_map(|s| {
                let u = url::Url::parse(&s.url).ok()?;
                let sitemap = format!("{}://{}/sitemap.xml", u.scheme(), u.host_str()?);
                let trust = match s.trust {
                    xustive_core::TrustTier::A => 100,
                    xustive_core::TrustTier::B => 60,
                    xustive_core::TrustTier::C => 30,
                };
                Some((sitemap, trust))
            })
            .collect::<std::collections::BTreeSet<_>>() // dedupe hosts sharing a sitemap
            .into_iter()
            .collect();

        tasks.spawn(async move {
            let Some(visits) = xustive_ingest::revisit::Visits::connect_in(&redis_url, "frontier")
            else {
                return;
            };
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

            let mut ticker = tokio::time::interval(SITEMAP_POLL_EVERY);
            loop {
                ticker.tick().await;
                if stop.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                let now = xustive_core::now_unix();
                for (sitemap, trust) in &targets {
                    if stop.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    let out = xustive_ingest::sitemap_poll::poll_sitemap(
                        &fetcher, &visits, &frontier, sitemap, *trust, now, 5_000,
                    )
                    .await;
                    if out.changed > 0 || out.unchanged > 0 {
                        tracing::info!(
                            %sitemap,
                            changed = out.changed,
                            unchanged = out.unchanged,
                            "sitemap poll"
                        );
                    }
                }
            }
        });
    }

    // Hot-document re-crawl (M6-T06.1): pages people click often are pulled forward for a revisit, so
    // the corpus stays freshest where attention actually is. Gated on interaction being enabled —
    // with it off there is nothing to read. The search plane writes the click counts and the
    // doc→URL map to Redis; this only reads them and defers the URL into the frontier's due set, per
    // the one-way cross-plane rule ([[ADR-0001]]). Popular ≠ owning the queue: it is capped per pass
    // and a revisit answers 304 cheaply when the page has not changed.
    if config.interaction.enabled {
        let frontier = orchestrator.frontier().clone();
        let stop = stop.clone();
        let redis_url = config.queue.signals_url().to_string();
        let salt = config.interaction.salt.clone();
        let k = config.interaction.k_anonymity;
        let hot_floor = config.interaction.hot_floor();
        let window = Duration::from_secs(config.interaction.window_days * 86_400);
        tasks.spawn(async move {
            let Some(interactions) = xustive_ingest::interaction::Interactions::connect_in(
                &redis_url,
                "interaction",
                k,
                window,
                &salt,
            )
            .await
            else {
                return;
            };
            let mut ticker = tokio::time::interval(HOT_RECRAWL_EVERY);
            ticker.tick().await; // skip the immediate first tick so startup is not a burst
            loop {
                ticker.tick().await;
                if stop.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                let hot = interactions
                    .hot_docs_to_recrawl(hot_floor, HOT_RECRAWL_LIMIT)
                    .await;
                let now_ms = xustive_core::now_unix().saturating_mul(1000);
                let mut pulled = 0usize;
                for (_doc, url) in hot {
                    let Ok(u) = url::Url::parse(&url) else {
                        continue;
                    };
                    let host = u.host_str().unwrap_or_default().to_string();
                    if host.is_empty() {
                        continue;
                    }
                    let pending = xustive_ingest::frontier::Pending {
                        url: xustive_ingest::frontier::canonical(&u),
                        host,
                        source_id: "hot".into(),
                        depth: 1,
                        trust: 60,
                        channel: xustive_core::DiscoveryChannel::Link,
                        priority: xustive_ingest::frontier::priority_for(1, 60, true),
                    };
                    frontier.defer(&pending, now_ms).await;
                    pulled += 1;
                }
                if pulled > 0 {
                    tracing::info!(pulled, "hot-doc re-crawl: pulled clicked pages forward");
                }
            }
        });
    }

    // Query-driven discovery in the background (M2-T16.4): every few minutes, resolve the weak
    // search terms users could not get answers for into URLs (via SERP or Brave) and seed them for
    // crawl. Only spawned when a resolution source is configured, so it costs nothing when off.
    if config.discovery.serp_enabled || config.discovery.brave_usable() {
        let cfg = config.clone();
        let stop = stop.clone();
        tasks.spawn(async move {
            let mut ticker = tokio::time::interval(DISCOVER_EVERY);
            ticker.tick().await; // the first tick is immediate; skip it so startup is not a burst
            loop {
                ticker.tick().await;
                if stop.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                if let Err(e) = crate::discover::run(&cfg).await {
                    tracing::warn!(error = %e, "background discovery run failed");
                }
            }
        });
        tracing::info!(
            every_s = DISCOVER_EVERY.as_secs(),
            "background discovery enabled"
        );
    }

    tracing::info!(workers, "workers started");

    // The main task watches for shutdown and reports; the workers do the crawling.
    let mut shutdown = std::pin::pin!(crate::shutdown::signal());
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
                // Reconcile the frontier's global counter from ground truth (PROB-001) — the
                // cheap self-heal for any drift a crash left between a queue write and its INCR.
                let waiting = orchestrator.frontier().reconcile_count().await;
                let (_, inflight) = orchestrator.frontier().depth().await;
                let backlog = queue.depth_of(xustive_queue::INDEXER_GROUP).await.unwrap_or(0);
                let memory_pct = orchestrator
                    .frontier()
                    .memory_usage()
                    .await
                    .filter(|(_, max)| *max > 0)
                    .map(|(used, max)| (used as f64 / max as f64 * 100.0).round() as u64);
                tracing::info!(
                    queued = produced.load(std::sync::atomic::Ordering::Relaxed),
                    waiting,
                    inflight,
                    backlog,
                    redis_memory_pct = memory_pct,
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
    // restart resumes instead of re-fetching pages we already paid a site for. Bounded by the grace
    // period so a wedged fetch cannot keep the process alive past SIGKILL (M4-T02.7).
    crate::shutdown::with_grace("crawler", async {
        while tasks.join_next().await.is_some() {}
    })
    .await;

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

/// Build the image-embedding pass from `[vector]`, or `None` if a client cannot be constructed.
///
/// Returns `None` (rather than failing the crawl) when the embedder or Qdrant client will not
/// build — image embedding is an enrichment, and a crawl must run without it. The Qdrant collection
/// is ensured lazily by the serving side's startup; the crawler only writes.
pub(crate) async fn build_image_embed(
    config: &Config,
) -> Option<xustive_ingest::media_embed::ImageEmbed> {
    let v = &config.vector;
    let timeout = std::time::Duration::from_millis(v.timeout_ms);
    let key = (!v.qdrant_key.is_empty()).then(|| v.qdrant_key.clone());
    let store =
        xustive_vector::Store::new(&v.qdrant_url, key, v.collection.clone(), timeout).ok()?;
    let embedder = xustive_vector::SidecarEmbedder::new(&v.embedder_endpoint, timeout).ok()?;
    let fetcher = xustive_ingest::media_ocr::ImageFetcher::new()?;
    // The reuse cache is optional: a missing Redis (or ttl = 0) just means every image is embedded.
    let cache = if v.embed_cache_ttl_days > 0 {
        xustive_ingest::embed_cache::EmbedCache::connect_in(
            &config.queue.url,
            "frontier",
            std::time::Duration::from_secs(v.embed_cache_ttl_days * 86_400),
        )
        .await
    } else {
        None
    };
    Some(xustive_ingest::media_embed::ImageEmbed {
        fetcher,
        embedder: std::sync::Arc::new(embedder),
        store,
        settings: xustive_ingest::media_embed::Settings {
            max_images: config.media.max_images_per_doc,
            max_bytes: config.media.max_image_bytes,
        },
        cache,
    })
}

/// Build the index-time text embedder (M7-T02), or `None` (crawl continues) if a client fails to
/// build. The Qdrant text collection is ensured by the serving side at startup; the crawler only
/// writes. `text_dim` must match the sidecar's model, or Qdrant rejects the upserts.
fn build_text_embed(config: &Config) -> Option<xustive_ingest::text_embed::TextEmbed> {
    let v = &config.vector;
    let timeout = std::time::Duration::from_millis(v.timeout_ms);
    let key = (!v.qdrant_key.is_empty()).then(|| v.qdrant_key.clone());
    let store = xustive_vector::Store::with_dim(
        &v.qdrant_url,
        key,
        v.text_collection.clone(),
        v.text_dim,
        timeout,
    )
    .ok()?;
    let embedder = xustive_vector::TextEmbedder::new(&v.text_embedder_endpoint, timeout).ok()?;
    Some(xustive_ingest::text_embed::TextEmbed {
        embedder,
        store,
        // The head of a page carries its topic; a few thousand characters is plenty for a retrieval
        // embedding and keeps the per-document embed bounded.
        max_chars: 4000,
    })
}
