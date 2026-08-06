//! `xustive-api` — the HTTP surface.

use std::path::PathBuf;

use clap::Parser;
use xustive_api::{app, state::AppState, telemetry};
use xustive_core::Config;

#[derive(Parser, Debug)]
#[command(name = "xustive-api", about = "Xustive HTTP API")]
struct Args {
    /// Path to the config file. Missing file means defaults plus environment overrides.
    #[arg(long, env = "XUSTIVE_CONFIG", default_value = "config/dev.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let config = Config::load(Some(&args.config))?;
    telemetry::init(&config.telemetry);

    tracing::info!(
        config = %args.config.display(),
        bind = %config.api.bind_addr,
        meili = %config.search.meili_url,
        "config loaded"
    );

    let bind = config.api.bind_addr.clone();
    let state = AppState::new(config)?;
    let router = app(state);

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    let addr = listener.local_addr()?;
    tracing::info!(%addr, "xustive-api listening");

    // The UI is served by this process from `static_dir`; there is no separate web server.
    // Saying so here saves people looking for one that does not exist.
    let host = if addr.ip().is_unspecified() {
        format!("localhost:{}", addr.port())
    } else {
        addr.to_string()
    };
    eprintln!();
    eprintln!("  Xustive is running.");
    eprintln!();
    eprintln!("    Web UI    http://{host}");
    eprintln!("    API       http://{host}/api/v1/search?q=...");
    eprintln!("    Health    http://{host}/readyz");
    eprintln!();
    eprintln!("  Ctrl-C to stop.");
    eprintln!();

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("shutdown complete");
    Ok(())
}

/// Drain on SIGTERM (containers) and Ctrl-C (development).
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("received SIGINT, draining"),
        _ = terminate => tracing::info!("received SIGTERM, draining"),
    }
}
