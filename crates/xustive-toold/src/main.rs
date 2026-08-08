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
use xustive_toold::{weather, Dataset};

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

    loop {
        let stats = fetch_weather(&client, &store).await;
        tracing::info!(
            written = stats.written,
            rejected = stats.rejected,
            failed = stats.failed,
            "weather pass complete"
        );

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
