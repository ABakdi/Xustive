//! The `commoncrawl` command: bootstrap the frontier from a Common Crawl snapshot (M2-T16.1–.3).
//!
//! Reads one snapshot's CDX index for Algerian hosts and seeds the discovered URLs into the ordinary
//! frontier at a discovered-tier trust, tagged `DiscoveryChannel::CommonCrawl`. Everything after
//! that — robots, politeness, `SafeUrl`, dedup — is the crawler's ordinary path; this only supplies
//! the list. Resumable: it records the last page finished and continues from there on a restart.

use std::time::Duration;

use anyhow::{Context, Result};

use xustive_core::{Config, DiscoveryChannel};
use xustive_ingest::commoncrawl::{self, AlgeriaFilter, CcProgress, CdxClient};
use xustive_ingest::crawl_stats::CrawlStats;
use xustive_ingest::frontier::{self, Frontier, Pending};

/// Trust for a Common-Crawl-discovered URL: low, below every curated tier. It is a plausible URL
/// from a third-party index, not a source we vouch for — so it is reached after seeds and links,
/// and its own outlinks inherit the low rating.
const DISCOVERED_TRUST: u8 = 20;

pub struct Options {
    pub index: String,
    pub pattern: String,
    pub registry_path: String,
    pub max_pages: Option<usize>,
    pub restart: bool,
    /// Seconds to wait between index pages — politeness to the CDX server, not to the sites.
    pub page_delay_secs: u64,
}

pub async fn run(config: &Config, opts: &Options) -> Result<()> {
    let filter = build_filter(&opts.registry_path)?;
    let client = CdxClient::new(&opts.index).context("building the CDX client")?;
    let frontier = Frontier::connect(&config.queue.url)
        .with_context(|| format!("no Redis at {}", config.queue.url))?;
    let stats = CrawlStats::connect(&config.queue.url).await;
    let progress = CcProgress::connect_in(&config.queue.url, "discovery");

    // Where the index says to stop, if it says. When it does not, the loop stops on the first empty
    // page instead.
    let total_pages = client
        .num_pages(&opts.pattern)
        .await
        .context("querying the snapshot page count")?;
    if let Some(n) = total_pages {
        tracing::info!(index = %opts.index, pattern = %opts.pattern, pages = n, "snapshot has pages");
        if n == 0 {
            tracing::info!("no captures for this pattern; nothing to do");
            return Ok(());
        }
    }

    // Resume, unless told to restart. `last` is the last page fully ingested, so start at `last + 1`.
    let mut start = 0;
    if !opts.restart {
        if let Some(p) = &progress {
            if let Some(last) = p.last_page(&opts.index, &opts.pattern).await {
                start = last + 1;
                tracing::info!(resume_from = start, "resuming from saved progress");
            }
        }
    }

    let mut page = start;
    let mut queued = 0usize;
    let mut pages_done = 0usize;
    loop {
        if let Some(n) = total_pages {
            if page >= n {
                break;
            }
        }
        if opts.max_pages.is_some_and(|m| pages_done >= m) {
            tracing::info!(max = ?opts.max_pages, "reached the page cap");
            break;
        }

        let text = match client.fetch_page(&opts.pattern, page).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(page, error = %e, "cdx page failed; stopping");
                break;
            }
        };
        // Empty page with an unknown total means we have walked off the end.
        if text.trim().is_empty() && total_pages.is_none() {
            break;
        }

        let urls = commoncrawl::select_urls(&text, &filter);
        for url in &urls {
            if seed_url(&frontier, url).await {
                queued += 1;
                if let Some(s) = &stats {
                    s.incr_channel(DiscoveryChannel::CommonCrawl.token(), "discovered", 1)
                        .await;
                }
            }
        }

        // Progress is written *after* the page's URLs are queued, so a crash re-does this page
        // rather than skipping it. Re-doing is safe — the frontier dedups.
        if let Some(p) = &progress {
            p.set_last_page(&opts.index, &opts.pattern, page).await;
        }
        pages_done += 1;
        tracing::info!(
            page,
            kept = urls.len(),
            queued_total = queued,
            "ingested a page"
        );

        page += 1;
        if opts.page_delay_secs > 0 {
            tokio::time::sleep(Duration::from_secs(opts.page_delay_secs)).await;
        }
    }

    tracing::info!(
        index = %opts.index,
        pattern = %opts.pattern,
        pages = pages_done,
        queued,
        "common crawl bootstrap finished"
    );
    println!(
        "queued {queued} new URLs from {pages_done} page(s) of {}",
        opts.index
    );
    Ok(())
}

/// Build the Algeria host filter from the registry's entry-point hosts, so "which non-`.dz` hosts
/// are Algerian" is the same curated answer the crawler already uses.
fn build_filter(registry_path: &str) -> Result<AlgeriaFilter> {
    let hosts = match xustive_core::Registry::load(registry_path) {
        Ok(reg) => reg
            .sources()
            .iter()
            .flat_map(|s| s.entry_points.iter())
            .filter_map(|u| xustive_core::model::domain_of(u))
            .collect::<Vec<_>>(),
        Err(e) => {
            // Not fatal: the `.dz` rule still works without the known-host list. But say so, since a
            // missing registry means every `.com` Algerian outlet is silently skipped.
            tracing::warn!(path = registry_path, error = %e, "no registry; only .dz will be kept");
            Vec::new()
        }
    };
    Ok(AlgeriaFilter::new(hosts))
}

/// Add one discovered URL to the frontier under the ordinary rules: `SafeUrl`-parseable, not a
/// trap, tagged as Common-Crawl-discovered at a low trust. Returns whether it was newly queued.
async fn seed_url(frontier: &Frontier, url: &str) -> bool {
    // `SafeUrl` is enforced again at fetch time; parsing here drops the obviously bad before they
    // take a frontier slot.
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
        source_id: "commoncrawl".into(),
        depth: 0,
        trust: DISCOVERED_TRUST,
        channel: DiscoveryChannel::CommonCrawl,
        priority: frontier::priority_for(0, DISCOVERED_TRUST, false),
    };
    matches!(frontier.add(&pending).await, Ok(true))
}
