//! `xustive-cli` — operator tooling.
//!
//! `migrate` applies index settings from code, `seed` loads a corpus, `text` explains what the
//! normaliser actually does to a string. That last one exists because "why does this query match
//! nothing" is almost always a normalisation question.

mod crawl;
mod crawld;
mod eval;
mod worker;

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
    /// Run the crawler continuously, resuming from the shared frontier.
    Crawld {
        #[arg(long, default_value = "data/sources/seeds.tsv")]
        seeds: PathBuf,
        /// Stop after this many documents. Omit to run until stopped.
        #[arg(long)]
        max: Option<usize>,
        /// Follow links to hosts not in the seed list. Off by default — this is the difference
        /// between crawling the seeds and crawling the web.
        #[arg(long)]
        discover: bool,
        /// Start from an empty frontier instead of resuming.
        #[arg(long)]
        reset: bool,
        /// Concurrent fetch workers. Politeness is per host, so this costs none of it.
        #[arg(long, default_value_t = crawld::DEFAULT_WORKERS)]
        workers: usize,
    },
    /// Crawl real sites from a seed list and index what they publish.
    Crawl {
        /// Seed file: `source_id <TAB> url <TAB> trust`.
        #[arg(long, default_value = "data/sources/seeds.tsv")]
        seeds: PathBuf,
        /// Only crawl this source id.
        #[arg(long)]
        source: Option<String>,
        #[arg(long, default_value_t = 60)]
        per_source: usize,
        #[arg(long, default_value_t = 500)]
        max: usize,
        /// Fetch only listed entry points; do not follow homepage links.
        #[arg(long)]
        no_discover: bool,
    },
    /// Create the scoped Meilisearch keys the running services use. Idempotent.
    Keys {
        /// Print the key values. Off by default so a routine run cannot leave credentials in a
        /// terminal scrollback or a CI log.
        #[arg(long)]
        show: bool,
    },
    /// Score the golden set against the live index.
    Eval {
        #[arg(long, default_value = "eval/golden/v1.jsonl")]
        golden: PathBuf,
        #[arg(long, default_value = "eval/reports")]
        out: PathBuf,
        /// Compare against this report and fail on a regression beyond the tolerance.
        #[arg(long)]
        baseline: Option<PathBuf>,
        /// Print the report without writing it.
        #[arg(long)]
        dry_run: bool,
    },
    /// Drain the index queue into Meilisearch.
    Worker {
        /// Process what is queued and exit, rather than running continuously.
        #[arg(long)]
        once: bool,
    },
    /// Inspect or replay the dead-letter queue.
    Dlq {
        /// `stats`, `peek` or `replay`.
        #[arg(default_value = "stats")]
        action: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
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
        Command::Crawld {
            seeds,
            max,
            discover,
            reset,
            workers,
        } => {
            crawld::run(
                &config,
                &crawld::Options {
                    workers,
                    seeds_path: seeds.display().to_string(),
                    max_documents: max,
                    discover_new_hosts: discover,
                    reset,
                },
            )
            .await
        }
        Command::Crawl {
            seeds,
            source,
            per_source,
            max,
            no_discover,
        } => {
            let tsv = tokio::fs::read_to_string(&seeds)
                .await
                .with_context(|| format!("reading seeds {}", seeds.display()))?;
            let mut list = crawl::parse_seeds(&tsv);
            if let Some(want) = source {
                list.retain(|s| s.source_id == want);
                if list.is_empty() {
                    anyhow::bail!("no seed with source id {want:?}");
                }
            }
            let opts = crawl::CrawlOptions {
                max_pages_per_source: per_source,
                max_total: max,
                discover_links: !no_discover,
                ..Default::default()
            };
            crawl::run(&client, &config, &list, &opts).await.map(|_| ())
        }
        Command::Eval {
            golden,
            out,
            baseline,
            dry_run,
        } => {
            let opts = eval::EvalOptions {
                golden,
                out_dir: out,
                baseline,
                dry_run,
                date: today(),
            };
            eval::run(&client, &config, &opts).await
        }
        Command::Worker { once } => worker::run(&config, &client, once).await,
        Command::Dlq { action, limit } => worker::dlq(&config, &action, limit).await,
        Command::Keys { show } => cmd_keys(&client, show).await,
        Command::Stats => cmd_stats(&client, &config).await,
        Command::Search { query, limit } => cmd_search(&client, &config, &query, limit).await,
        Command::Text { .. } => unreachable!("handled above"),
    }
}

async fn cmd_migrate(client: &MeiliClient, check: bool) -> Result<()> {
    for (alias, primary_key, desired) in settings::all() {
        // Fresh installs get `documents_v1`; existing ones keep whatever they already have.
        // Everything reads through `resolve`, so this is invisible to callers and is what makes
        // a live reindex possible later — build vN+1 alongside, verify, flip.
        //
        // Never renames an existing plain index. Doing so would need a copy of every document,
        // and a half-finished migration would leave a search engine that has forgotten
        // everything, which does not look like a migration problem from the outside.
        let resolved = client.resolve(alias).await?;
        let index = if resolved != alias {
            resolved
        } else if client.index_exists(alias).await? {
            println!("  {alias}: pre-alias index in place, left as it is");
            alias.to_string()
        } else {
            format!("{alias}_v1")
        };
        let index = index.as_str();

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

/// Today, as `YYYY-MM-DD`.
///
/// Hand-rolled from the epoch rather than pulling a date crate in for one call. Reports are named
/// by day, which is the only granularity that makes a series readable.
fn today() -> String {
    let secs = xustive_core::now_unix();
    let days = secs.div_euclid(86_400) + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Provision the scoped keys.
///
/// The master key can delete every index. `xustive-api` only searches and the worker only
/// writes, so neither should hold it — and after this runs, the master key is needed by exactly
/// one thing: this migration tool.
async fn cmd_keys(client: &MeiliClient, show: bool) -> Result<()> {
    // A Meilisearch started without MEILI_MASTER_KEY has no key API at all, and the 401 it
    // returns says nothing about what to do. Development runs that way by default, so this is
    // the common case rather than an edge one.
    if let Err(e) = client.find_key("probe").await {
        let msg = e.to_string();
        if msg.contains("without a master key") {
            anyhow::bail!(
                "Meilisearch is running without a master key, so scoped keys cannot be \
                 created.\n\n  Set MEILI_MASTER_KEY in .env, run `make dev-down && make \
                 dev-up`, then re-run this.\n  Development defaults to no master key, which \
                 is why this is not already set."
            );
        }
        return Err(e).context("listing existing keys");
    }

    for spec in [&xustive_search::SEARCH_KEY, &xustive_search::INDEX_KEY] {
        let existed = client.find_key(spec.name).await?.is_some();
        let key = client
            .ensure_key(spec)
            .await
            .with_context(|| format!("provisioning key {}", spec.name))?;

        println!(
            "{} {}  {}",
            if existed { "=" } else { "✓" },
            spec.name,
            spec.description
        );
        println!("    actions: {}", key.actions.join(", "));
        if show {
            println!("    key:     {}", key.key);
        }
    }

    if show {
        println!();
        println!("Put the search key in MEILI_KEY for xustive-api, and the index key in the");
        println!("worker's environment. Keep the master key out of both.");
    } else {
        println!();
        println!("Re-run with --show to print the key values.");
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

    // Resolve, never write to the bare alias.
    //
    // Meilisearch creates a missing index on first write, so submitting to `documents` when only
    // `documents_v1` exists silently manufactures a second, keyless index. Inference then has to
    // guess a primary key from the batch, sees both `id` and `source_id`, and fails the task.
    //
    // The second-order damage is worse than the failed seed: `resolve` prefers a plain index over
    // a versioned one, so once the empty `documents` exists every other component starts resolving
    // to it instead of the real index — a corruption that repairs itself only by hand.
    let index = &client.resolve(&config.search.documents_index).await?;
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
    for alias in [
        &config.search.documents_index,
        &config.search.comments_index,
    ] {
        // Reads resolve too, or `stats` reports the alias as unavailable while the versioned index
        // behind it is perfectly healthy.
        let index = &client
            .resolve(alias)
            .await
            .unwrap_or_else(|_| alias.clone());
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

    // Printed unescaped: the point of this command is to show the text as the user sees it.
    // What normalisation removed is reported separately, because those characters are invisible
    // by definition and listing their codepoints is the only way to see them at all.
    println!("input      {input}");
    println!("normalized {normalized}");
    if folded != normalized {
        println!("folded     {folded}");
    }

    let changed = changed_codepoints(input, &normalized);
    if !changed.is_empty() {
        println!("changed");
        for line in &changed {
            println!("           {line}");
        }
    }

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

/// Characters present in the input that did not survive normalisation unchanged.
///
/// Says *why* rather than just listing codepoints, and distinguishes removal from folding —
/// an Arabic-Indic digit is not deleted, it becomes an ASCII one, and reporting that as
/// "removed" would send someone looking for a bug that is not there.
///
/// A multiset difference rather than a set one, so dropping one of two identical marks shows up.
fn changed_codepoints(input: &str, normalized: &str) -> Vec<String> {
    use std::collections::HashMap;

    let mut after: HashMap<char, usize> = HashMap::new();
    for c in normalized.chars() {
        *after.entry(c).or_default() += 1;
    }

    let mut names: Vec<String> = Vec::new();
    let mut seen: Vec<char> = Vec::new();
    for c in input.chars() {
        match after.get_mut(&c) {
            Some(n) if *n > 0 => *n -= 1,
            _ => {
                if !seen.contains(&c) {
                    seen.push(c);
                    names.push(format!("U+{:04X}  {}", c as u32, describe(c)));
                }
            }
        }
    }
    names
}

fn describe(c: char) -> &'static str {
    match c {
        '\u{0660}'..='\u{0669}' => "Arabic-Indic digit, folded to ASCII",
        '\u{06F0}'..='\u{06F9}' => "Extended Arabic-Indic digit, folded to ASCII",
        '\u{0640}' => "tatweel, removed",
        '\u{064B}'..='\u{0652}' => "harakat, removed",
        '\u{0653}'..='\u{065F}' => "hamza/madda mark, removed",
        '\u{0670}' => "superscript alef, removed",
        '\u{200B}'..='\u{200F}' => "zero-width / bidi mark, removed",
        '\u{FEFF}' => "byte order mark, removed",
        c if c.is_whitespace() => "whitespace, collapsed",
        c if c.is_control() => "control character, removed",
        _ => "removed",
    }
}

async fn cmd_search(
    client: &MeiliClient,
    config: &Config,
    query: &str,
    limit: usize,
) -> Result<()> {
    let normalized = xustive_text::normalize(query);
    let q = xustive_search::Query::new(&normalized).limit(limit);
    let index = client.resolve(&config.search.documents_index).await?;
    let hits = client.search::<Value>(&index, &q).await?;

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
