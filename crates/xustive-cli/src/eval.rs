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

    let mut observations = Vec::with_capacity(golden.len());
    for g in golden {
        // The candidate pool, not one page. Recall@50 cannot be measured from ten results, and
        // re-ranking can only reorder what it is given.
        let normalized = xustive_text::normalize(&g.query);
        let pool = config.search.candidate_pool.max(50);
        let query = Query::new(&normalized).limit(pool);
        let mut hits = client
            .search::<Value>(&index, &query)
            .await
            .with_context(|| format!("searching for {:?}", g.id))?;

        // The same expanded leg the API runs. A harness that skips it measures a pipeline
        // nobody uses — which is how the Arabizi failure looked like a ranking problem for a
        // while rather than a missing retrieval step.
        if hits.hits.len() < 5 {
            let detected = detector.detect(&normalized);
            let expansion = expander.expand(&normalized, detected.lang);
            let terms: Vec<String> = expansion
                .variants
                .iter()
                .map(|v| v.text.clone())
                .take(12)
                .collect();
            if !terms.is_empty() {
                let expanded = Query::new(terms.join(" ")).limit(pool);
                if let Ok(extra) = client.search::<Value>(&index, &expanded).await {
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

        let ranked = rank::rerank(
            &hits.hits,
            &normalized,
            now,
            &trust,
            &authority,
            &std::collections::HashMap::new(),
            &weights,
        );
        let results: Vec<String> = ranked
            .iter()
            .filter_map(|r| r.hit.get("id")?.as_str().map(str::to_string))
            .collect();
        let published: Vec<i64> = ranked
            .iter()
            .filter_map(|r| r.hit.get("published_at")?.as_i64())
            .collect();

        observations.push(Observed {
            golden: g,
            results,
            published,
        });
    }

    let mut report = eval::score(&observations, now);
    report.generated_at = opts.date.clone();
    print_report(&report);

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

/// The golden set, plus the corpus size its judgements were made against.
fn load_golden(path: &Path) -> Result<(Vec<GoldenQuery>, Option<usize>)> {
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
    let previous = baseline["ndcg_at_10"].as_f64().unwrap_or(0.0);
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
