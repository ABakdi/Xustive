//! The Xustive HTTP surface.
//!
//! Handlers are thin: validate, call a service, serialise. Transport concerns (limits, timeouts,
//! headers, error shaping) live in the middleware stack so no downstream component ever deals
//! with HTTP.

pub mod admin;
pub mod dataage;
pub mod deadline;
pub mod error;
pub mod metrics;
pub mod ratelimit;
pub mod search;
pub mod state;
pub mod suggest;
pub mod summary;
pub mod telemetry;
pub mod tools;
pub mod translate;
pub mod weather;

use std::time::{Duration, Instant};

use axum::error_handling::HandleErrorLayer;
use axum::extract::{MatchedPath, Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use tower::ServiceBuilder;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::request_id::{
    MakeRequestId, PropagateRequestIdLayer, RequestId, SetRequestIdLayer,
};
use tower_http::timeout::TimeoutLayer;

use crate::state::AppState;

/// Build the router with the full middleware stack.
///
/// **JSON and operations only.** HTML comes from the Next.js frontend
/// ([[ADR-0010 - Next.js for the Frontend]]); the hand-written renderer that used to live here is
/// deleted rather than kept alongside, because two renderers drifting apart is what made the
/// language filter ship broken and is the whole reason for the rewrite.
///
/// `/healthz`, `/readyz` and `/metrics` stay on this port. A liveness probe must not be answered
/// by a process that merely depends on this one.
///
/// Layer order is load-bearing. `CatchPanicLayer` is outermost so a panic anywhere inside becomes
/// a 500 rather than killing the worker; compression is innermost so it wraps the final body.
pub fn app(state: AppState) -> Router {
    // Timeouts are applied per group rather than once around the whole app. A single outer layer
    // is simpler, but it silently caps every route at the search budget — which turned every
    // summary into a 504 the first time this was wired up, since generation legitimately takes
    // tens of seconds on CPU.
    let search_budget = TimeoutLayer::with_status_code(
        StatusCode::GATEWAY_TIMEOUT,
        Duration::from_millis(state.config.api.timeout_search_ms),
    );

    let api = Router::new()
        .route("/search", get(search::handler))
        .layer(middleware::from_fn_with_state(state.clone(), limit_search))
        .layer(search_budget)
        .with_state(state.clone());

    // Translation streams, so it gets **no response timeout at all**. A timeout layer bounds the
    // whole response including the body, and a streamed body is not finished until the model is —
    // wrapping this the way `/summary` is wrapped would cut every translation off mid-sentence at
    // a fixed number of seconds. The engine's own deadline is what bounds the work here, and it is
    // the right place for it: it stops generating rather than severing a connection.
    let translate = Router::new()
        .route("/translate", axum::routing::post(translate::handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            limit_translate,
        ))
        .with_state(state.clone());

    // Suggestions fire per keystroke, so they get a much tighter budget and a much higher rate
    // limit. Sharing the search budget would let a slow index hold a suggestion box open for a
    // second and a half, which a user reads as a broken input rather than a slow one.
    let suggest_routes = Router::new()
        .route("/suggest", get(suggest::handler))
        .route("/tools", get(tools::handler))
        .route("/languages", get(translate::languages))
        .layer(middleware::from_fn_with_state(state.clone(), limit_suggest))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            Duration::from_millis(state.config.api.timeout_suggest_ms),
        ))
        .with_state(state.clone());

    // Summaries get their own, far longer timeout. Generation on CPU runs to tens of seconds,
    // and the global search budget applied to this endpoint turns every summary into a 504 —
    // which is also what happened the first time it was wired in. The engine enforces its own
    // deadline internally, so this only has to be looser than that one.
    let summary = Router::new()
        .route("/summary", axum::routing::post(summary::handler))
        // Tighter than search: each one costs tens of seconds of CPU, so the limit that matters
        // is not requests per minute but how many can be in flight from one place at all.
        .layer(middleware::from_fn_with_state(state.clone(), limit_summary))
        // Looser than the engine's own deadline, which is the one that actually bounds the work.
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            Duration::from_millis(state.config.ml.deadline_ms + 10_000),
        ))
        .with_state(state.clone());

    let ops = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics_handler))
        // Operator surface. Read-mostly, and nothing here can stop the process starting.
        .route("/admin", get(admin::page))
        .route(
            "/admin/politeness",
            axum::routing::post(admin::set_politeness),
        )
        .route("/admin.css", get(admin::admin_css))
        .route("/admin.js", get(admin::admin_js))
        .route("/admin/status", get(admin::status))
        .route("/admin/device", axum::routing::post(admin::set_device))
        .route(
            "/admin/log-level",
            axum::routing::post(admin::set_log_level),
        )
        .layer(search_budget)
        .with_state(state.clone());

    Router::new()
        .nest(
            "/api/v1",
            api.merge(summary).merge(translate).merge(suggest_routes),
        )
        .merge(ops)
        // Request ids, outermost after panic catching so even a shed or rejected request
        // carries one. A ULID rather than a UUID: it sorts by time, so grepping a log for ids
        // near an incident actually narrows the window.
        //
        // Applied around `observe` so the id exists before anything is recorded, and propagated
        // to the response so a user reporting a problem can quote something we can find.
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(middleware::from_fn_with_state(state.clone(), observe))
        .layer(middleware::from_fn(security_headers))
        .layer(CompressionLayer::new())
        .layer(RequestBodyLimitLayer::new(
            state.config.api.body_limit_default,
        ))
        .layer(cors_layer(&state))
        .layer(SetRequestIdLayer::x_request_id(UlidRequestId))
        // Outside `SetRequestIdLayer`, so it runs first and that layer always sees an absent
        // header. tower-http keeps a client-supplied id by default, which is right behind a
        // trusted proxy and wrong on a public endpoint: it lets a caller choose a value we echo
        // back in a response header and group log lines under an id of its choosing.
        .layer(middleware::from_fn(strip_client_request_id))
        // Global in-flight cap. Sheds with 503 rather than queueing: under real overload a
        // queue converts a fast failure into a slow one and every waiting client times out
        // anyway, having occupied a connection for the whole wait.
        //
        // The three layers are one unit and the order inside matters. `ConcurrencyLimit` is
        // innermost and does the counting; `LoadShed` turns "would have to wait" into an error
        // instead of waiting; `HandleError` renders that error, because the router has no other
        // way to. Drop `LoadShed` and requests hang instead of failing.
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(|_: axum::BoxError| async {
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        [(header::RETRY_AFTER, "1")],
                        axum::Json(serde_json::json!({"error": {
                            "code": "overloaded",
                            "message": "too many requests in flight; try again shortly",
                        }})),
                    )
                }))
                .layer(tower::load_shed::LoadShedLayer::new())
                .layer(tower::limit::ConcurrencyLimitLayer::new(
                    state.config.api.max_concurrent,
                )),
        )
        .layer(CatchPanicLayer::new())
}

/// Rate limit `/search`.
///
/// A separate function per route rather than one parameterised layer: the limits differ, the
/// route label has to be a `&'static str` for the bucket key, and a wrong label silently merges
/// two routes' budgets.
async fn limit_search(State(state): State<AppState>, req: Request, next: Next) -> Response {
    enforce(&state, "/search", ratelimit::SEARCH, req, next).await
}

async fn limit_summary(State(state): State<AppState>, req: Request, next: Next) -> Response {
    enforce(&state, "/summary", ratelimit::SUMMARY, req, next).await
}

async fn limit_translate(State(state): State<AppState>, req: Request, next: Next) -> Response {
    enforce(&state, "/translate", ratelimit::TRANSLATE, req, next).await
}

async fn limit_suggest(State(state): State<AppState>, req: Request, next: Next) -> Response {
    enforce(&state, "/suggest", ratelimit::SUGGEST, req, next).await
}

async fn enforce(
    state: &AppState,
    route: &'static str,
    limit: ratelimit::Limit,
    req: Request,
    next: Next,
) -> Response {
    let peer = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|c| c.0);
    let decision = state
        .limiter
        .check(ratelimit::client_ip(peer), route, limit);

    if !decision.allowed {
        state.metrics.incr(
            metrics::RATE_LIMITED,
            metrics::RATE_LIMITED_HELP,
            &[("route", route)],
        );
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, decision.retry_after.to_string())],
            axum::Json(serde_json::json!({"error": {
                "code": "rate_limited",
                "message": "too many requests",
                "retry_after": decision.retry_after,
            }})),
        )
            .into_response();
    }

    let mut response = next.run(req).await;
    // Advertising the remaining budget lets a well-behaved client back off before it is refused,
    // which is cheaper for both sides than a 429.
    if let Ok(v) = decision.remaining.to_string().parse() {
        response.headers_mut().insert("x-ratelimit-remaining", v);
    }
    response
}

/// Discard any inbound `X-Request-Id`.
///
/// If a trusted reverse proxy is added later, this is the one place to teach about it — but the
/// default has to be distrust, because the only caller today is the open internet.
async fn strip_client_request_id(mut req: Request, next: Next) -> Response {
    req.headers_mut().remove("x-request-id");
    next.run(req).await
}

/// Generates the `X-Request-Id` for each request.
///
/// Reuses the same ULID generator as document ids so the format is uniform across the system —
/// an operator correlating a log line with a document should not have to know which kind of id
/// they are looking at.
#[derive(Clone, Copy)]
struct UlidRequestId;

impl MakeRequestId for UlidRequestId {
    fn make_request_id<B>(&mut self, _: &Request<B>) -> Option<RequestId> {
        // `SetRequestIdLayer` only calls this when the header is absent, which
        // `strip_client_request_id` guarantees it always is.
        HeaderValue::from_str(&xustive_core::new_id())
            .ok()
            .map(RequestId::new)
    }
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
