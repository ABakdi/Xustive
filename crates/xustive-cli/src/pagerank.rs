//! `xustive-cli pagerank` — compute domain authority from the crawl link graph.
//!
//! Reads the domain link graph the crawler has been accumulating in Redis, runs weighted PageRank
//! over it, maps the raw scores into authority values (`.dz` home floor preserved), and writes them
//! back to Redis for the API to blend with the curated prior at startup. Offline and idempotent:
//! run it after a crawl has covered new ground, restart the API, and earned authority is live.

use anyhow::{Context, Result};
use xustive_core::Config;
use xustive_ingest::link_graph::LinkGraphStore;
use xustive_ingest::pagerank::{self, DEFAULT_DAMPING};
use xustive_search::authority;

pub async fn run(config: &Config) -> Result<()> {
    let store = LinkGraphStore::connect(&config.queue.url)
        .with_context(|| format!("no Redis at {}", config.queue.url))?;

    let graph = store.load_graph().await;
    if graph.node_count() == 0 {
        println!(
            "the link graph is empty — run the crawler (crawld) first so it can record \
             cross-domain links, then compute PageRank."
        );
        return Ok(());
    }
    println!(
        "link graph: {} domains, {} edges",
        graph.node_count(),
        graph.edge_count()
    );

    let raw = graph.pagerank(DEFAULT_DAMPING, 100, 1e-8);
    let scores = pagerank::to_authority(&raw, authority::PAGERANK_CAP, |d| {
        if d == "dz" || d.ends_with(".dz") {
            authority::HOME_FLOOR
        } else {
            authority::BASELINE
        }
    });

    store.store_authority(&scores).await;

    // Show the top domains so a run is legible: this is the earned-authority leaderboard.
    let mut ranked: Vec<(&String, &f32)> = scores.iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
    println!("\ntop domains by earned authority:");
    for (domain, score) in ranked.iter().take(20) {
        println!("  {score:.3}  {domain}");
    }
    println!(
        "\nwrote authority for {} domains to Redis (pagerank:authority). Restart the API to apply.",
        scores.len()
    );
    Ok(())
}
