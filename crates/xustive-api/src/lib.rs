//! The Xustive HTTP surface.
//!
//! Handlers are thin: validate, call a service, serialise. Transport concerns (limits, timeouts,
//! headers, error shaping) live in the middleware stack so no downstream component ever deals
//! with HTTP.

pub mod error;
pub mod metrics;
pub mod search;
pub mod state;
pub mod telemetry;
pub mod web;

use std::time::{Duration, Instant};

use axum::extract::{MatchedPath, Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::timeout::TimeoutLayer;

use crate::state::AppState;

/// Build the router with the full middleware stack.
///
/// Layer order is load-bearing. `CatchPanicLayer` is outermost so a panic anywhere inside becomes
/// a 500 rather than killing the worker; compression is innermost so it wraps the final body.
pub fn app(state: AppState) -> Router {
    let api = Router::new()
        .route("/search", get(search::handler))
        .with_state(state.clone());

    let ops = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics_handler))
        // Server-rendered results, so core search works without JavaScript.
        .route("/search", get(web::search_page))
        .with_state(state.clone());

    let static_dir = &state.config.api.static_dir;
    let serve_static =
        ServeDir::new(static_dir).fallback(ServeFile::new(format!("{static_dir}/index.html")));

    Router::new()
        .nest("/api/v1", api)
        .merge(ops)
        .fallback_service(serve_static)
        .layer(middleware::from_fn_with_state(state.clone(), observe))
        .layer(middleware::from_fn(security_headers))
        .layer(CompressionLayer::new())
        // Contract says a blown budget is 504, not the layer's default 408.
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            Duration::from_millis(state.config.api.timeout_search_ms),
        ))
        .layer(RequestBodyLimitLayer::new(
            state.config.api.body_limit_default,
        ))
        .layer(cors_layer(&state))
        .layer(CatchPanicLayer::new())
}

fn cors_layer(state: &AppState) -> CorsLayer {
    let origins = &state.config.api.cors_origins;
    if origins.is_empty() {
        // Same-origin only: the UI is served from this process.
        CorsLayer::new()
    } else {
        let parsed: Vec<HeaderValue> = origins.iter().filter_map(|o| o.parse().ok()).collect();
        CorsLayer::new()
            .allow_origin(parsed)
            .allow_methods([http::Method::GET])
    }
}

/// Request metrics and timing.
///
/// Records the **matched route pattern**, never the concrete path — a path could contain user
/// content and would blow up label cardinality.
async fn observe(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "other".to_string());

    let started = Instant::now();
    let response = next.run(req).await;
    let elapsed = started.elapsed().as_secs_f64();
    let status = response.status().as_u16().to_string();

    state.metrics.incr(
        metrics::HTTP_REQUESTS,
        metrics::HTTP_REQUESTS_HELP,
        &[("route", &route), ("status", &status)],
    );
    state.metrics.observe(
        metrics::HTTP_DURATION,
        metrics::HTTP_DURATION_HELP,
        &[("route", &route)],
        elapsed,
    );
    response
}

/// Security headers on every response.
///
/// `default-src 'self'` is what makes "no third-party scripts" enforced rather than intended:
/// an analytics snippet added later simply will not execute.
async fn security_headers(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    let h = response.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; img-src 'self' data: https:; media-src 'self' blob:; \
             script-src 'self'; style-src 'self'; connect-src 'self'; \
             frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("microphone=(self), camera=(self), geolocation=()"),
    );
    h.insert(
        header::HeaderName::from_static("cross-origin-opener-policy"),
        HeaderValue::from_static("same-origin"),
    );
    response
}

/// Liveness: the process is running. Deliberately checks nothing else — a dependency outage
/// should not cause a restart loop.
async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

/// Readiness: this replica can serve search. The load balancer removes it when this fails.
async fn readyz(State(state): State<AppState>) -> Response {
    match state.search.health().await {
        Ok(true) => (StatusCode::OK, "ready").into_response(),
        Ok(false) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "search backend unavailable",
        )
            .into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "readiness check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "search backend unreachable",
            )
                .into_response()
        }
    }
}

async fn metrics_handler(State(state): State<AppState>) -> Response {
    state
        .metrics
        .set_gauge(metrics::BUILD_INFO, metrics::BUILD_INFO_HELP, 1);
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        state.metrics.render(),
    )
        .into_response()
}
