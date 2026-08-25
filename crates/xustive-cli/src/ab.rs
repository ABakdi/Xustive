//! `xustive-cli eval-ab` — the settings A/B review (M7-T01.4).
//!
//! Ranking rules and searchable-attribute order live inside Meilisearch, so unlike the re-ranker
//! weights they cannot be swept offline against a retrieved pool: each variant has to be **applied**
//! to the index and the golden set scored through it. This command does exactly that, in order:
//! snapshot the live settings, apply each variant, score the golden set through the same
//! retrieve→expand→re-rank path `eval` uses, then **restore the snapshot** — the index leaves this
//! command configured exactly as it entered.
//!
//! The output is a measured table, not a change: settings in code (`xustive_search::settings`) are
//! the source of truth, so a winning variant is applied by editing `settings.rs` and running
//! `make migrate`, never by leaving the index in the variant state. Run it against the dev index —
//! it temporarily reconfigures whatever index it points at, and a searchable-attribute variant
//! triggers a full background reindex per apply.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;

use xustive_core::Config;
use xustive_search::eval::{self, Observed};
use xustive_search::{rank, settings, MeiliClient};

use crate::eval::{load_golden, rerank_ids, retrieve_with_expansion};

pub struct AbOptions {
    pub golden: PathBuf,
    pub out_dir: PathBuf,
    /// Report only; never write a file.
    pub dry_run: bool,
    pub date: String,
}

/// How long one settings apply may take. A searchable-attributes variant reindexes the whole
/// corpus, which on this hardware overruns the client's 300s default — the first run died exactly
/// there. An offline A/B can afford to wait for the truth.
const APPLY_WAIT: Duration = Duration::from_secs(1800);

/// One settings variant: a name, the reasoning, and the settings object to apply.
struct Variant {
    name: &'static str,
    why: &'static str,
    settings: Value,
}

#[derive(Serialize)]
struct VariantScore {
    name: String,
    why: String,
    ndcg_at_10: f64,
    mrr_at_10: f64,
    recall_at_50: f64,
    zero_result_rate: f64,
}

#[derive(Serialize)]
struct AbReport {
    generated_at: String,
    queries: usize,
    variants: Vec<VariantScore>,
}

/// The variants under review. Each is the shipped settings with exactly one deliberate difference,
/// so a score delta is attributable to that difference and nothing else.
fn variants() -> Vec<Variant> {
    let base = settings::documents_settings();

    // Post-morphology hypothesis 1: entity matches deserve to outrank excerpt matches. The
    // expansion leg now generates Arabic variants that often hit entity names, and T03 made
    // entities richer — if a query names a thing, the page *about* that thing should beat a page
    // that merely mentions it in the excerpt.
    let mut entities_up = base.clone();
    entities_up["searchableAttributes"] = serde_json::json!([
        "title",
        "entities",
        "excerpt",
        "body",
        "media.ocr_text",
        "translit_body",
        "author.name"
    ]);

    // Post-morphology hypothesis 2: with variant terms in the query, exactness may need to be
    // decided before attribute position — otherwise a generated variant matching the title can
    // outrank the exact form matching the body.
    let mut exact_early = base.clone();
    exact_early["rankingRules"] = serde_json::json!([
        "words",
        "typo",
        "proximity",
        "exactness",
        "attribute",
        "sort",
        "published_at:desc",
        "quality_score:desc"
    ]);

    // Post-morphology hypothesis 3: the aggressive oneTypo=4 threshold existed because Arabic
    // roots are short and nothing else bridged their variant forms. Morphology now does that at
    // query time, so the engine may no longer need typo matching on 4-character words — where
    // "وهران"↔"إيران" style near-misses live.
    let mut typo5 = base.clone();
    typo5["typoTolerance"]["minWordSizeForTypos"]["oneTypo"] = serde_json::json!(5);

    vec![
        Variant {
            name: "baseline",
            why: "the shipped settings, scored first so every delta is against the same run",
            settings: base,
        },
        Variant {
            name: "entities-above-excerpt",
            why: "entity matches outrank excerpt matches in the attribute rule",
            settings: entities_up,
        },
        Variant {
            name: "exactness-before-attribute",
            why: "exact matches beat attribute position, so expansion variants cannot outrank the exact form",
            settings: exact_early,
        },
        Variant {
            name: "typo-min-5",
            why: "no typo matching on 4-char words now that morphology bridges Arabic variants",
            settings: typo5,
        },
    ]
}

pub async fn run(client: &MeiliClient, config: &Config, opts: &AbOptions) -> Result<()> {
    let (golden, _) = load_golden(&opts.golden)?;
    if golden.is_empty() {
        anyhow::bail!("no queries in {}", opts.golden.display());
    }
    let index = client.resolve(&config.search.documents_index).await?;

    // The restore target is what the index actually runs now, not what the code says it should —
    // if the two have drifted, this command must not be the thing that silently "fixes" it.
    let snapshot = client
        .get_settings(&index)
        .await
        .context("snapshotting the live settings")?;

    println!(
        "A/B over {} golden queries on {index} — each variant is applied, scored, and the\n\
         original settings restored afterwards. Settings in code stay the source of truth:\n\
         apply a winner by editing settings.rs + `make migrate`, not by keeping the variant.\n",
        golden.len()
    );

    // Score every variant, but never leave without restoring — a scoring error must not strand the
    // index in a variant configuration, so the loop collects its error instead of returning early.
    let mut scores: Vec<VariantScore> = Vec::new();
    let mut failed: Option<anyhow::Error> = None;
    for v in variants() {
        print!("  {:<28}", v.name);
        if let Err(e) = client
            .apply_settings_within(&index, &v.settings, APPLY_WAIT)
            .await
        {
            failed = Some(anyhow::Error::new(e).context(format!("applying variant {:?}", v.name)));
            break;
        }
        match score_current(client, config, &index, &golden).await {
            Ok(r) => {
                println!(
                    "nDCG@10 {:.4}  MRR {:.4}  recall@50 {:.4}  zero {:.4}",
                    r.ndcg_at_10, r.mrr_at_10, r.recall_at_50, r.zero_result_rate
                );
                scores.push(VariantScore {
                    name: v.name.to_string(),
                    why: v.why.to_string(),
                    ndcg_at_10: r.ndcg_at_10,
                    mrr_at_10: r.mrr_at_10,
                    recall_at_50: r.recall_at_50,
                    zero_result_rate: r.zero_result_rate,
                });
            }
            Err(e) => {
                failed = Some(e.context(format!("scoring variant {:?}", v.name)));
                break;
            }
        }
    }

    // Restore unconditionally. If the restore ALSO fails, the original failure must not vanish
    // behind it (BUG-016) — which variant broke, and why, is the information the operator needs.
    if let Err(restore_err) = client
        .apply_settings_within(&index, &snapshot, APPLY_WAIT)
        .await
    {
        let restore_err = anyhow::Error::new(restore_err).context(
            "restoring the original settings — the index may be left in a variant state; re-run `make migrate`",
        );
        return Err(match failed {
            Some(original) => {
                original.context(format!("and then the restore failed too: {restore_err:#}"))
            }
            None => restore_err,
        });
    }
    println!("\n  original settings restored.");

    // The verdict and the report are produced from whatever completed, *before* any failure is
    // surfaced (BUG-017): a late variant failure must not discard hours of finished reindex+scoring.
    // The verdict, in deltas against the baseline run. "Wins" means past the same relative
    // tolerance the regression gate uses — a hair above baseline is noise, not a win (BUG-018).
    if let Some(baseline) = scores.iter().find(|s| s.name == "baseline") {
        let base_ndcg = baseline.ndcg_at_10;
        let noise = base_ndcg * crate::eval::NDCG_TOLERANCE;
        println!("\n  against baseline (nDCG@10 {:.4}):", base_ndcg);
        for s in scores.iter().filter(|s| s.name != "baseline") {
            let d = s.ndcg_at_10 - base_ndcg;
            println!(
                "    {:<28}{:+.4}{}",
                s.name,
                d,
                if d > noise { "  ← wins" } else { "" }
            );
        }
        println!(
            "\n  Keep only measured wins; a delta within noise (±{noise:.4}, {:.0}% of baseline) is not a win.",
            crate::eval::NDCG_TOLERANCE * 100.0
        );
    }

    if !opts.dry_run && !scores.is_empty() {
        let report = AbReport {
            generated_at: opts.date.clone(),
            queries: golden.len(),
            variants: scores,
        };
        std::fs::create_dir_all(&opts.out_dir)
            .with_context(|| format!("creating {}", opts.out_dir.display()))?;
        let path = opts.out_dir.join(format!("ab-{}.json", opts.date));
        std::fs::write(&path, serde_json::to_string_pretty(&report)?)
            .with_context(|| format!("writing {}", path.display()))?;
        println!("\nwrote {}", path.display());
    }
    if let Some(e) = failed {
        return Err(e);
    }
    Ok(())
}

/// Score the golden set through whatever settings the index currently has — the same retrieval
/// (primary + expansion legs) and the same re-rank (default weights, no interaction) `eval` uses,
/// so an `eval-ab` baseline row and a `make eval` run are directly comparable.
async fn score_current(
    client: &MeiliClient,
    config: &Config,
    index: &str,
    golden: &[xustive_search::eval::GoldenQuery],
) -> Result<eval::Report> {
    let detector = xustive_lang::Detector::default();
    let expander = xustive_lang::Expander::default();
    let trust: HashMap<String, xustive_core::TrustTier> = HashMap::new();
    let authority = xustive_search::authority::load();
    let interaction: HashMap<String, f32> = HashMap::new();
    let weights = rank::Weights::default();
    let now = xustive_core::now_unix();

    let mut observed: Vec<Observed> = Vec::with_capacity(golden.len());
    for g in golden {
        let hits = retrieve_with_expansion(client, config, index, &detector, &expander, &g.query)
            .await
            .with_context(|| format!("searching for {:?}", g.id))?;
        let normalized = xustive_text::normalize(&g.query);
        let (results, published) = rerank_ids(
            &hits,
            &normalized,
            now,
            &trust,
            &authority,
            &interaction,
            &weights,
        );
        observed.push(Observed {
            golden: g.clone(),
            results,
            published,
        });
    }
    Ok(eval::score(&observed, now))
}
