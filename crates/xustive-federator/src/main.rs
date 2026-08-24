//! `xustive-federator` — the Federation Gateway binary ([[ADR-0017]], M7-T04.1).
//!
//! Dual-homed by deployment: it listens on the internal `core` network (where the serving API
//! reaches it) and, on the egress network, holds the only client that can reach the self-hosted
//! SearXNG. It receives a query and nothing else identifying — no user, no IP, no session — and
//! returns a URL+snippet list. If this process dies, federation simply stops and every search keeps
//! working index-only. That is the test for whether the separation is right.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use xustive_federation::SearxngClient;
use xustive_federator::{app, AppState};

#[derive(Parser, Debug)]
#[command(
    name = "xustive-federator",
    about = "The Federation Gateway: serving plane ↔ self-hosted SearXNG"
)]
struct Args {
    /// Address to listen on (the `core`-side interface the serving API calls).
    #[arg(long, env = "FEDERATOR_BIND", default_value = "0.0.0.0:8095")]
    bind: String,

    /// The self-hosted SearXNG JSON endpoint. Empty leaves the gateway inert — it answers empty
    /// rather than failing, so a deployment with federation off still runs the process harmlessly.
    #[arg(long, env = "SEARXNG_URL", default_value = "")]
    searxng_url: String,

    /// Hits to request per query.
    #[arg(long, env = "FEDERATION_MAX_HITS", default_value_t = 10)]
    max_hits: usize,

    /// Upstream timeout for a single SearXNG call, in ms. The per-request budget bounds it further.
    #[arg(long, env = "FEDERATION_TIMEOUT_MS", default_value_t = 2000)]
    timeout_ms: u64,

    /// Default per-request budget in ms, applied when a `/federate` call carries none.
    #[arg(long, env = "FEDERATION_BUDGET_MS", default_value_t = 250)]
    budget_ms: u64,
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

    let client = SearxngClient::new(
        &args.searxng_url,
        args.max_hits,
        Duration::from_millis(args.timeout_ms),
    )
    .map(Arc::new);

    match &client {
        Some(_) => tracing::info!(searxng = %args.searxng_url, "federation gateway starting"),
        None => tracing::warn!("no SEARXNG_URL configured — gateway is inert (answers empty)"),
    }

    let state = AppState {
        client,
        default_budget: Duration::from_millis(args.budget_ms),
    };

    let listener = tokio::net::TcpListener::bind(&args.bind).await?;
    tracing::info!(bind = %args.bind, "listening");
    axum::serve(listener, app(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("shutting down");
        })
        .await?;
    Ok(())
}
