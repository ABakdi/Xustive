//! The `discover` command: resolve weak-coverage terms to URLs via Brave (M2-T16.4 + M2-T16.6).
//!
//! This is the actuator half of query-driven discovery. [[weak_coverage]] records which searches the
//! corpus cannot answer, k-anonymously; this reads those terms and asks Brave for URLs that would
//! answer them, seeding the results into the ordinary frontier at a discovered-tier trust tagged
//! `DiscoveryChannel::Brave`. A resolved term is forgotten so the next run does not pay to search it
//! again — if the gap persists, it re-accumulates on its own.
//!
//! Off unless Brave is both enabled and keyed, and every run is capped by the query budget: Brave
//! charges per query, so the ceiling is spend, not politeness.

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

pub async fn run(config: &Config) -> Result<()> {
    let disc = &config.discovery;
    if !disc.brave_usable() {
        println!(
            "brave is disabled (discovery.brave_enabled = {}, key {}). Nothing to do.",
            disc.brave_enabled,
            if disc.brave_api_key.trim().is_empty() {
                "absent"
            } else {
                "present"
            }
        );
        return Ok(());
    }

    let brave = BraveClient::new(&disc.brave_api_key, disc.brave_results_per_query)
        .context("brave key present but client could not be built")?;
    let weak = WeakCoverage::connect_in(
        &config.queue.url,
        "discovery",
        disc.effective_k(),
        Duration::from_secs(disc.weak_coverage_window_days * 86_400),
    )
    .with_context(|| format!("no Redis at {}", config.queue.url))?;
    let frontier = Frontier::connect(&config.queue.url)
        .with_context(|| format!("no Redis at {}", config.queue.url))?;
    let stats = CrawlStats::connect(&config.queue.url);

    // Only the k-anonymous terms, capped at the query budget — this is the spend ceiling.
    let terms = weak.weak_terms(disc.brave_max_queries_per_run).await;
    if terms.is_empty() {
        println!("no weak-coverage terms above the k-anonymity floor; nothing to resolve.");
        return Ok(());
    }

    let mut queries = 0usize;
    let mut queued = 0usize;
    for term in &terms {
        let urls = match brave.search(&term.term).await {
            Ok(u) => u,
            Err(e) => {
                // One failed query does not abort the run — the budget is precious and the other
                // terms are independent. A persistent failure (bad key, quota) shows as every query
                // failing, which the totals make obvious.
                tracing::warn!(error = %e, "a brave query failed; skipping this term");
                queries += 1;
                continue;
            }
        };
        queries += 1;
        for r in &urls {
            if seed_url(&frontier, &r.url).await {
                queued += 1;
                if let Some(s) = &stats {
                    s.incr_channel(DiscoveryChannel::Brave.token(), "discovered", 1)
                        .await;
                }
            }
        }
        // Actioned: forget it so the next run does not pay to search it again. A gap that is still
        // weak will re-accumulate past the floor on its own.
        weak.forget(&term.term).await;
    }

    tracing::info!(queries, queued, "brave discovery finished");
    println!("resolved {queries} weak term(s) via Brave, queued {queued} new URL(s)");
    Ok(())
}

/// Seed one Brave-discovered URL into the frontier under the ordinary rules. Returns whether it was
/// newly queued.
async fn seed_url(frontier: &Frontier, url: &str) -> bool {
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
        source_id: "brave".into(),
        depth: 0,
        trust: DISCOVERED_TRUST,
        channel: DiscoveryChannel::Brave,
        priority: frontier::priority_for(0, DISCOVERED_TRUST, false),
    };
    matches!(frontier.add(&pending).await, Ok(true))
}
