//! The `discover` command: resolve weak-coverage terms to URLs (M2-T16.4/.6/.9).
//!
//! This is the actuator half of query-driven discovery. [[weak_coverage]] records which searches the
//! corpus cannot answer, k-anonymously; this reads those terms, asks a search source for URLs that
//! would answer them, and seeds the results into the ordinary frontier at a discovered-tier trust,
//! tagged with the channel that found them. A resolved term is forgotten so the next run does not
//! re-resolve it — if the gap persists, it re-accumulates on its own.
//!
//! Two sources, chosen by config: **direct SERP scraping** ([[serp]], `serp_enabled`) is preferred
//! per ADR-0013, and **Brave** ([[brave]]) is the API fallback. Off unless one is enabled, and every
//! run is capped by a query budget.

use std::time::Duration;

use anyhow::{Context, Result};

use xustive_core::{Config, DiscoveryChannel};
use xustive_ingest::brave::BraveClient;
use xustive_ingest::crawl_stats::CrawlStats;
use xustive_ingest::frontier::{self, Frontier, Pending};
use xustive_ingest::weak_coverage::WeakCoverage;

/// Trust for a Brave-discovered URL: the same low discovered-tier as Common Crawl. It answers a real
/// query, but it is a third-party pick, not a source we vouch for.
const DISCOVERED_TRUST: u8 = 20;

/// Where weak terms get resolved to URLs. Direct SERP scraping is preferred when enabled; Brave is
/// the API fallback.
enum Source {
    Serp(xustive_ingest::serp::SerpClient),
    Brave(BraveClient),
}

impl Source {
    /// The discovery channel a URL from this source is tagged with.
    fn channel(&self) -> DiscoveryChannel {
        match self {
            Source::Serp(_) => DiscoveryChannel::Serp,
            Source::Brave(_) => DiscoveryChannel::Brave,
        }
    }

    /// Resolve one term to result URLs.
    async fn resolve(&self, term: &str) -> Vec<String> {
        match self {
            Source::Serp(c) => c.search(term).await,
            Source::Brave(c) => match c.search(term).await {
                Ok(rs) => rs.into_iter().map(|r| r.url).collect(),
                Err(e) => {
                    tracing::warn!(error = %e, "a brave query failed; skipping this term");
                    Vec::new()
                }
            },
        }
    }
}

pub async fn run(config: &Config) -> Result<()> {
    let disc = &config.discovery;

    // Direct SERP scraping wins when enabled (my direction, ADR-0013); Brave is the API fallback.
    let (source, budget) = if disc.serp_enabled {
        let engines = disc
            .serp_engines
            .iter()
            .filter_map(|s| xustive_ingest::serp::Engine::parse(s))
            .collect();
        let proxy = Some(disc.serp_proxy.as_str()).filter(|s| !s.trim().is_empty());
        let client = xustive_ingest::serp::SerpClient::new(engines, proxy).context(
            "could not build the SERP client (check discovery.serp_proxy — a malformed proxy URL \
             fails the build)",
        )?;
        (Source::Serp(client), disc.serp_max_queries_per_run)
    } else if disc.brave_usable() {
        let client = BraveClient::new(&disc.brave_api_key, disc.brave_results_per_query)
            .context("brave key present but client could not be built")?;
        (Source::Brave(client), disc.brave_max_queries_per_run)
    } else {
        println!(
            "discovery resolution is off. Enable direct SERP (discovery.serp_enabled = true) or \
             Brave (discovery.brave_enabled + brave_api_key). Nothing to do."
        );
        return Ok(());
    };

    let weak = WeakCoverage::connect_in(
        config.queue.signals_url(),
        "discovery",
        disc.effective_k(),
        Duration::from_secs(disc.weak_coverage_window_days * 86_400),
    )
    .with_context(|| format!("no signals Redis at {}", config.queue.signals_url()))?;
    let frontier = Frontier::connect(&config.queue.url)
        .with_context(|| format!("no Redis at {}", config.queue.url))?;
    let stats = CrawlStats::connect(&config.queue.url).await;

    // Only the k-anonymous terms, capped at the query budget — this is the spend ceiling.
    let terms = weak.weak_terms(budget).await;
    if terms.is_empty() {
        println!("no weak-coverage terms above the k-anonymity floor; nothing to resolve.");
        return Ok(());
    }

    let channel = source.channel();
    let mut queries = 0usize;
    let mut queued = 0usize;
    let mut unresolved = 0usize;
    for term in &terms {
        let urls = source.resolve(&term.term).await;
        queries += 1;
        let mut queued_here = 0usize;
        for url in &urls {
            if seed_url(&frontier, url, channel).await {
                queued_here += 1;
                if let Some(s) = &stats {
                    s.incr_channel(channel.token(), "discovered", 1).await;
                }
            }
        }
        queued += queued_here;
        if queued_here > 0 {
            // Actioned: forget it so the next run does not resolve it again. A gap that is still
            // weak re-accumulates past the floor on its own.
            weak.forget(&term.term).await;
        } else {
            // The source gave us nothing crawlable — an engine challenge/anomaly page, or every
            // result was a trap or already known. Keep the term weak so it stays visible on the
            // dashboard and the next run retries it (e.g. once a proxy makes the engines answer).
            // The coverage window expires it eventually if it is never satisfied.
            unresolved += 1;
        }
    }

    tracing::info!(
        queries,
        queued,
        unresolved,
        source = channel.token(),
        "discovery finished"
    );
    if unresolved > 0 {
        tracing::warn!(
            unresolved,
            source = channel.token(),
            "some weak terms resolved to no crawlable URL — likely the engine served a bot \
             challenge page; direct SERP is being blocked from this IP (needs a proxy)"
        );
    }
    println!(
        "resolved {} of {queries} weak term(s) via {}, queued {queued} new URL(s){}",
        queries - unresolved,
        channel.token(),
        if unresolved > 0 {
            format!(", {unresolved} left unresolved (engine returned no results)")
        } else {
            String::new()
        }
    );
    Ok(())
}

/// Seed one discovered URL into the frontier under the ordinary rules, tagged with the channel that
/// found it so the document it becomes carries the right provenance. Returns whether it was newly
/// queued.
async fn seed_url(frontier: &Frontier, url: &str, channel: DiscoveryChannel) -> bool {
    let Ok(safe) = xustive_core::SafeUrl::parse(url) else {
        return false;
    };
    let parsed = safe.as_url().clone();
    let host = parsed.host_str().unwrap_or_default().to_string();
    if host.is_empty() || frontier::detect_trap(&parsed).is_some() {
        return false;
    }
    let pending = Pending {
        url: frontier::canonical(&parsed),
        host,
        source_id: channel.token().into(),
        depth: 0,
        trust: DISCOVERED_TRUST,
        channel,
        priority: frontier::priority_for(0, DISCOVERED_TRUST, false),
    };
    matches!(frontier.add(&pending).await, Ok(true))
}
