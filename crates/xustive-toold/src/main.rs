//! `xustive-toold` — scheduled fetch of external tool data.
//!
//! Runs on the ingest network, which has egress. It writes to Redis, which the serving plane can
//! read. It has no other route to `core` and receives no user input at all — its inputs are a
//! schedule and a fixed list of URLs.
//!
//! Nothing here can degrade search. That is the test for whether the separation is right: if this
//! process dies, cards age out and disappear over hours, and every result page keeps working.

use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use xustive_toold::store::Store;
use xustive_toold::{knowledge, rates, weather, Dataset};

#[derive(Parser, Debug)]
#[command(
    name = "xustive-toold",
    about = "Fetch external tool data on a schedule"
)]
struct Args {
    #[arg(long, env = "REDIS_URL", default_value = "redis://127.0.0.1:6390")]
    redis: String,

    /// Fetch every dataset once and exit.
    #[arg(long)]
    once: bool,

    /// Seconds between passes. Each dataset still respects its own cadence.
    #[arg(long, default_value_t = 300)]
    tick: u64,

    /// Meilisearch, for the knowledge index. Empty leaves the knowledge harvest switched off, and
    /// the weather pass runs exactly as before — this process must stay useful without it.
    #[arg(long, env = "MEILI_URL", default_value = "")]
    meili: String,

    #[arg(long, env = "MEILI_KEY", default_value = "")]
    meili_key: String,

    /// The seed list. Absent or empty means nothing to harvest, which is not an error.
    #[arg(long, default_value = "data/knowledge/seeds.tsv")]
    seeds: String,

    /// Harvest an entity at most this often. Facts about a person or a film change on the scale of
    /// weeks; re-fetching them hourly would be traffic spent on nothing.
    #[arg(long, default_value_t = 7 * 24 * 3600)]
    knowledge_max_age: i64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let store = Store::connect(&args.redis)?;

    if !store.ping().await {
        // Fatal here, unlike everywhere else in this codebase. Without the cache there is nowhere
        // to put anything, so a process that started would burn a publisher's bandwidth fetching
        // data it must immediately discard.
        anyhow::bail!("no Redis at {}", args.redis);
    }

    let client = xustive_toold::client()?;
    tracing::info!(redis = %args.redis, once = args.once, "tool fetcher starting");

    let meili = knowledge_client(&args);
    if meili.is_none() && !args.meili.is_empty() {
        tracing::warn!(meili = %args.meili, "could not build a Meilisearch client; knowledge harvest is off");
    }

    loop {
        let stats = fetch_weather(&client, &store).await;
        tracing::info!(
            written = stats.written,
            rejected = stats.rejected,
            failed = stats.failed,
            "weather pass complete"
        );

        match fetch_rates(&client, &store).await {
            Ok(()) => tracing::info!("rates pass complete"),
            // Never fatal, and never clears the previous table: slightly old and correct beats
            // fresh and wrong, which is the whole posture of this process.
            Err(e) => tracing::warn!(error = %e, "rates pass failed; keeping previous table"),
        }

        if let Some(meili) = &meili {
            match harvest_knowledge(&client, meili, &args).await {
                Ok(stats) => tracing::info!(
                    written = stats.written,
                    rejected = stats.rejected,
                    failed = stats.failed,
                    "knowledge pass complete"
                ),
                // Never fatal. The knowledge index going unfetched costs panels; it must not cost
                // the weather cards this process also feeds.
                Err(e) => tracing::warn!(error = %e, "knowledge pass failed"),
            }
        }

        if args.once {
            return Ok(());
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(args.tick)) => {}
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutting down");
                return Ok(());
            }
        }
    }
}

#[derive(Default, Debug)]
struct Stats {
    written: usize,
    rejected: usize,
    failed: usize,
}

async fn fetch_weather(client: &reqwest::Client, store: &Store) -> Stats {
    let dataset = weather::Weather;
    let mut stats = Stats::default();
    let now = xustive_core::now_unix();
    // Well past the staleness limit. The expiry is a backstop; "too old to show" is decided by
    // the serving plane, and letting Redis delete at exactly the limit would make a stale entry
    // indistinguishable from one never fetched.
    let ttl = dataset.staleness_limit().as_secs() * 4;

    for wilaya in weather::targets() {
        let key = weather::key(dataset.key_prefix(), wilaya.code);
        let previous = store
            .get::<weather::Forecast>(&key)
            .await
            .ok()
            .flatten()
            .map(|c| c.payload);

        match weather::fetch(client, wilaya, now, previous.as_ref()).await {
            Ok(fresh) => match store.put(&key, &fresh, ttl).await {
                Ok(()) => stats.written += 1,
                Err(e) => {
                    tracing::warn!(wilaya = wilaya.code, error = %e, "could not write");
                    stats.failed += 1;
                }
            },
            Err(xustive_toold::FetchError::Rejected(reason)) => {
                // The previous value is kept, not cleared. Slightly old and correct beats fresh
                // and wrong, and clearing would turn one bad publisher response into a missing
                // card for the next three hours.
                tracing::warn!(
                    wilaya = wilaya.code,
                    reason,
                    "rejected; keeping previous value"
                );
                stats.rejected += 1;
            }
            Err(e) => {
                tracing::warn!(wilaya = wilaya.code, error = %e, "fetch failed");
                stats.failed += 1;
            }
        }

        // Paced. 58 requests in a burst is rude to a free publisher that asks for nothing in
        // return, and there is no deadline here worth being rude for.
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    stats
}

/// A Meilisearch client for the knowledge index, or `None` when none is configured.
fn knowledge_client(args: &Args) -> Option<xustive_search::MeiliClient> {
    if args.meili.trim().is_empty() {
        return None;
    }
    xustive_search::MeiliClient::new(&args.meili, &args.meili_key, Duration::from_secs(30)).ok()
}

/// One knowledge harvest pass (M8-T01.2).
///
/// Reads the seed list, fetches what is due, resolves referenced labels in one batch, attaches
/// extracts, and writes the renderable entities. Everything about the pacing here is deliberate
/// politeness towards publishers who charge us nothing: sequential requests, a fixed pace, and a
/// re-harvest age measured in days.
async fn harvest_knowledge(
    client: &reqwest::Client,
    meili: &xustive_search::MeiliClient,
    args: &Args,
) -> anyhow::Result<Stats> {
    let mut stats = Stats::default();
    let raw = std::fs::read_to_string(&args.seeds).unwrap_or_default();
    let seeds = knowledge::parse_seeds(&raw);
    if seeds.is_empty() {
        return Ok(stats);
    }

    meili
        .ensure_index(
            xustive_search::settings::KNOWLEDGE,
            xustive_knowledge::index::F_ID,
        )
        .await?;
    meili
        .apply_settings(
            xustive_search::settings::KNOWLEDGE,
            &xustive_search::settings::knowledge_settings(),
        )
        .await?;

    let now = xustive_core::now_unix();
    // Only what is due. Facts about a person or a film change on the scale of weeks, so
    // re-fetching everything every pass would be traffic spent on nothing and rudeness towards a
    // publisher who charges us nothing.
    let fresh = recently_harvested(meili, now, args.knowledge_max_age).await;
    let ids: Vec<String> = seeds
        .iter()
        .map(|s| s.id.clone())
        .filter(|id| !fresh.contains(id))
        .collect();
    if ids.is_empty() {
        tracing::debug!(seeds = seeds.len(), "every seed is fresh; nothing due");
        return Ok(stats);
    }
    let mut harvested = knowledge::fetch_entities(client, &ids, now).await?;

    // Drop anything that is not what the seed said it was, before it can reach the index.
    harvested.retain(|h| {
        let ok = seeds
            .iter()
            .find(|s| s.id == h.entity.id)
            .map(|s| knowledge::matches_expectation(s, &h.entity))
            .unwrap_or(true);
        if !ok {
            tracing::warn!(
                id = %h.entity.id,
                "harvested entity is not the one the seed expected; skipping"
            );
        }
        ok
    });
    stats.rejected += ids.len().saturating_sub(harvested.len());

    let references = knowledge::unresolved_references(&harvested);
    let labels = knowledge::resolve_labels(client, &references)
        .await
        .unwrap_or_default();
    for h in &mut harvested {
        xustive_knowledge::wikidata::fill_entity_labels(&mut h.entity, &labels);
        if let Err(e) = knowledge::attach_extracts(client, h).await {
            // An entity with facts and no paragraph is still a panel worth having.
            tracing::debug!(id = %h.entity.id, error = %e, "no extract");
        }
    }

    let documents = knowledge::documents(&harvested);
    if documents.is_empty() {
        return Ok(stats);
    }
    let task = meili
        .add_documents(xustive_search::settings::KNOWLEDGE, &documents)
        .await?;
    match meili.wait_task(task).await {
        Ok(status) if status.is_success() => stats.written += documents.len(),
        Ok(status) => {
            tracing::warn!(error = %status.error_message(), "index refused the batch");
            stats.failed += documents.len();
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not confirm the batch");
            stats.failed += documents.len();
        }
    }
    Ok(stats)
}

/// The ids harvested recently enough not to be due again.
///
/// Scanned from the index rather than tracked separately: the index is the record of what we hold,
/// and a second store of "when did we last fetch this" would be one more thing to drift. A scan
/// that fails returns nothing, which re-harvests everything — wasteful but correct, and the
/// failure it covers is Meilisearch being unreachable, in which case the write would fail anyway.
async fn recently_harvested(
    meili: &xustive_search::MeiliClient,
    now: i64,
    max_age: i64,
) -> std::collections::HashSet<String> {
    use xustive_knowledge::index::{F_ID, F_UPDATED_AT};
    let mut fresh = std::collections::HashSet::new();
    let mut offset = 0u64;
    loop {
        let page = match meili
            .documents_page_fields(
                xustive_search::settings::KNOWLEDGE,
                offset,
                1000,
                &[F_ID, F_UPDATED_AT],
            )
            .await
        {
            Ok(p) => p,
            Err(_) => return fresh,
        };
        if page.is_empty() {
            return fresh;
        }
        for doc in &page {
            let (Some(id), Some(at)) = (
                doc.get(F_ID).and_then(|v| v.as_str()),
                doc.get(F_UPDATED_AT).and_then(|v| v.as_i64()),
            ) else {
                continue;
            };
            if now.saturating_sub(at) < max_age {
                fresh.insert(id.to_string());
            }
        }
        offset += page.len() as u64;
    }
}

/// One exchange-rate pass (M8-T06.1).
///
/// One request for the whole table, unlike weather's 58: the publisher quotes every currency in a
/// single document, so there is nothing to pace.
async fn fetch_rates(client: &reqwest::Client, store: &Store) -> anyhow::Result<()> {
    let dataset = rates::Rates;
    let now = xustive_core::now_unix();
    let ttl = dataset.staleness_limit().as_secs() * 4;
    let key = rates::key(dataset.key_prefix());

    let previous = store
        .get::<rates::RateTable>(&key)
        .await
        .ok()
        .flatten()
        .map(|c| c.payload);

    let fresh = rates::fetch(client, now, previous.as_ref()).await?;
    store
        .put(&key, &fresh, ttl)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}
