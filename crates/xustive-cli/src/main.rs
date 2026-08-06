//! `xustive-cli` — operator tooling.
//!
//! `migrate` applies index settings from code, `seed` loads a corpus, `text` explains what the
//! normaliser actually does to a string. That last one exists because "why does this query match
//! nothing" is almost always a normalisation question.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde_json::Value;

use xustive_core::Config;
use xustive_search::{settings, MeiliClient};

#[derive(Parser, Debug)]
#[command(name = "xustive-cli", about = "Xustive operator tooling")]
struct Args {
    #[arg(long, env = "XUSTIVE_CONFIG", default_value = "config/dev.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Create indexes and apply settings. Idempotent.
    Migrate {
        /// Report differences without writing anything.
        #[arg(long)]
        check: bool,
    },
    /// Index documents from a JSON or JSON-Lines file.
    Seed {
        #[arg(long, default_value = "tests/fixtures/corpus/documents.jsonl")]
        path: PathBuf,
        #[arg(long, default_value_t = 1000)]
        batch: usize,
    },
    /// Show index document counts.
    Stats,
    /// Show what normalisation does to a string.
    Text { input: String },
    /// Run a search from the command line.
    Search {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .compact()
        .init();

    let args = Args::parse();
    let config = Config::load(Some(&args.config)).context("loading config")?;

    // `text` is pure and must work with no backend running.
    if let Command::Text { input } = &args.command {
        return cmd_text(input);
    }

    let client = MeiliClient::new(
        &config.search.meili_url,
        &config.search.meili_key,
        Duration::from_secs(30),
    )
    .context("building search client")?;

    match args.command {
        Command::Migrate { check } => cmd_migrate(&client, check).await,
        Command::Seed { path, batch } => cmd_seed(&client, &config, &path, batch).await,
        Command::Stats => cmd_stats(&client, &config).await,
        Command::Search { query, limit } => cmd_search(&client, &config, &query, limit).await,
        Command::Text { .. } => unreachable!("handled above"),
    }
}

async fn cmd_migrate(client: &MeiliClient, check: bool) -> Result<()> {
    for (index, primary_key, desired) in settings::all() {
        if check {
            let live = client
                .get_settings(index)
                .await
                .with_context(|| format!("reading settings for {index}"))?;
            let drift = diff_settings(&desired, &live);
            if drift.is_empty() {
                println!("✓ {index}: settings match");
            } else {
                println!("✗ {index}: {} setting(s) differ", drift.len());
                for key in drift {
                    println!("    - {key}");
                }
            }
            continue;
        }

        client
            .ensure_index(index, primary_key)
            .await
            .with_context(|| format!("creating index {index}"))?;
        client
            .apply_settings(index, &desired)
            .await
            .with_context(|| format!("applying settings to {index}"))?;
        println!("✓ {index}: created and configured");
    }
    Ok(())
}

/// Compare the settings we declare against what the server reports.
///
/// Only keys we set are compared — the server fills in many defaults we do not care about, and
/// reporting those as drift would make the check useless.
fn diff_settings(desired: &Value, live: &Value) -> Vec<String> {
    let Some(map) = desired.as_object() else {
        return Vec::new();
    };
    let mut drift = Vec::new();
    for (key, want) in map {
        match live.get(key) {
            Some(have) if values_equal(want, have) => {}
            _ => drift.push(key.clone()),
        }
    }
    drift
}

/// Semantic comparison of a declared setting against the live one.
///
/// Three normalisations are needed, each for a real reason observed against a live server:
///
/// - **Strings are folded.** Meilisearch normalises word lists itself — it lowercases Latin and
///   folds ta marbuta, so a declared `بجاية` comes back as `بجايه` and `Oran` as `oran`. Comparing
///   raw would report permanent drift. (That the engine's folding matches [`xustive_text::fold`]
///   is a useful independent check on our own normalisation.)
/// - **Arrays are multisets.** The server does not preserve declaration order.
/// - **Objects are compared on declared keys only.** The server fills nested defaults we never
///   set, and those are not drift.
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(x), Value::String(y)) => xustive_text::fold(x) == xustive_text::fold(y),
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return false;
            }
            let mut xs: Vec<String> = x.iter().map(canonical).collect();
            let mut ys: Vec<String> = y.iter().map(canonical).collect();
            xs.sort();
            ys.sort();
            xs == ys
        }
        (Value::Object(x), Value::Object(y)) => x
            .iter()
            .all(|(k, want)| y.get(k).is_some_and(|have| values_equal(want, have))),
        _ => a == b,
    }
}

/// Order-independent key for array comparison, with the same string folding as `values_equal`.
fn canonical(v: &Value) -> String {
    match v {
        Value::String(s) => xustive_text::fold(s),
        other => other.to_string(),
    }
}

async fn cmd_seed(
    client: &MeiliClient,
    config: &Config,
    path: &PathBuf,
    batch_size: usize,
) -> Result<()> {
    let text = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("reading corpus {}", path.display()))?;

    let docs: Vec<Value> = if path.extension().is_some_and(|e| e == "jsonl") {
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .enumerate()
            .map(|(i, l)| {
                serde_json::from_str(l).with_context(|| format!("line {} is not valid JSON", i + 1))
            })
            .collect::<Result<_>>()?
    } else {
        serde_json::from_str(&text).context("corpus is not a JSON array")?
    };

    if docs.is_empty() {
        println!("corpus is empty, nothing to do");
        return Ok(());
    }

    let index = &config.search.documents_index;
    let total = docs.len();
    let mut indexed = 0usize;

    for chunk in docs.chunks(batch_size) {
        let uid = client
            .add_documents(index, chunk)
            .await
            .context("submitting batch")?;
        // Wait for the task before acknowledging progress: reporting success for a batch that
        // later fails is worse than being slow.
        client.wait_task(uid).await.context("indexing batch")?;
        indexed += chunk.len();
        println!("  indexed {indexed}/{total}");
    }

    println!("✓ seeded {indexed} documents into {index}");
    Ok(())
}

async fn cmd_stats(client: &MeiliClient, config: &Config) -> Result<()> {
    for index in [
        &config.search.documents_index,
        &config.search.comments_index,
    ] {
        match client.stats(index).await {
            Ok(s) => println!(
                "{index}: {} documents{}",
                s.number_of_documents,
                if s.is_indexing { " (indexing)" } else { "" }
            ),
            Err(e) => println!("{index}: unavailable ({e})"),
        }
    }
    Ok(())
}

fn cmd_text(input: &str) -> Result<()> {
    use xustive_text::script;

    let normalized = xustive_text::normalize(input);
    let folded = xustive_text::fold(input);

    println!("input      {input:?}");
    println!("normalized {normalized:?}");
    println!("folded     {folded:?}");
    println!("script     {:?}", script::detect(&normalized));
    println!(
        "tokens     {:?}",
        xustive_text::tokens(&normalized).collect::<Vec<_>>()
    );
    println!(
        "chars      {} -> {}",
        input.chars().count(),
        normalized.chars().count()
    );
    println!("hash       {}", xustive_core::hash::content_hash(input));
    match xustive_core::hash::simhash(input) {
        Some(h) => println!("simhash    {}", xustive_core::hash::simhash_hex(h)),
        None => println!("simhash    (text too short to be meaningful)"),
    }
    Ok(())
}

async fn cmd_search(
    client: &MeiliClient,
    config: &Config,
    query: &str,
    limit: usize,
) -> Result<()> {
    let normalized = xustive_text::normalize(query);
    let q = xustive_search::Query::new(&normalized).limit(limit);
    let hits = client
        .search::<Value>(&config.search.documents_index, &q)
        .await?;

    println!("query      {query:?}");
    println!("normalized {normalized:?}");
    println!(
        "hits       {} (in {} ms)\n",
        hits.estimated_total_hits, hits.processing_time_ms
    );

    for (i, hit) in hits.hits.iter().enumerate() {
        let title = hit
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("(no title)");
        let url = hit.get("url").and_then(Value::as_str).unwrap_or("");
        let src = hit
            .get("source_type")
            .and_then(Value::as_str)
            .unwrap_or("?");
        println!("{:>3}. [{src}] {title}", i + 1);
        println!("     {url}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn diff_reports_only_keys_we_declare() {
        let desired = json!({ "stopWords": ["a"], "rankingRules": ["words"] });
        // The server returns extra keys we never set; those are not drift.
        let live = json!({
            "stopWords": ["a"],
            "rankingRules": ["words"],
            "somethingElse": 42
        });
        assert!(diff_settings(&desired, &live).is_empty());
    }

    #[test]
    fn diff_detects_a_changed_value() {
        let desired = json!({ "stopWords": ["a", "b"] });
        let live = json!({ "stopWords": ["a"] });
        assert_eq!(diff_settings(&desired, &live), vec!["stopWords"]);
    }

    #[test]
    fn diff_detects_a_missing_key() {
        let desired = json!({ "rankingRules": ["words"] });
        assert_eq!(diff_settings(&desired, &json!({})), vec!["rankingRules"]);
    }

    #[test]
    fn array_comparison_ignores_order() {
        // Meilisearch does not preserve declaration order for every array-valued setting.
        assert!(values_equal(&json!(["a", "b"]), &json!(["b", "a"])));
        assert!(!values_equal(&json!(["a", "b"]), &json!(["a"])));
    }

    #[test]
    fn scalar_comparison_is_exact() {
        assert!(values_equal(&json!(4), &json!(4)));
        assert!(!values_equal(&json!(4), &json!(5)));
        assert!(values_equal(&json!({"oneTypo": 4}), &json!({"oneTypo": 4})));
        assert!(!values_equal(
            &json!({"oneTypo": 4}),
            &json!({"oneTypo": 5})
        ));
    }

    #[test]
    fn strings_compare_after_folding() {
        // Observed against a live server: Meilisearch folds ta marbuta and lowercases Latin in
        // word lists, so the value it returns is not the value we declared.
        assert!(values_equal(&json!("بجاية"), &json!("بجايه")));
        assert!(values_equal(&json!("عنابة"), &json!("عنابه")));
        assert!(values_equal(&json!("Oran"), &json!("oran")));
        // Genuinely different words are still different.
        assert!(!values_equal(&json!("وهران"), &json!("قسنطينة")));
    }

    #[test]
    fn word_lists_survive_server_side_folding() {
        let declared = json!(["وهران", "بجاية", "Oran", "CNAS"]);
        let live = json!(["oran", "cnas", "وهران", "بجايه"]);
        assert!(
            values_equal(&declared, &live),
            "folding + reordering should not read as drift"
        );
    }

    #[test]
    fn nested_objects_compare_on_declared_keys_only() {
        // The server fills in nested defaults we never set; those are not drift.
        let declared = json!({ "enabled": true, "minWordSizeForTypos": { "oneTypo": 4 } });
        let live = json!({
            "enabled": true,
            "minWordSizeForTypos": { "oneTypo": 4, "twoTypos": 9 },
            "disableOnAttributes": []
        });
        assert!(values_equal(&declared, &live));
    }

    #[test]
    fn nested_drift_is_still_detected() {
        let declared = json!({ "minWordSizeForTypos": { "oneTypo": 4 } });
        let live = json!({ "minWordSizeForTypos": { "oneTypo": 5 } });
        assert!(
            !values_equal(&declared, &live),
            "a real nested change must be reported"
        );
    }

    #[test]
    fn a_missing_nested_key_is_drift() {
        let declared = json!({ "disableOnAttributes": ["entities"] });
        assert!(!values_equal(&declared, &json!({})));
    }
}
