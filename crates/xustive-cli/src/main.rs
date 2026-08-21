//! `xustive-cli` — operator tooling.
//!
//! `migrate` applies index settings from code, `seed` loads a corpus, `text` explains what the
//! normaliser actually does to a string. That last one exists because "why does this query match
//! nothing" is almost always a normalisation question.

mod commoncrawl;
mod crawl;
mod crawld;
mod discover;
mod eval;
mod pagerank;
mod parsecheck;
mod registry;
mod serp_eval;
mod shutdown;
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
        /// Registry file whose approved, active sources are also seeded. Missing file is fine.
        #[arg(long, default_value = "data/sources/registry.jsonl")]
        registry: PathBuf,
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
    /// Bootstrap the frontier from a Common Crawl snapshot's index (M2-T16.1).
    CommonCrawl {
        /// Snapshot id, e.g. `CC-MAIN-2026-05`.
        #[arg(long)]
        index: String,
        /// CDX URL pattern to scan. `*.dz` is every `.dz` host.
        #[arg(long, default_value = "*.dz")]
        pattern: String,
        /// Registry file, for the known Algerian hosts on generic TLDs.
        #[arg(long, default_value = "data/sources/registry.jsonl")]
        registry: PathBuf,
        /// Stop after this many index pages. Omit to ingest the whole snapshot.
        #[arg(long)]
        max_pages: Option<usize>,
        /// Ignore saved progress and start from page 0.
        #[arg(long)]
        restart: bool,
        /// Seconds between index pages — politeness to the CDX server.
        #[arg(long, default_value_t = 1)]
        page_delay: u64,
    },
    /// Resolve weak-coverage search terms to URLs via Brave, seeding them for crawl (M2-T16.4/.6).
    Discover,
    /// Compute domain authority (PageRank) from the crawl link graph and store it for ranking.
    #[command(name = "pagerank")]
    PageRank,
    /// Fetch a URL and show what the parser extracts — for authoring per-domain rules.
    ParseCheck {
        url: String,
        #[arg(long, default_value = "data/parsers/domains.toml")]
        rules: PathBuf,
        /// Try a candidate `date` selector against the fetched page without writing a rule.
        #[arg(long)]
        date: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        body: Option<String>,
    },
    /// Curate the data sources registry: list, approve, activate, disable, lint.
    Registry {
        /// The JSON-Lines registry file, versioned in git.
        #[arg(long, default_value = "data/sources/registry.jsonl")]
        path: PathBuf,
        #[command(subcommand)]
        action: registry::RegistryAction,
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
    /// Compare our result ordering to a reference search engine (the "compare to Google" yardstick).
    EvalSerp {
        #[arg(long, default_value = "eval/serp-queries.txt")]
        queries: PathBuf,
        #[arg(long, default_value = "eval/reports")]
        out: PathBuf,
        /// Reference engine: `duckduckgo` | `bing` | `google`. Default: google if a SERP proxy is
        /// configured, else duckduckgo (the engine that answers a direct connection).
        #[arg(long)]
        engine: Option<String>,
        /// How many top domains to compare.
        #[arg(long, default_value_t = 10)]
        k: usize,
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
    /// Rebuild the search index into a staging copy and atomically swap it in (M4-T04.8).
    ///
    /// The zero-downtime migration: build `<index>_next` with the current code's settings, copy
    /// every document into it, verify the count, then swap it into place in one atomic operation —
    /// searches never see a half-built index. `--rollback` swaps back to the previous contents
    /// (kept in the staging index). This is the drill that proves the alias machinery works before
    /// a real settings change needs it.
    Reindex {
        /// Operate on this index/alias instead of the configured documents index — used to drill
        /// against a throwaway index without touching the live one.
        #[arg(long)]
        index: Option<String>,
        /// Swap back to the previous contents (undo the last reindex swap).
        #[arg(long)]
        rollback: bool,
        /// Report what would happen without creating, copying, or swapping anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove all already-indexed content for a domain — the composite takedown (M4-T09.3).
    ///
    /// A takedown lives across stores, so one command clears them all: every document with this
    /// domain is deleted from the lexical index, its image vectors from Qdrant, and its raw stored
    /// body from Redis. **Destructive and deliberate** — it previews by default and only deletes
    /// with `--yes`. It does NOT stop *future* crawling; pair it with `registry disable <source>`
    /// (or a takedown-tier exclusion) for that.
    Takedown {
        /// The domain to remove, e.g. `example.dz`. Matched exactly against each document's `domain`.
        #[arg(long)]
        domain: String,
        /// Actually delete. Without this the command only reports what it would remove.
        #[arg(long)]
        yes: bool,
    },
    /// Show index document counts.
    Stats,
    /// Delete image vectors whose parent document is gone from the index (orphan reconciliation).
    ///
    /// A takedown or reindex can remove a document from the lexical index while its image
    /// embeddings linger in Qdrant, leaving a removed image findable by similarity. This walks the
    /// vector collection, checks each document against the index, and deletes the orphans
    /// ([[Vector Index]] §7, [[Security and Privacy]] §8).
    ReconcileVectors {
        /// Report what would be deleted without deleting anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Score transcription/OCR output against references (WER or CER) — the M3 exit-gate metric.
    ///
    /// Reads a JSON-Lines file of `{"reference": "...", "hypothesis": "...", "id": "..."}` and
    /// reports the micro-averaged error rate. Decoupled from the models on purpose: produce the
    /// hypotheses however (the STT/OCR sidecars, or by hand), then score them here. This is what
    /// M3-T02.10 (WER ≤ 25/20/45 %) and M3-T04.8 (CER ≤ 15 %) are measured with.
    ScoreTranscripts {
        #[arg(long)]
        input: PathBuf,
        /// `wer` (word, for voice) or `cer` (character, for OCR).
        #[arg(long, default_value = "wer")]
        metric: String,
        /// Fail (non-zero exit) if the corpus rate exceeds this fraction (e.g. 0.25).
        #[arg(long)]
        threshold: Option<f32>,
        /// Print the worst-scoring lines too, this many of them.
        #[arg(long, default_value_t = 0)]
        worst: usize,
    },
    /// Show what normalisation does to a string.
    Text { input: String },
    /// Run a search from the command line.
    Search {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        /// Show the per-signal score breakdown behind each result's position.
        #[arg(long)]
        explain: bool,
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

    // `score-transcripts` reads a file and computes a metric — no backend involved.
    if let Command::ScoreTranscripts {
        input,
        metric,
        threshold,
        worst,
    } = &args.command
    {
        return cmd_score_transcripts(input, metric, *threshold, *worst);
    }

    // `registry` curates a git-versioned file and needs no backend either.
    if let Command::Registry { path, action } = &args.command {
        return registry::run(path, action);
    }

    // `commoncrawl` reads the CDX index and seeds the frontier; no Meili involved.
    if let Command::CommonCrawl {
        index,
        pattern,
        registry,
        max_pages,
        restart,
        page_delay,
    } = &args.command
    {
        return commoncrawl::run(
            &config,
            &commoncrawl::Options {
                index: index.clone(),
                pattern: pattern.clone(),
                registry_path: registry.display().to_string(),
                max_pages: *max_pages,
                restart: *restart,
                page_delay_secs: *page_delay,
            },
        )
        .await;
    }

    // `discover` reads weak terms and resolves them via Brave; no Meili involved.
    if let Command::Discover = &args.command {
        return discover::run(&config).await;
    }

    // `pagerank` computes domain authority from the link graph in Redis; no Meili involved.
    if let Command::PageRank = &args.command {
        return pagerank::run(&config).await;
    }

    // `parse-check` fetches a URL and runs the parser; no index or queue involved.
    if let Command::ParseCheck {
        url,
        rules,
        date,
        title,
        body,
    } = &args.command
    {
        return parsecheck::run(&parsecheck::CheckOptions {
            url: url.clone(),
            rules_path: rules.display().to_string(),
            date: date.clone(),
            title: title.clone(),
            body: body.clone(),
        })
        .await;
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
            registry,
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
                    registry_path: Some(registry.display().to_string()),
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
        Command::EvalSerp {
            queries,
            out,
            engine,
            k,
            dry_run,
        } => {
            let opts = serp_eval::SerpEvalOptions {
                queries,
                out_dir: out,
                engine,
                k,
                dry_run,
                date: today(),
            };
            serp_eval::run(&client, &config, &opts).await
        }
        Command::Worker { once } => worker::run(&config, &client, once).await,
        Command::Dlq { action, limit } => worker::dlq(&config, &action, limit).await,
        Command::Keys { show } => cmd_keys(&client, show).await,
        Command::Stats => cmd_stats(&client, &config).await,
        Command::Takedown { domain, yes } => cmd_takedown(&client, &config, &domain, yes).await,
        Command::Reindex {
            index,
            rollback,
            dry_run,
        } => cmd_reindex(&client, &config, index.as_deref(), rollback, dry_run).await,
        Command::ReconcileVectors { dry_run } => {
            cmd_reconcile_vectors(&client, &config, dry_run).await
        }
        Command::Search {
            query,
            limit,
            explain,
        } => cmd_search(&client, &config, &query, limit, explain).await,
        Command::Text { .. }
        | Command::Registry { .. }
        | Command::ParseCheck { .. }
        | Command::CommonCrawl { .. }
        | Command::Discover
        | Command::ScoreTranscripts { .. }
        | Command::PageRank => {
            unreachable!("handled above")
        }
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

/// Delete image vectors whose parent document no longer exists in the lexical index.
///
/// The direction matters: it walks what the *vector* store holds and checks each document against
/// the *lexical* index, deleting the vectors of anything the index no longer has. It never adds or
/// changes vectors — reconciliation only removes, so at worst a transient index outage makes it a
/// no-op, never a data-losing one.
async fn cmd_reconcile_vectors(client: &MeiliClient, config: &Config, dry_run: bool) -> Result<()> {
    use serde_json::Value;

    let v = &config.vector;
    let key = (!v.qdrant_key.is_empty()).then(|| v.qdrant_key.clone());
    let store = xustive_vector::Store::new(
        &v.qdrant_url,
        key,
        v.collection.clone(),
        Duration::from_millis(v.timeout_ms.max(30_000)),
    )
    .context("building the vector store client")?;

    let doc_ids = match store.all_document_ids(1_000).await {
        Ok(ids) => ids,
        // A missing collection is not an error to reconcile — there is simply nothing embedded yet.
        Err(xustive_vector::VectorError::Backend { status: 404, .. }) => {
            println!(
                "vector collection '{}' does not exist; nothing to reconcile",
                v.collection
            );
            return Ok(());
        }
        Err(e) => return Err(anyhow::Error::new(e).context("scrolling the vector collection")),
    };
    if doc_ids.is_empty() {
        println!(
            "vector collection '{}' is empty; nothing to reconcile",
            v.collection
        );
        return Ok(());
    }
    println!(
        "{} distinct documents in the vector collection",
        doc_ids.len()
    );

    let index = client
        .resolve(&config.search.documents_index)
        .await
        .unwrap_or_else(|_| config.search.documents_index.clone());

    // Check existence in batches: `id IN [...]` returns the ids the index still has, so the orphans
    // are the batch minus what came back.
    let mut orphans: Vec<String> = Vec::new();
    for batch in doc_ids.chunks(200) {
        let quoted: Vec<String> = batch.iter().map(|id| format!("\"{id}\"")).collect();
        let query = xustive_search::Query::new("")
            .filter(format!("id IN [{}]", quoted.join(", ")))
            .limit(batch.len());
        let hits = client
            .search::<Value>(&index, &query)
            .await
            .context("checking documents against the index")?;
        let live: std::collections::HashSet<String> = hits
            .hits
            .iter()
            .filter_map(|h| h.get("id").and_then(Value::as_str).map(str::to_string))
            .collect();
        for id in batch {
            if !live.contains(id) {
                orphans.push(id.clone());
            }
        }
    }

    if orphans.is_empty() {
        println!("no orphaned vectors — every embedded document still exists in the index");
        return Ok(());
    }
    println!("{} orphaned documents (vectors to delete)", orphans.len());

    if dry_run {
        for id in &orphans {
            println!("  would delete vectors for {id}");
        }
        println!("dry run: nothing deleted");
        return Ok(());
    }

    let mut deleted = 0usize;
    for id in &orphans {
        match store.delete_by_document(id).await {
            Ok(()) => deleted += 1,
            Err(e) => eprintln!("  failed to delete vectors for {id}: {e}"),
        }
    }
    println!(
        "deleted vectors for {deleted}/{} orphaned documents",
        orphans.len()
    );
    Ok(())
}

/// Remove every already-indexed artefact of a domain across all stores.
async fn cmd_takedown(
    client: &MeiliClient,
    config: &Config,
    domain: &str,
    execute: bool,
) -> Result<()> {
    use serde_json::Value;

    let index = client
        .resolve(&config.search.documents_index)
        .await
        .unwrap_or_else(|_| config.search.documents_index.clone());

    // Find every document for this domain. `domain` is a filterable attribute, so this is one
    // filtered query paged to the end — the same shape the reindex copy uses.
    let filter = format!("domain = {}", quote_meili(domain));
    let mut targets: Vec<(String, String)> = Vec::new(); // (id, url)
    let mut offset = 0usize;
    const PAGE: usize = 1000;
    loop {
        let query = xustive_search::Query::new("")
            .filter(filter.clone())
            .offset(offset)
            .limit(PAGE);
        let hits = client
            .search::<Value>(&index, &query)
            .await
            .context("searching for the domain's documents")?;
        if hits.hits.is_empty() {
            break;
        }
        for h in &hits.hits {
            if let Some(id) = h.get("id").and_then(Value::as_str) {
                let url = h
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                targets.push((id.to_string(), url));
            }
        }
        offset += PAGE;
    }

    if targets.is_empty() {
        println!("takedown: no indexed documents for domain '{domain}' — nothing to remove");
        return Ok(());
    }
    println!("takedown: {} documents for '{domain}'", targets.len());

    if !execute {
        for (id, url) in targets.iter().take(20) {
            println!("  would remove {id}  {url}");
        }
        if targets.len() > 20 {
            println!("  … and {} more", targets.len() - 20);
        }
        println!("preview only — re-run with --yes to delete. This does NOT stop future crawling;");
        println!("pair it with `registry disable <source>` to prevent re-indexing.");
        return Ok(());
    }

    // Optional stores: image vectors and raw bodies. Absent is fine — a document simply had none.
    let vectors = if config.vector.enabled {
        let v = &config.vector;
        let key = (!v.qdrant_key.is_empty()).then(|| v.qdrant_key.clone());
        xustive_vector::Store::new(
            &v.qdrant_url,
            key,
            v.collection.clone(),
            Duration::from_millis(v.timeout_ms),
        )
        .ok()
    } else {
        None
    };
    let raw = xustive_ingest::raw_store::RawStore::connect_in(
        &config.queue.url,
        "frontier",
        Duration::from_secs(1),
    );

    let (mut docs, mut vecs, mut bodies) = (0u64, 0u64, 0u64);
    for (id, url) in &targets {
        // Lexical index.
        match client.delete_document(&index, id).await {
            Ok(_) => docs += 1,
            Err(e) => eprintln!("  ⚠ failed to delete document {id}: {e}"),
        }
        // Image vectors keyed by this document id.
        if let Some(store) = &vectors {
            if store.delete_by_document(id).await.is_ok() {
                vecs += 1;
            }
        }
        // Raw stored body, keyed by url.
        if let (Some(rs), false) = (&raw, url.is_empty()) {
            rs.forget(url).await;
            bodies += 1;
        }
    }

    println!("takedown complete for '{domain}':");
    println!("  documents removed from the index : {docs}");
    println!("  image-vector groups deleted      : {vecs}");
    println!("  raw bodies forgotten             : {bodies}");
    println!("Reminder: future crawls are NOT blocked by this. Run `registry disable <source>`");
    println!("(or add the host to the takedown exclusion tier) to prevent re-indexing.");
    Ok(())
}

/// Quote a value for a Meilisearch filter expression.
fn quote_meili(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\\\""))
}

/// The index-migration drill: build a staging copy, verify, atomically swap, and support rollback.
async fn cmd_reindex(
    client: &MeiliClient,
    config: &Config,
    index_override: Option<&str>,
    rollback: bool,
    dry_run: bool,
) -> Result<()> {
    // The live index (resolved through the alias) and its staging sibling.
    let alias = index_override.unwrap_or(&config.search.documents_index);
    let live = client
        .resolve(alias)
        .await
        .unwrap_or_else(|_| alias.to_string());
    let staging = format!("{live}_next");

    if rollback {
        // The previous contents are sitting in the staging index from the last swap; swapping again
        // puts them back. Nothing is rebuilt — rollback is instant.
        println!("rollback: swapping '{live}' ↔ '{staging}' to restore the previous contents");
        if dry_run {
            println!("dry run: no swap performed");
            return Ok(());
        }
        let uid = client
            .swap_indexes(&live, &staging)
            .await
            .context("swap-indexes (rollback)")?;
        client
            .wait_task(uid)
            .await
            .context("waiting for the swap")?;
        println!("✓ rolled back");
        return Ok(());
    }

    let live_count = client
        .stats(&live)
        .await
        .map(|s| s.number_of_documents)
        .unwrap_or(0);
    println!("reindex: '{live}' ({live_count} docs) → build '{staging}' → verify → atomic swap");
    if dry_run {
        println!("dry run: would rebuild {live_count} documents into '{staging}' and swap it in");
        return Ok(());
    }

    // Fresh staging index with the *current code's* settings (the whole point of a reindex is to
    // apply a settings change). Delete any leftover from a prior run first; Meilisearch runs tasks
    // in order, so the create below is applied after the delete.
    let _ = client.delete_index(&staging).await;
    client
        .ensure_index(&staging, "id")
        .await
        .context("creating the staging index")?;
    client
        .apply_settings(&staging, &xustive_search::settings::documents_settings())
        .await
        .context("configuring the staging index")?;

    // Copy every document, page by page, waiting for each write so a slow indexer cannot let the
    // verify race ahead of the copy.
    const PAGE: u64 = 1000;
    let mut offset = 0u64;
    let mut copied = 0u64;
    loop {
        let docs = client
            .documents_page(&live, offset, PAGE)
            .await
            .context("reading a page of documents")?;
        if docs.is_empty() {
            break;
        }
        let uid = client
            .add_documents(&staging, &docs)
            .await
            .context("writing a page into staging")?;
        client.wait_task(uid).await.context("waiting for a write")?;
        copied += docs.len() as u64;
        offset += PAGE;
        if copied % 10_000 == 0 {
            println!("  copied {copied}/{live_count}");
        }
    }
    println!("  copied {copied} documents into '{staging}'");

    // Verify before flipping: a staging index short of the live one is a copy that failed, and
    // swapping it in would lose documents silently.
    let staging_count = client
        .stats(&staging)
        .await
        .map(|s| s.number_of_documents)
        .unwrap_or(0);
    if staging_count < live_count {
        anyhow::bail!(
            "verify failed: staging has {staging_count} documents but live has {live_count} — not swapping"
        );
    }
    println!("  verify ok: staging has {staging_count} documents (live had {live_count})");

    // The atomic flip. After this, '{live}' serves the new content and '{staging}' holds the old
    // one — which is exactly the rollback source.
    let uid = client
        .swap_indexes(&live, &staging)
        .await
        .context("swap-indexes")?;
    client
        .wait_task(uid)
        .await
        .context("waiting for the swap")?;
    println!(
        "✓ swapped '{staging}' into '{live}'. Previous contents kept in '{staging}' for rollback."
    );
    println!("  verify a search, then `reindex --rollback` to undo or delete '{staging}' when satisfied.");
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

#[derive(serde::Deserialize)]
struct TranscriptPair {
    reference: String,
    hypothesis: String,
    #[serde(default)]
    id: Option<String>,
}

/// Score a JSON-Lines file of (reference, hypothesis) pairs with WER or CER.
fn cmd_score_transcripts(
    input: &std::path::Path,
    metric: &str,
    threshold: Option<f32>,
    worst: usize,
) -> Result<()> {
    use xustive_text::metrics::{char_counts, word_counts, Accumulator, Counts};

    let is_cer = match metric.to_ascii_lowercase().as_str() {
        "wer" => false,
        "cer" => true,
        other => anyhow::bail!("unknown metric '{other}' — use 'wer' or 'cer'"),
    };

    let text =
        std::fs::read_to_string(input).with_context(|| format!("reading {}", input.display()))?;

    let mut acc = Accumulator::new();
    let mut scored: Vec<(String, Counts)> = Vec::new();
    let mut skipped = 0usize;
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let pair: TranscriptPair = match serde_json::from_str(line) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("  line {}: skipped ({e})", n + 1);
                skipped += 1;
                continue;
            }
        };
        let counts = if is_cer {
            char_counts(&pair.reference, &pair.hypothesis)
        } else {
            word_counts(&pair.reference, &pair.hypothesis)
        };
        acc.add(counts);
        let label = pair.id.unwrap_or_else(|| format!("line {}", n + 1));
        scored.push((label, counts));
    }

    if acc.pairs() == 0 {
        anyhow::bail!("no scorable pairs in {}", input.display());
    }

    let metric_name = if is_cer { "CER" } else { "WER" };
    let rate = acc.rate();
    println!(
        "{metric_name}: {:.2}%  ({} edits over {} reference {}, {} pairs{})",
        rate * 100.0,
        acc.edits(),
        acc.reference_len(),
        if is_cer { "chars" } else { "words" },
        acc.pairs(),
        if skipped > 0 {
            format!(", {skipped} skipped")
        } else {
            String::new()
        },
    );

    if worst > 0 {
        // Sort by per-pair rate, worst first, so the offenders are easy to inspect.
        scored.sort_by(|a, b| {
            b.1.rate()
                .partial_cmp(&a.1.rate())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        println!("worst {}:", worst.min(scored.len()));
        for (label, c) in scored.iter().take(worst) {
            println!(
                "  {:.1}%  {label}  ({}/{})",
                c.rate() * 100.0,
                c.edits,
                c.reference_len
            );
        }
    }

    if let Some(limit) = threshold {
        if rate > limit {
            anyhow::bail!(
                "{metric_name} {:.2}% exceeds the {:.2}% threshold",
                rate * 100.0,
                limit * 100.0
            );
        }
        println!("within the {:.2}% threshold", limit * 100.0);
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
    explain: bool,
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

    // Ranked through the same `rerank` the API uses, not Meilisearch's own order.
    //
    // The point of `--explain` is to answer "why is this result here", and that question is only
    // meaningful about the order a user actually sees. Explaining the raw engine order would
    // describe a ranking nothing serves.
    let ranked = if explain {
        Some(xustive_search::rank::rerank(
            &hits.hits,
            &normalized,
            xustive_core::now_unix(),
            &trust_tiers(),
            &xustive_search::authority::load(),
            &xustive_search::rank::Weights::default(),
        ))
    } else {
        None
    };

    if let Some(ranked) = ranked {
        for (i, r) in ranked.iter().enumerate() {
            let title = r
                .hit
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("(no title)");
            let url = r.hit.get("url").and_then(Value::as_str).unwrap_or("");
            let src = r
                .hit
                .get("source_type")
                .and_then(Value::as_str)
                .unwrap_or("?");
            let e = &r.explain;
            println!("{:>3}. [{src}] {title}", i + 1);
            println!("     {url}");
            println!(
                "     score {:.4}  = relevance {:.3}  freshness {:.3}  trust {:.3}  \
                 authority {:.3}  quality {:.3}  spam {:.3}",
                e.total, e.relevance, e.freshness, e.trust, e.authority, e.quality, e.spam
            );
            // Age is only meaningful when the date is trusted. Printing `0 days` for a document
            // whose date we guessed reads as "published today", which is how a freshness bug hides.
            let age = if e.date_trusted {
                format!("{:.1} days", e.age_days)
            } else {
                format!("{:.1} days (date not trusted)", e.age_days)
            };
            println!(
                "     age {age}, {} near-duplicate(s) folded in",
                e.collapsed
            );
        }
        return Ok(());
    }

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

/// Trust tiers from the seed list.
///
/// Read the same way the API reads them, and from the same file, or `--explain` would describe a
/// ranking the server does not perform — which is worse than not explaining it at all.
fn trust_tiers() -> std::collections::HashMap<String, xustive_core::TrustTier> {
    use xustive_core::TrustTier;
    const SEEDS: &str = include_str!("../../../data/sources/seeds.tsv");
    let mut out = std::collections::HashMap::new();
    for line in SEEDS.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').map(str::trim).collect();
        if cols.len() < 3 {
            continue;
        }
        let tier = match cols[2].to_ascii_uppercase().as_str() {
            "A" => TrustTier::A,
            "C" => TrustTier::C,
            _ => TrustTier::B,
        };
        out.insert(cols[0].to_string(), tier);
    }
    out
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
