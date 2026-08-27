//! The Xustive HTTP surface.
//!
//! Handlers are thin: validate, call a service, serialise. Transport concerns (limits, timeouts,
//! headers, error shaping) live in the middleware stack so no downstream component ever deals
//! with HTTP.

pub mod admin;
pub mod admin_crawler;
pub mod admin_eval;
pub mod admin_maintenance;
pub mod admin_queue;
pub mod currency;
pub mod dataage;
pub mod deadline;
pub mod error;
pub mod federate;
pub mod geoip;
pub mod image_search;
pub mod interaction;
pub mod knowledge;
pub mod knowledge_model;
pub mod metrics;
pub mod ocr;
pub mod ratelimit;
pub mod search;
pub mod state;
pub mod stt;
pub mod suggest;
pub mod summary;
pub mod telemetry;
pub mod text_search;
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

/// Time the transport timeout allows past the search deadline for the ladder to answer (BUG-041).
const SEARCH_GRACE_MS: u64 = 1_000;

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
    // The transport cut sits *above* the search's own deadline, not on it (BUG-041). The deadline
    // ladder exists to degrade a slow search — drop the summary, the expansion, the facets, the
    // rerank, and still answer — but it can only do that if it fires before this layer does. With
    // the two equal, any slack consumed anywhere let the layer cut the request first and answer a
    // bare 504 with no body, which the web tier could only render as "Search failed". The grace
    // is the time the ladder needs to shape and send a degraded page after its last stage; the
    // layer is now the backstop for a search that has genuinely hung, not the common case.
    let search_budget = TimeoutLayer::with_status_code(
        StatusCode::GATEWAY_TIMEOUT,
        Duration::from_millis(state.config.api.timeout_search_ms + SEARCH_GRACE_MS),
    );

    let api = Router::new()
        .route("/search", get(search::handler))
        .layer(middleware::from_fn_with_state(state.clone(), limit_search))
        .layer(search_budget)
        .with_state(state.clone());

    // The web group of a reverse image search (M10-T03.4): words in, through the gateway. It
    // waits on the metasearch, so its budget is the federation *fetch* budget, not the search
    // one — the page asks for it after the local groups are already on screen.
    let image_web = Router::new()
        .route("/search/image/web", get(image_search::web_handler))
        .layer(middleware::from_fn_with_state(state.clone(), limit_search))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            Duration::from_millis(state.config.federation.fetch_budget_ms + 1_000),
        ))
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

    // The anonymous click beacon (M6-T03). A tiny POST, fire-and-forget; it shares the suggest-tier
    // rate limit (a click happens far less often than a keystroke) and always answers 204.
    let interaction_routes = Router::new()
        .route("/interaction", axum::routing::post(interaction::handler))
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

    // The entity panel (M8-T02). Out of band, like the summary: the search path never waits for
    // it, and a slow or empty answer degrades to a rail that is simply not there. It reads the
    // knowledge index and nothing else — no egress, and no cache keyed by the query.
    let knowledge_routes = Router::new()
        .route("/knowledge", get(knowledge::panel))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            limit_knowledge,
        ))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            Duration::from_millis(state.config.api.timeout_search_ms),
        ))
        .with_state(state.clone());

    let ops = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics_handler))
        .with_state(state.clone());

    // The admin surface is **JSON only** now — the console itself is a set of pages in the Next.js
    // app, which reaches these through the same `/api/v1/*` proxy as the search UI. Nothing here
    // renders HTML; the frontend lives in one place ([[web]]), not split across two servers.
    let admin_api = Router::new()
        .route("/crawler/channels", get(admin_crawler::channels))
        .route(
            "/crawler/sources/health",
            get(admin_crawler::sources_health),
        )
        .route("/crawler/weak-coverage", get(admin_crawler::weak_coverage))
        .route(
            "/crawler/weak-coverage/forget",
            axum::routing::post(admin_crawler::weak_forget),
        )
        .route(
            "/crawler/sources",
            get(admin_crawler::sources).post(admin_crawler::add_source),
        )
        .route(
            "/crawler/sources/remove",
            axum::routing::post(admin_crawler::remove_source),
        )
        .route("/crawler/status", get(admin_crawler::status))
        .route("/crawler/events", get(admin_crawler::events))
        .route("/crawler/documents", get(admin_crawler::documents))
        .route(
            "/crawler/enqueue",
            axum::routing::post(admin_crawler::enqueue),
        )
        .route("/crawler/pause", axum::routing::post(admin_crawler::pause))
        .route(
            "/crawler/registry",
            axum::routing::post(admin_crawler::registry_edit),
        )
        .route("/politeness", axum::routing::post(admin::set_politeness))
        .route("/status", get(admin::status))
        .route("/config", get(admin::config))
        .route("/eval", get(admin_eval::status))
        .route("/media", get(admin::media))
        .route("/interaction", get(admin::interaction))
        .route(
            "/integrations",
            get(admin::integrations).post(admin::set_integrations),
        )
        .route("/queue", get(admin_queue::status))
        .route("/queue/replay", axum::routing::post(admin_queue::replay))
        .route(
            "/queue/dead/replay",
            axum::routing::post(admin_queue::replay_one),
        )
        .route(
            "/queue/dead/drop",
            axum::routing::post(admin_queue::drop_one),
        )
        .route(
            "/takedown",
            axum::routing::post(admin_maintenance::takedown),
        )
        .route("/device", axum::routing::post(admin::set_device))
        .route("/log-level", axum::routing::post(admin::set_log_level))
        .layer(search_budget)
        .with_state(state.clone());

    // OCR takes an *image* body — up to `max_image_bytes`, three orders of magnitude past the 8 KB
    // default the text endpoints live under. It therefore cannot sit inside the global body-limit
    // layer below (a limit applied outside a route binds it regardless of any looser inner limit),
    // so it is built here with its own limit and merged at the top level, outside that layer. Its
    // timeout is looser than search — tesseract runs in seconds, the sidecar up to its own timeout —
    // and bounded by the sidecar timeout plus slack so a wedged VLM cannot hold the request open.
    let ocr_route = Router::new()
        .route("/ocr", axum::routing::post(ocr::handler))
        // Image-similarity search also takes an image body, so it shares this large-body group and
        // its media rate limit. Its work is an embed + an ANN query, well within the group timeout.
        .route("/search/image", axum::routing::post(image_search::handler))
        .layer(middleware::from_fn_with_state(state.clone(), limit_ocr))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            Duration::from_millis(state.config.media.sidecar.timeout_ms + 10_000),
        ))
        .layer(RequestBodyLimitLayer::new(
            state.config.media.max_image_bytes,
        ))
        .with_state(state.clone());

    // Transcription takes an *audio* body — its own large-body group, bounded by the STT timeout.
    let stt_route = Router::new()
        .route("/transcribe", axum::routing::post(stt::handler))
        .layer(middleware::from_fn_with_state(state.clone(), limit_ocr))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            Duration::from_millis(state.config.stt.timeout_ms + 10_000),
        ))
        .layer(RequestBodyLimitLayer::new(state.config.stt.max_audio_bytes))
        .with_state(state.clone());

    // The live entity fallback's render call carries a whole Wikidata document — a country's
    // runs to a few megabytes — so, like OCR, it lives outside the global 8 KB body limit (a limit
    // applied outside a route binds it regardless of any looser inner one) and is merged at the
    // top level with its own.
    let knowledge_render = Router::new()
        .route(
            "/knowledge/render",
            axum::routing::post(knowledge::render_document),
        )
        .route(
            "/knowledge/resolve-live",
            axum::routing::post(knowledge::resolve_live),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            limit_knowledge,
        ))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            Duration::from_millis(state.config.api.timeout_search_ms),
        ))
        // Seven candidate documents, and a country's runs to megabytes on its own.
        .layer(RequestBodyLimitLayer::new(32 * 1024 * 1024))
        .with_state(state.clone());

    let core = Router::new()
        .nest(
            "/api/v1",
            api.merge(summary)
                .merge(translate)
                .merge(image_web)
                .merge(suggest_routes)
                .merge(interaction_routes)
                .merge(knowledge_routes),
        )
        .nest("/api/v1/admin", admin_api)
        .merge(ops)
        // The 8 KB default guards the text endpoints. OCR is deliberately outside it (see above).
        .layer(RequestBodyLimitLayer::new(
            state.config.api.body_limit_default,
        ));

    Router::new()
        .merge(core)
        .nest("/api/v1", ocr_route)
        .nest("/api/v1", knowledge_render)
        .nest("/api/v1", stt_route)
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
        // No global body-limit layer here: it would re-cap `/ocr` at 8 KB. The 8 KB default is
        // applied to `core` above; `/ocr` carries its own, larger limit.
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

async fn limit_knowledge(State(state): State<AppState>, req: Request, next: Next) -> Response {
    enforce(&state, "/knowledge", ratelimit::KNOWLEDGE, req, next).await
}

async fn limit_suggest(State(state): State<AppState>, req: Request, next: Next) -> Response {
    enforce(&state, "/suggest", ratelimit::SUGGEST, req, next).await
}

async fn limit_ocr(State(state): State<AppState>, req: Request, next: Next) -> Response {
    // The same budget as other media work: OCR is CPU-heavy (tesseract) or holds a GPU slot (the
    // sidecar), so what matters is how many one client can have in flight, not raw request rate.
    // Two buckets, not one: a reader who reverse-searched ten pictures should still be able to
    // read one (M10-T03.6). Same limit each.
    let key = if req.uri().path().ends_with("/search/image") {
        "/search/image"
    } else {
        "/ocr"
    };
    enforce(&state, key, ratelimit::MEDIA, req, next).await
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
    sample_queue_gauges(&state).await;
    sample_crawl_gauges(&state).await;
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

/// Read the ingestion queue's real numbers at scrape time.
///
/// Sampled here rather than pushed from the indexer because the indexer is not always running, and
/// a backlog nobody is draining is precisely the condition worth alerting on. A gauge that only
/// updates while a worker is alive would go silent exactly when it matters.
///
/// Connects per scrape rather than holding a connection in [`AppState`]. Scrapes are infrequent,
/// and the alternative is a Redis handle on the serving path that has to be kept healthy for the
/// benefit of a metrics endpoint. **Every failure here is silent on purpose:** `/metrics` must
/// answer even when Redis does not, or the monitoring goes dark at the same moment as the thing it
/// monitors — and a liveness probe shares this port.
/// Read the crawler's fetched/revisited split at scrape time.
///
/// Sampled from the shared crawl counters rather than pushed, for the same reason as the queue
/// gauges: the crawler is a separate process that restarts, and a gauge that only updates while it
/// is alive goes dark exactly when an operator is asking why. Silent on any failure — /metrics
/// shares a port with the liveness probe.
async fn sample_crawl_gauges(state: &AppState) {
    // Reuse the shared, once-connected store rather than reconnecting each sample.
    let Some(stats) = state.crawl_stats() else {
        return;
    };
    let snap = stats.snapshot().await;
    if snap.unavailable {
        return;
    }
    state.metrics.set_gauge(
        metrics::CRAWL_FETCHED,
        metrics::CRAWL_FETCHED_HELP,
        snap.fetched,
    );
    state.metrics.set_gauge(
        metrics::CRAWL_REVISITED,
        metrics::CRAWL_REVISITED_HELP,
        snap.revisited,
    );
}

async fn sample_queue_gauges(state: &AppState) {
    let cfg = &state.config.queue;
    let Ok(queue) = xustive_queue::Queue::connect(&cfg.url, &cfg.index_stream, "indexers").await
    else {
        return;
    };
    if let Ok(depth) = queue.depth().await {
        state.metrics.set_gauge(
            metrics::QUEUE_DEPTH,
            metrics::QUEUE_DEPTH_HELP,
            depth as u64,
        );
    }
    if let Ok(pending) = queue.pending().await {
        state.metrics.set_gauge(
            metrics::QUEUE_PENDING,
            metrics::QUEUE_PENDING_HELP,
            pending as u64,
        );
    }
    if let Ok(dead) = queue.dead_count().await {
        state
            .metrics
            .set_gauge(metrics::QUEUE_DEAD, metrics::QUEUE_DEAD_HELP, dead as u64);
    }
}
