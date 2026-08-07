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

/// Load the summariser in the background.
///
/// Reading two gigabytes of weights takes seconds, and search does not depend on it. Doing this
/// before binding the port would mean the process looked dead while it worked; doing it lazily on
/// the first request would put those seconds in one unlucky user's latency. So it happens
/// alongside, and until it finishes `/v1/summary` answers "no summary" — a state it already
/// handles, because a busy model looks the same from outside.
#[cfg(feature = "summariser")]
fn spawn_model_load(state: &AppState) {
    use std::sync::Arc;
    use xustive_ml::engine::Engine;
    use xustive_ml::registry::{Registry, Role};

    if !state.config.ml.summaries_enabled {
        tracing::info!("summaries are disabled; no model will be loaded");
        return;
    }

    let registry = Registry::new(&state.config.ml.model_dir);
    let preferred = (!state.config.ml.summariser_model.is_empty())
        .then(|| state.config.ml.summariser_model.clone());
    let Some(status) = registry.resolve(Role::Summariser, preferred.as_deref()) else {
        tracing::warn!(
            dir = %state.config.ml.model_dir,
            "no summariser model found; summaries will be unavailable"
        );
        return;
    };

    let device = state.device_config();
    let slots = state.config.ml.slots;
    let engine_slot = Arc::clone(&state.engine);

    // A blocking thread, not a task: loading pins a core for seconds and would otherwise stall
    // the runtime's worker threads, which are also serving search.
    tokio::task::spawn_blocking(move || match Engine::load(&status.path, &device, slots) {
        Ok(engine) => {
            tracing::info!(
                model = %status.path,
                device = engine.resolved().active.as_str(),
                "summariser ready"
            );
            if let Ok(mut slot) = engine_slot.write() {
                *slot = Some(Arc::new(engine));
            }
        }
        // Not fatal. A search engine that will not start because a summariser model is missing
        // has traded its whole purpose for one feature.
        Err(e) => tracing::error!(error = %e, "summariser failed to load; summaries unavailable"),
    });
}

#[cfg(not(feature = "summariser"))]
fn spawn_model_load(_state: &AppState) {
    tracing::info!("built without the summariser feature; summaries unavailable");
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
    state.resolve_index().await;
    spawn_model_load(&state);
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

    axum::serve(
        listener,
        // Connection info is needed by the admin guard, which restricts the operator surface to
        // loopback callers when no admin key is configured.
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
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
