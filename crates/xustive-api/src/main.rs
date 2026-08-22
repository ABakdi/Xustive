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

    // A non-commercial model is fine for local evaluation but must not ship unnoticed. Shout about
    // it at load so it is impossible to run one commercially by accident (see models/LICENSES.md).
    if !status.spec.commercial_use {
        tracing::warn!(
            model = status.spec.id,
            licence = status.spec.licence,
            "summariser model is NOT licensed for commercial use — pin a commercial-safe size via [ml] summariser_model before launch"
        );
    }

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
    // Refuse to start rather than warn. A configuration that crawls abusively produces no symptom
    // in this process at all — the damage is entirely on other people's servers, and by the time
    // anyone notices we are in an abuse report. Failing at startup is the only feedback loop that
    // closes here.
    config.crawl.guard(&config.environment)?;
    config.interaction.guard(&config.environment)?;
    telemetry::init(&config.telemetry);
    // Reverts any /admin/log-level override once its fifteen minutes are up.
    telemetry::spawn_override_expiry();

    tracing::info!(
        config = %args.config.display(),
        bind = %config.api.bind_addr,
        meili = %config.search.meili_url,
        "config loaded"
    );

    let bind = config.api.bind_addr.clone();
    let state = AppState::new(config)?;
    state.resolve_index().await;
    state.refresh_suggestions().await;
    // Connect the anonymous interaction store if enabled (M6). Non-fatal if Redis is down.
    state.connect_interactions().await;
    // Create the image-similarity collection if enabled. Failure here (Qdrant down) is not fatal:
    // the endpoint returns a clean "unavailable" and text search is unaffected ([[Vector Index]] §7).
    if let Some(engine) = &state.image_search {
        match engine.store.ensure_collection().await {
            Ok(()) => tracing::info!("image-similarity collection ready"),
            Err(e) => tracing::warn!(error = %e, "image similarity enabled but Qdrant unreachable"),
        }
    }
    spawn_model_load(&state);
    // Publishes how old the cached tool data is, on a timer rather than on request. A fetcher
    // that stops silently leaves the last values in place and every card keeps rendering, so
    // traffic-driven sampling would report a healthy number for a dead fetcher.
    xustive_api::dataage::spawn(state.clone());
    let router = app(state);

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    let addr = listener.local_addr()?;
    tracing::info!(%addr, "xustive-api listening");

    // This process is the **API only** — JSON, no HTML. The frontend (search *and* admin) is the
    // Next.js app in `web/`, a separate server that proxies `/api/v1/*` here. There is no UI on this
    // port; saying so plainly saves people looking for one.
    let host = if addr.ip().is_unspecified() {
        format!("localhost:{}", addr.port())
    } else {
        addr.to_string()
    };
    eprintln!();
    eprintln!("  Xustive API is running.");
    eprintln!();
    // Written without a literal query string on purpose. The nightly log scan flags any line
    // containing one, and a banner that trips it teaches people to ignore the check.
    eprintln!("    API       http://{host}/api/v1/search   (takes a \"q\" parameter)");
    eprintln!("    Health    http://{host}/readyz");
    eprintln!("    Frontend  the Next.js app in web/  (npm run dev), which proxies to this API");
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

    // Bound the drain (M4-T02.7). Returning from this future lets axum stop accepting connections
    // and wait for in-flight requests to finish — but that wait is unbounded, so a single hung
    // request (a stalled summary, a wedged upstream) would keep the process alive until the
    // orchestrator SIGKILLs it. Arm a grace timer: if the drain has not completed by then, exit
    // cleanly ourselves rather than be killed uncleanly.
    const GRACE: std::time::Duration = std::time::Duration::from_secs(25);
    tokio::spawn(async move {
        tokio::time::sleep(GRACE).await;
        tracing::warn!(
            grace_secs = GRACE.as_secs(),
            "graceful shutdown grace period elapsed with requests still in flight; forcing exit"
        );
        std::process::exit(0);
    });
}
