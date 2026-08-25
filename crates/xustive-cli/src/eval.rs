//! `xustive-cli eval` — score the golden set against the live index.
//!
//! Runs every golden query through the same retrieval and re-ranking path the API uses, then
//! reports the metrics in [`xustive_search::eval`]. Two things it is careful about:
//!
//! - It goes through `MeiliClient` and `rank::rerank` rather than the HTTP API, so a report can
//!   be produced without a running server and so the numbers reflect ranking rather than
//!   middleware.
//! - It writes a dated report and, when a previous one exists, compares against it. A single
//!   number is nearly useless; the delta is the thing worth gating on.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;
use xustive_core::Config;
use xustive_search::eval::{self, GoldenQuery, Observed};
use xustive_search::{rank, MeiliClient, Query};

/// How far nDCG@10 may fall before the gate fails.
///
/// One percent. Tight enough that a real regression is caught, loose enough to absorb the noise
/// from a corpus that changes between runs — which is the normal state of a live index and the
/// reason this is a percentage rather than an absolute.
pub const NDCG_TOLERANCE: f64 = 0.01;

pub struct EvalOptions {
    pub golden: PathBuf,
    pub out_dir: PathBuf,
    /// Compare against this report and fail on a regression.
    pub baseline: Option<PathBuf>,
    /// Report only; never write a file.
    pub dry_run: bool,
    pub date: String,
}

pub async fn run(client: &MeiliClient, config: &Config, opts: &EvalOptions) -> Result<()> {
    let (golden, judged_against) = load_golden(&opts.golden)?;
    if golden.is_empty() {
        anyhow::bail!("no queries in {}", opts.golden.display());
    }

    let index = client.resolve(&config.search.documents_index).await?;
    let trust = HashMap::new();
    let authority = xustive_search::authority::load();
    let weights = rank::Weights::default();
    let now = xustive_core::now_unix();
    let detector = xustive_lang::Detector::default();
    let expander = xustive_lang::Expander::default();

    println!(
        "Scoring {} queries against {index} ({})",
        golden.len(),
        config.search.meili_url
    );

    // Retrieve each query's candidate pool once — Meili is the expensive step. Re-ranking is cheap,
    // so it runs twice below: a baseline, and again with a replayed interaction signal (M6-T09.1).
    let mut retrieved: Vec<(&GoldenQuery, Vec<Value>)> = Vec::with_capacity(golden.len());
    for g in &golden {
        let hits = retrieve_with_expansion(client, config, &index, &detector, &expander, &g.query)
            .await
            .with_context(|| format!("searching for {:?}", g.id))?;
        retrieved.push((g, hits));
    }

    // Pass 1 — baseline re-rank (no interaction), and a replayed click stream over its top results.
    // The stream simulates a **cohort of users per query** so the counts feed the very signal shape
    // runtime serves (BUG-011): `Interactions::ctr_for` returns a per-(query, doc) Wilson lower
    // bound above the k floor, so the replay produces per-query integer impression/click counts and
    // scores them with the same `wilson_lower_bound` — not the global fractional CTR this used to
    // fake, whose magnitude and curve matched nothing in production. Each cohort member impresses
    // every top-10 result; expected clicks scale with the result's true relevance and, mildly, its
    // position — so a relevant-but-mid-ranked document still earns clicks and the signal can lift
    // it, rather than only ever reinforcing the baseline's order.
    //
    // COHORT of 20 = the ADR-0015 k floor, so every shown (query, doc) pair clears `surfaceable`
    // and takes the qd path — which is why the doc-global fallback needs no simulation here.
    const COHORT: u32 = 20;
    let empty: HashMap<String, f32> = HashMap::new();
    // Per-query replayed counts: query index → doc id → (impressions, clicks).
    let mut counts: Vec<HashMap<String, (u32, u32)>> = vec![HashMap::new(); retrieved.len()];
    let mut baseline_obs: Vec<Observed> = Vec::with_capacity(retrieved.len());
    for (qi, (g, hits)) in retrieved.iter().enumerate() {
        let normalized = xustive_text::normalize(&g.query);
        let (results, published) =
            rerank_ids(hits, &normalized, now, &trust, &authority, &empty, &weights);
        for (rank_i, id) in results.iter().take(10).enumerate() {
            let rel = (g.grade(id) as f32 / 3.0).clamp(0.0, 1.0);
            let pos = 1.0 / (rank_i as f32 + 1.0);
            let clicks = (COHORT as f32 * rel * (0.5 + 0.5 * pos)).round() as u32;
            counts[qi].insert(id.clone(), (COHORT, clicks.min(COHORT)));
        }
        baseline_obs.push(Observed {
            golden: (*g).clone(),
            results,
            published,
        });
    }

    // Pass 2 — re-rank again, each query under its own replayed signal, exactly as the API feeds
    // `ctr_for(query, docs)` into the ranker at runtime.
    let mut interaction_obs: Vec<Observed> = Vec::with_capacity(retrieved.len());
    for (qi, (g, hits)) in retrieved.iter().enumerate() {
        let normalized = xustive_text::normalize(&g.query);
        let interaction: HashMap<String, f32> = counts[qi]
            .iter()
            .map(|(id, &(imp, clk))| {
                (
                    id.clone(),
                    xustive_ingest::interaction::wilson_lower_bound(clk, imp),
                )
            })
            .collect();
        let (results, published) = rerank_ids(
            hits,
            &normalized,
            now,
            &trust,
            &authority,
            &interaction,
            &weights,
        );
        interaction_obs.push(Observed {
            golden: (*g).clone(),
            results,
            published,
        });
    }
    let interaction_report = eval::score(&interaction_obs, now);

    let mut report = eval::score(&baseline_obs, now);
    report.generated_at = opts.date.clone();
    print_report(&report);

    // The interaction uplift (M6-T09.1) + the guardrail (T09.2). A replayed click stream should lift
    // the relevant documents people would click, and — since interaction only reorders — must never
    // raise the zero-result rate.
    let uplift = interaction_report.ndcg_at_10 - report.ndcg_at_10;
    println!();
    println!("Interaction replay (M6-T09):");
    println!("  nDCG@10 baseline       {:.4}", report.ndcg_at_10);
    println!(
        "  nDCG@10 + interaction  {:.4}",
        interaction_report.ndcg_at_10
    );
    println!("  uplift                 {uplift:+.4}");
    println!(
        "  zero-result guardrail  {:.4} → {:.4}{}",
        report.zero_result_rate,
        interaction_report.zero_result_rate,
        if interaction_report.zero_result_rate > report.zero_result_rate {
            "  ⚠ interaction raised zero-results (should be impossible — it only reorders)"
        } else {
            "  ok"
        }
    );

    let live = client
        .stats(&index)
        .await
        .map(|s| s.number_of_documents)
        .unwrap_or(0);
    let drifted = check_corpus_drift(judged_against, live as usize);

    if let Some(baseline) = &opts.baseline {
        if drifted {
            println!(
                "\n  Skipping the regression gate: the corpus changed, so the comparison would\n  \
                 measure the crawl rather than the ranker."
            );
        } else {
            gate(&report, baseline)?;
        }
    }

    if !opts.dry_run {
        std::fs::create_dir_all(&opts.out_dir)?;
        let path = opts.out_dir.join(format!("{}.json", opts.date));
        std::fs::write(&path, serde_json::to_string_pretty(&report)?)?;
        println!("\nwrote {}", path.display());
    }
    Ok(())
}

/// One query through the same retrieval the API runs (BUG-003): the primary leg with ranking
/// scores and the spam filter, the all-stop-word phrase rescue (M7-T01.5), then — when the primary
/// found too little *or* its best hit is weak (M7-T01.3) — the expanded leg, merged and
/// deduplicated by id. A harness that skips any of these measures a pipeline nobody uses, which is
/// how the Arabizi failure looked like a ranking problem for a while rather than a missing
/// retrieval step. The trigger helpers live in `xustive-search` and are the very ones the API
/// calls, so the two cannot drift again. Shared with the settings A/B (`eval-ab`).
pub(crate) async fn retrieve_with_expansion(
    client: &MeiliClient,
    config: &Config,
    index: &str,
    detector: &xustive_lang::Detector,
    expander: &xustive_lang::Expander,
    query: &str,
) -> Result<Vec<Value>> {
    use xustive_search::filter::{Filters, SPAM_THRESHOLD};
    use xustive_search::rank::top_result_is_weak;
    use xustive_search::settings::is_all_stop_words;

    let normalized = xustive_text::normalize(query);
    let pool = config.search.candidate_pool.max(50);
    // The default-filter spam clause the API applies to every search.
    let spam_filter = Filters {
        exclude_spam: true,
        ..Filters::default()
    }
    .to_expression(SPAM_THRESHOLD);

    let mut q = Query::new(&normalized).limit(pool).ranking_score(true);
    if let Some(expr) = spam_filter.clone() {
        q = q.filter(expr);
    }
    let mut hits = client.search::<Value>(index, &q).await?;

    // The stop-word phrase rescue (M7-T01.5), exactly as the API runs it.
    if hits.hits.is_empty() && is_all_stop_words(&normalized) {
        let phrase = format!("\"{normalized}\"");
        let mut retry = Query::new(&phrase).limit(pool).ranking_score(true);
        if let Some(expr) = spam_filter.clone() {
            retry = retry.filter(expr);
        }
        if let Ok(recovered) = client.search::<Value>(index, &retry).await {
            hits = recovered;
        }
    }

    // Few *or weak* (M7-T01.3) — the same condition, via the same shared helper, the API uses.
    if hits.hits.len() < 5 || top_result_is_weak(&hits.hits) {
        let detected = detector.detect(&normalized);
        let expansion = expander.expand(&normalized, detected.lang);
        let terms: Vec<String> = expansion
            .variants
            .iter()
            .map(|v| v.text.clone())
            .take(12)
            .collect();
        if !terms.is_empty() {
            let mut expanded = Query::new(terms.join(" ")).limit(pool);
            if let Some(expr) = spam_filter {
                expanded = expanded.filter(expr);
            }
            if let Ok(extra) = client.search::<Value>(index, &expanded).await {
                let mut seen: std::collections::HashSet<String> = hits
                    .hits
                    .iter()
                    .filter_map(|h| h.get("id")?.as_str().map(str::to_string))
                    .collect();
                for hit in extra.hits {
                    if let Some(id) = hit.get("id").and_then(Value::as_str) {
                        if seen.insert(id.to_string()) {
                            hits.hits.push(hit);
                        }
                    }
                }
            }
        }
    }
    Ok(hits.hits)
}

/// The golden set, plus the corpus size its judgements were made against.
pub(crate) fn load_golden(path: &Path) -> Result<(Vec<GoldenQuery>, Option<usize>)> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    let mut queries = Vec::new();
    let mut corpus_size = None;
    for line in text.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let value: Value = serde_json::from_str(line).context("parsing a golden line")?;
        if let Some(meta) = value.get("_meta") {
            corpus_size = meta["corpus_size"].as_u64().map(|n| n as usize);
            continue;
        }
        queries.push(serde_json::from_value(value).context("parsing a golden query")?);
    }
    Ok((queries, corpus_size))
}

/// Warn when the index has moved since the judgements were made.
///
/// Judgements are frozen; the index is not. Every document added afterwards is relevant to some
/// query and judged for none of them, so it counts as irrelevant and drags nDCG down. That is a
/// bigger corpus, not a worse ranker, and a gate that cannot tell the difference will be turned
/// off the first time it fires on a successful crawl.
///
/// The specification calls for a frozen index snapshot. This is the honest approximation:
/// detect the drift and refuse to pretend the comparison is valid.
fn check_corpus_drift(judged_against: Option<usize>, live: usize) -> bool {
    let Some(judged) = judged_against else {
        println!("  note: this golden set records no corpus size; comparisons may be unsound");
        return false;
    };
    if judged == live {
        return false;
    }
    let drift = (live as f64 - judged as f64) / judged.max(1) as f64;
    println!(
        "\n  ⚠ corpus drift: judged against {judged} documents, index now has {live} \
         ({}{:.0}%).",
        if drift >= 0.0 { "+" } else { "" },
        drift * 100.0
    );
    println!(
        "    Judgements are frozen, so documents added since count as irrelevant and pull\n             nDCG down. Regenerate with `make golden` before treating this as a regression."
    );
    true
}

/// Re-rank one query's candidate hits and return the ordered document ids and their publish dates.
/// Factored out so the harness can re-rank the same retrieved pool twice — baseline and with a
/// replayed interaction signal — without re-fetching from Meili.
pub(crate) fn rerank_ids(
    hits: &[Value],
    normalized: &str,
    now: i64,
    trust: &HashMap<String, xustive_core::TrustTier>,
    authority: &HashMap<String, f32>,
    interaction: &HashMap<String, f32>,
    weights: &rank::Weights,
) -> (Vec<String>, Vec<i64>) {
    let ranked = rank::rerank(
        hits,
        normalized,
        now,
        trust,
        authority,
        interaction,
        weights,
    );
    let results = ranked
        .iter()
        .filter_map(|r| r.hit.get("id")?.as_str().map(str::to_string))
        .collect();
    let published = ranked
        .iter()
        .filter_map(|r| r.hit.get("published_at")?.as_i64())
        .collect();
    (results, published)
}

fn print_report(r: &eval::Report) {
    println!();
    println!("  nDCG@10          {:.4}", r.ndcg_at_10);
    println!("  MRR@10           {:.4}", r.mrr_at_10);
    println!("  recall@50        {:.4}", r.recall_at_50);
    println!("  zero-result      {:.1}%", r.zero_result_rate * 100.0);
    println!("  queries          {}", r.queries);
    if r.unjudged > 0 {
        println!(
            "  unjudged         {} (excluded from nDCG and recall)",
            r.unjudged
        );
    }

    println!("\n  by language");
    let mut langs: Vec<_> = r.by_language.iter().collect();
    langs.sort_by(|a, b| a.0.cmp(b.0));
    for (lang, s) in langs {
        println!(
            "    {lang:6} n={:<4} nDCG@10 {:.4}   zero {:.0}%",
            s.queries,
            s.ndcg_at_10,
            s.zero_result_rate * 100.0
        );
    }

    println!("\n  freshness of returned documents");
    let mut buckets: Vec<_> = r.freshness.iter().collect();
    buckets.sort_by(|a, b| a.0.cmp(b.0));
    for (bucket, n) in buckets {
        println!("    {bucket:10} {n}");
    }

    if r.machine_judged > 0 {
        let share = r.machine_judged as f64 / r.queries.max(1) as f64;
        println!(
            "\n  {:.0}% of these queries are machine-judged. This report detects regressions;\n  \
             it does not measure quality. See eval/README.md.",
            share * 100.0
        );
    }
}

/// Fail if nDCG@10 dropped by more than the tolerance.
fn gate(current: &eval::Report, baseline_path: &Path) -> Result<()> {
    let text = std::fs::read_to_string(baseline_path)
        .with_context(|| format!("reading baseline {}", baseline_path.display()))?;
    let baseline: Value = serde_json::from_str(&text)?;
    // A baseline without the field is a wrong file, not a baseline of zero (BUG-019): defaulting
    // to 0.0 made every current score a pass, so one mis-pointed `--baseline` in CI turned the
    // gate permanently green. Refuse loudly instead.
    let previous = baseline["ndcg_at_10"].as_f64().with_context(|| {
        format!(
            "{} has no numeric `ndcg_at_10` — is this an eval report? (ab-*/serp-*/calibration-* \
             reports are not baselines)",
            baseline_path.display()
        )
    })?;
    let delta = current.ndcg_at_10 - previous;

    println!(
        "\n  baseline {:.4} → {:.4}  ({}{:.4})",
        previous,
        current.ndcg_at_10,
        if delta >= 0.0 { "+" } else { "" },
        delta
    );

    // Relative, not absolute: a one-point drop means something different at 0.9 than at 0.2, and
    // the number this gate protects will move as the corpus grows.
    let allowed = previous * NDCG_TOLERANCE;
    if delta < -allowed {
        anyhow::bail!(
            "nDCG@10 fell by {:.4}, more than the {:.1}% tolerance ({:.4}).\n\
             If this change is intended, re-baseline deliberately — the point of the gate is \
             that a ranking regression cannot arrive unnoticed.",
            -delta,
            NDCG_TOLERANCE * 100.0,
            allowed
        );
    }
    println!("  ✓ within the {:.0}% tolerance", NDCG_TOLERANCE * 100.0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tolerance_is_tight_enough_to_catch_a_real_regression() {
        // A gate loose enough to pass anything is worse than no gate: it is a green tick that
        // means nothing, and people stop reading it.
        const { assert!(NDCG_TOLERANCE <= 0.02) };
        const { assert!(NDCG_TOLERANCE > 0.0) };
    }

    #[test]
    fn corpus_drift_is_detected_and_a_matching_corpus_is_not() {
        // The gate must not fire on a successful crawl. A gate that cries wolf when the corpus
        // grows is a gate somebody turns off, and then it is not protecting anything.
        assert!(check_corpus_drift(Some(100), 250), "growth must be flagged");
        assert!(check_corpus_drift(Some(250), 100), "shrinkage too");
        assert!(!check_corpus_drift(Some(100), 100), "a match must not warn");
    }

    #[test]
    fn golden_queries_parse_from_jsonl() {
        let dir = std::env::temp_dir().join("xustive-eval-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("g.jsonl");
        std::fs::write(
            &path,
            "# a comment line\n\
             {\"id\":\"q1\",\"query\":\"الجزائر\",\"lang\":\"ar\",\"judgements\":{\"d1\":3}}\n\
             \n",
        )
        .unwrap();

        let loaded = load_golden(&path).unwrap();
        assert_eq!(loaded.0.len(), 1, "comments and blank lines are skipped");
        assert_eq!(loaded.0[0].grade("d1"), 3);
        assert_eq!(
            loaded.0[0].judged_by,
            xustive_search::Provenance::Machine,
            "an unlabelled set must default to machine, never to human"
        );
        let _ = std::fs::remove_file(path);
    }
}
