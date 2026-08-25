//! `xustive-cli calibrate` — learn ranking weights from external ordering, offline (M7-T07).
//!
//! Two phases, both on the ingestion plane with no user in the loop:
//!
//! - **Capture (T07.1):** for a sample of queries, fetch SearXNG's ranked result domains and record
//!   them as a durable relevance *reference*. Written to a JSONL so a re-run calibrates without
//!   hitting the network again — and so the reference a report was tuned against is auditable.
//! - **Calibrate (T07.2):** retrieve our own candidate pool once per query, then sweep a small grid
//!   of re-ranker side-weights, scoring each against the reference by rank-biased overlap. Report the
//!   best-agreeing vector next to the current default.
//!
//! This is a **tuning signal, never a live input.** The reference is not consulted at query time and
//! nothing here writes config: the output is a recommendation a human reads, weighs, and applies by
//! hand — exactly as `eval-serp` is a yardstick, not a target. The invariant *relevance dominates*
//! is enforced structurally: relevance is held at its default and any candidate whose side-weights
//! sum past the relevance gap is rejected before it is ever scored, so the sweep cannot recommend a
//! ranking where a side signal outweighs the text match.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use xustive_core::Config;
use xustive_ingest::federation::SearxngClient;
use xustive_search::{rank, MeiliClient, Query};

use crate::serp_eval::{domains_of, load_queries, ndcg_vs_reference, rbo, RBO_P};

pub struct CalibrateOptions {
    pub queries: PathBuf,
    /// A previously captured reference JSONL. When set, the capture phase is skipped and SearXNG is
    /// never contacted — the sweep runs against the recorded ordering.
    pub reference: Option<PathBuf>,
    pub out_dir: PathBuf,
    /// How many top domains to compare.
    pub k: usize,
    /// Report only; never write a file.
    pub dry_run: bool,
    pub date: String,
}

/// Pause between SearXNG lookups. The aggregator answers a steady trickle but walls a burst; an
/// offline calibration can afford to pace itself like a person would.
const QUERY_DELAY: Duration = Duration::from_millis(1500);

/// Upper bound on the summed side-weights, the point past which a side signal could overtake the
/// relevance gap across the page. Mirrors the reasoning behind [`rank::Weights::default`] (~0.43 side
/// budget against a ~0.48 relevance gap); the sweep never proposes a vector above it.
const SIDE_BUDGET_MAX: f32 = 0.46;

/// One query's captured external ordering.
#[derive(Serialize, Deserialize)]
pub(crate) struct ReferenceRow {
    pub(crate) query: String,
    /// SearXNG's ranked, de-duplicated top domains.
    pub(crate) domains: Vec<String>,
    /// The hit titles, kept as raw material for synonym co-occurrence mining (M7-T07.3) — the
    /// calibration itself never reads them. Defaulted so captures from before this field load.
    #[serde(default)]
    pub(crate) titles: Vec<String>,
}

#[derive(Serialize, Clone)]
struct Candidate {
    freshness: f32,
    trust: f32,
    authority: f32,
    quality: f32,
    /// Mean rank-biased overlap with the reference across the answered queries.
    mean_rbo: f64,
    /// Mean NDCG of our ordering graded by the reference — a second view on the same agreement.
    mean_ndcg: f64,
}

#[derive(Serialize)]
struct CalibrationReport {
    generated_at: String,
    k: usize,
    queries: usize,
    /// Queries the reference answered (the ones every candidate is scored over).
    answered: usize,
    /// The current shipped default, scored the same way — the thing any recommendation must beat.
    default: Candidate,
    /// The best-agreeing feasible vector found.
    best: Candidate,
    /// Every feasible vector, best first — so a human can see how flat or peaked the surface is
    /// before trusting the winner.
    grid: Vec<Candidate>,
}

pub async fn run(client: &MeiliClient, config: &Config, opts: &CalibrateOptions) -> Result<()> {
    let queries = load_queries(&opts.queries)?;
    if queries.is_empty() {
        anyhow::bail!("no queries in {}", opts.queries.display());
    }
    let k = opts.k.max(1);

    // --- phase 1: the reference ordering (captured, or loaded) ----------------------------------
    let reference = match &opts.reference {
        Some(path) => load_reference(path)?,
        None => capture_reference(config, &queries, k, opts).await?,
    };
    let reference: HashMap<String, Vec<String>> = reference
        .into_iter()
        .filter(|r| !r.domains.is_empty())
        .map(|r| (r.query, r.domains))
        .collect();
    if reference.is_empty() {
        anyhow::bail!(
            "the reference is empty — SearXNG answered nothing, or the capture file has no rows"
        );
    }

    // --- phase 2: retrieve our pool once per query ----------------------------------------------
    let index = client.resolve(&config.search.documents_index).await?;
    let authority = xustive_search::authority::load();
    let trust: HashMap<String, xustive_core::TrustTier> = HashMap::new();
    let now = xustive_core::now_unix();
    let interaction: HashMap<String, f32> = HashMap::new();

    let mut pools: Vec<(&Vec<String>, Vec<Value>)> = Vec::new();
    for q in &queries {
        let Some(ref_domains) = reference.get(q) else {
            continue; // reference had nothing for this query; it cannot be scored.
        };
        let normalized = xustive_text::normalize(q);
        let query = Query::new(&normalized).limit(200);
        let hits = client
            .search::<Value>(&index, &query)
            .await
            .with_context(|| format!("searching for {q:?}"))?;
        pools.push((ref_domains, hits.hits));
    }
    let answered = pools.len();
    if answered == 0 {
        anyhow::bail!("no query had both a reference ordering and a local candidate pool");
    }

    // --- phase 3: sweep side-weights against the reference --------------------------------------
    // Relevance and interaction stay fixed: relevance so "relevance dominates" holds by
    // construction, interaction because an external SERP carries no click signal to calibrate it
    // against. Each of the four remaining side weights is tried at a few multipliers of its default.
    let base = rank::Weights::default();
    let mults = [0.6_f32, 1.0, 1.4, 1.8];
    let grid_dims = |d: f32| mults.iter().map(move |m| d * m).collect::<Vec<_>>();
    // Normalise the scored queries once — the sweep re-ranks the pools hundreds of times and must
    // not re-normalise the same strings each pass. Aligned with `pools` by the same reference filter.
    let normalized = normalized_queries(&queries, &reference);

    let mut grid: Vec<Candidate> = Vec::new();
    for &fr in &grid_dims(base.freshness) {
        for &tr in &grid_dims(base.trust) {
            for &au in &grid_dims(base.authority) {
                for &qu in &grid_dims(base.quality) {
                    // The relevance-dominance guard: reject before scoring.
                    if fr + tr + au + qu + base.interaction > SIDE_BUDGET_MAX {
                        continue;
                    }
                    let w = rank::Weights {
                        freshness: fr,
                        trust: tr,
                        authority: au,
                        quality: qu,
                        ..base.clone()
                    };
                    let (mean_rbo, mean_ndcg) = score_weights(
                        &pools,
                        &normalized,
                        &w,
                        &authority,
                        &trust,
                        &interaction,
                        now,
                        k,
                    );
                    grid.push(Candidate {
                        freshness: fr,
                        trust: tr,
                        authority: au,
                        quality: qu,
                        mean_rbo,
                        mean_ndcg,
                    });
                }
            }
        }
    }
    grid.sort_by(|a, b| {
        b.mean_rbo
            .partial_cmp(&a.mean_rbo)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let default = {
        let (mean_rbo, mean_ndcg) = score_weights(
            &pools,
            &normalized,
            &base,
            &authority,
            &trust,
            &interaction,
            now,
            k,
        );
        Candidate {
            freshness: base.freshness,
            trust: base.trust,
            authority: base.authority,
            quality: base.quality,
            mean_rbo,
            mean_ndcg,
        }
    };
    // The grid always contains the default vector (feasible by construction), so `first` is present.
    let best = grid
        .first()
        .cloned()
        .expect("the weight grid is non-empty — the default vector is always feasible");

    println!(
        "\nCalibrated {answered} answered queries against SearXNG (top-{k} domains, RBO p={RBO_P})\n"
    );
    println!(
        "  default   freshness {:.3} trust {:.3} authority {:.3} quality {:.3}   rbo {:.3}  ndcg {:.3}",
        default.freshness, default.trust, default.authority, default.quality, default.mean_rbo, default.mean_ndcg
    );
    println!(
        "  best      freshness {:.3} trust {:.3} authority {:.3} quality {:.3}   rbo {:.3}  ndcg {:.3}",
        best.freshness, best.trust, best.authority, best.quality, best.mean_rbo, best.mean_ndcg
    );
    let lift = best.mean_rbo - default.mean_rbo;
    if lift <= 1e-4 {
        println!("\n  the default already agrees best — no weight change is recommended.");
    } else {
        println!(
            "\n  recommendation (a tuning signal, apply by hand): +{lift:.3} mean RBO over the default.\n  \
             Verify with `make eval` before shipping — external agreement is not the golden set."
        );
    }

    if !opts.dry_run {
        let report = CalibrationReport {
            generated_at: opts.date.clone(),
            k,
            queries: queries.len(),
            answered,
            default,
            best,
            grid,
        };
        std::fs::create_dir_all(&opts.out_dir)
            .with_context(|| format!("creating {}", opts.out_dir.display()))?;
        let path = opts.out_dir.join(format!("calibration-{}.json", opts.date));
        std::fs::write(&path, serde_json::to_string_pretty(&report)?)
            .with_context(|| format!("writing {}", path.display()))?;
        println!("\nwrote {}", path.display());
    }
    Ok(())
}

/// Pre-normalise the queries once, paired with whether the reference has an entry, so the hot sweep
/// loop does not re-normalise the same strings thousands of times.
fn normalized_queries(queries: &[String], reference: &HashMap<String, Vec<String>>) -> Vec<String> {
    queries
        .iter()
        .filter(|q| reference.contains_key(*q))
        .map(|q| xustive_text::normalize(q))
        .collect()
}

/// Score one weight vector: rerank each query's pre-retrieved pool, take our top-`k` domains, and
/// average the agreement (RBO and reference-graded NDCG) with the captured reference.
#[allow(clippy::too_many_arguments)]
fn score_weights(
    pools: &[(&Vec<String>, Vec<Value>)],
    normalized: &[String],
    weights: &rank::Weights,
    authority: &HashMap<String, f32>,
    trust: &HashMap<String, xustive_core::TrustTier>,
    interaction: &HashMap<String, f32>,
    now: i64,
    k: usize,
) -> (f64, f64) {
    let (mut sum_rbo, mut sum_ndcg) = (0.0, 0.0);
    for ((reference, hits), norm) in pools.iter().zip(normalized) {
        let ranked = rank::rerank(hits, norm, now, trust, authority, interaction, weights);
        let ours = domains_of(
            ranked
                .iter()
                .filter_map(|r| r.hit.get("domain").and_then(Value::as_str)),
            k,
        );
        sum_rbo += rbo(&ours, reference, RBO_P);
        sum_ndcg += ndcg_vs_reference(&ours, reference);
    }
    let n = pools.len() as f64;
    (sum_rbo / n, sum_ndcg / n)
}

/// Fetch SearXNG's ordering for every query and write it as a durable reference (T07.1).
async fn capture_reference(
    config: &Config,
    queries: &[String],
    k: usize,
    opts: &CalibrateOptions,
) -> Result<Vec<ReferenceRow>> {
    let url = config.federation.searxng_url.trim();
    if url.is_empty() {
        anyhow::bail!("federation.searxng_url is not set — cannot capture a reference; pass --reference a captured file instead");
    }
    let searxng = SearxngClient::new(url, 50, Duration::from_secs(20))
        .context("could not build the SearXNG client — check federation.searxng_url")?;

    println!(
        "Capturing SearXNG top-{k} domains for {} queries",
        queries.len()
    );
    let mut rows = Vec::new();
    for (i, q) in queries.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(QUERY_DELAY).await;
        }
        let hits = match searxng.search(q).await {
            Ok(h) => h,
            Err(e) => {
                println!("  {q:<40}  SearXNG error ({e}) — skipped");
                continue;
            }
        };
        let domains = domains_of(hits.iter().map(|h| h.url.as_str()), k);
        let titles: Vec<String> = hits
            .iter()
            .map(|h| h.title.trim().to_string())
            .filter(|t| !t.is_empty())
            .take(k)
            .collect();
        println!("  {q:<40}  {} domains", domains.len());
        rows.push(ReferenceRow {
            query: q.clone(),
            domains,
            titles,
        });
    }

    if !opts.dry_run {
        std::fs::create_dir_all(&opts.out_dir)
            .with_context(|| format!("creating {}", opts.out_dir.display()))?;
        let path = opts
            .out_dir
            .join(format!("external-ref-{}.jsonl", opts.date));
        let body: String = rows
            .iter()
            .map(|r| serde_json::to_string(r).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
        println!("wrote reference {}", path.display());
    }
    Ok(rows)
}

/// Load a previously captured reference JSONL (one [`ReferenceRow`] per line). Shared with the
/// synonym miner (`mine-synonyms`), which reads the same captures for their titles.
pub(crate) fn load_reference(path: &PathBuf) -> Result<Vec<ReferenceRow>> {
    let body =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut rows = Vec::new();
    for (n, line) in body.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let row: ReferenceRow = serde_json::from_str(line).with_context(|| {
            format!("{}: line {} is not a reference row", path.display(), n + 1)
        })?;
        rows.push(row);
    }
    Ok(rows)
}
