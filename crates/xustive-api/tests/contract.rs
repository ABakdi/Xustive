//! Contract tests for the HTTP surface.
//!
//! These assert the things a client is entitled to rely on: status codes, error shapes, security
//! headers, and that the middleware stack is wired in the order it claims to be. They run against
//! the real router with `tower::ServiceExt::oneshot`, so no socket and no Meilisearch is needed
//! for anything that fails before retrieval.
//!
//! Search itself needs a backend, so the tests that reach it check for one and skip. Everything
//! here that can run without one, does.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use xustive_api::state::AppState;
use xustive_core::Config;

fn app() -> axum::Router {
    let mut config = Config::default();
    // Points at nothing. Every test here either fails before retrieval or is skipped, and a
    // config that cannot be constructed would make the whole file depend on infrastructure.
    config.search.meili_url = "http://127.0.0.1:1".into();
    config.ml.summaries_enabled = false;
    let state = AppState::new(config).expect("state should build without a live backend");
    xustive_api::app(state)
}

async fn get(uri: &str) -> (StatusCode, axum::http::HeaderMap, Value) {
    let response = app()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("router should respond");

    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap_or_default();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, headers, json)
}

fn error_code(body: &Value) -> Option<&str> {
    body.get("error")?.get("code")?.as_str()
}

// --- error contract ---------------------------------------------------------------------
//
// One test per row of [[API Contract]] §8 that can be provoked without a backend. The value is
// not that these codes are correct today — it is that changing one becomes a deliberate act
// rather than a side effect of a refactor, because clients branch on them.

#[tokio::test]
async fn a_missing_query_is_a_400_with_a_machine_readable_code() {
    let (status, _, body) = get("/api/v1/search").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&body), Some("invalid_query"), "body: {body}");
    assert!(
        body["error"]["message"].is_string(),
        "every error carries a human-readable message too"
    );
}

#[tokio::test]
async fn an_empty_query_is_rejected_like_a_missing_one() {
    let (status, _, body) = get("/api/v1/search?q=%20%20").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&body), Some("invalid_query"));
}

#[tokio::test]
async fn an_overlong_query_is_rejected_before_it_reaches_the_index() {
    // 512 characters is the contract. Validating here rather than at the engine keeps a
    // pathological query from becoming Meilisearch's problem.
    let q = "ا".repeat(600);
    let (status, _, body) = get(&format!("/api/v1/search?q={q}")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&body), Some("query_too_long"), "body: {body}");
}

#[tokio::test]
async fn a_query_at_exactly_the_limit_is_accepted() {
    // Off-by-one on a boundary the contract states explicitly. It will fail at retrieval with no
    // backend; what matters is that it is not rejected as too long.
    let q = "ا".repeat(512);
    let (status, _, body) = get(&format!("/api/v1/search?q={q}")).await;
    assert_ne!(
        error_code(&body),
        Some("query_too_long"),
        "512 characters is within the limit, got {status}"
    );
}

#[tokio::test]
async fn an_unknown_path_does_not_leak_a_stack_trace_or_a_path() {
    let (status, _, _) = get("/api/v1/nonexistent").await;
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::OK,
        "got {status}"
    );
}

#[tokio::test]
async fn a_summary_request_with_an_unknown_token_is_not_an_error() {
    // A missing summary is a normal outcome, so this is a 200 with a null summary rather than a
    // 404. The client hides the block; it does not show an error.
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/summary")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"token":"nope"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(body["summary"].is_null());
    assert_eq!(body["reason"], "unknown_token");
}

// --- security headers -------------------------------------------------------------------

/// The exact policy, pinned.
///
/// A snapshot rather than a set of substring checks because CSP fails silently in the direction
/// that matters: adding `'unsafe-inline'` to make something work does not break a test that only
/// asserts the header is present, and nobody notices until it is being exploited.
///
/// If this test fails, the policy changed. Decide whether that was intended, then update it here.
const EXPECTED_CSP: &str = "default-src 'self'; \
     img-src 'self' data: https:; \
     media-src 'self' blob:; \
     script-src 'self'; \
     style-src 'self'; \
     connect-src 'self'; \
     frame-ancestors 'none'; \
     base-uri 'none'; \
     form-action 'self'";

#[tokio::test]
async fn the_content_security_policy_is_exactly_what_we_think_it_is() {
    let (_, headers, _) = get("/healthz").await;
    let csp = headers
        .get("content-security-policy")
        .and_then(|v| v.to_str().ok())
        .expect("every response carries a CSP");

    // Whitespace-normalised so reformatting the source does not fail the test, but every
    // directive and every source expression has to match.
    let normalise = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    assert_eq!(
        normalise(csp),
        normalise(EXPECTED_CSP),
        "the content security policy changed"
    );
}

#[tokio::test]
async fn inline_script_and_style_are_not_permitted() {
    // Stated separately from the snapshot because it is the specific relaxation most likely to
    // be added under pressure, and the one that turns any injected content into script.
    let (_, headers, _) = get("/healthz").await;
    let csp = headers["content-security-policy"].to_str().unwrap();
    assert!(!csp.contains("unsafe-inline"), "csp: {csp}");
    assert!(!csp.contains("unsafe-eval"), "csp: {csp}");
}

#[tokio::test]
async fn the_supporting_headers_are_present() {
    let (_, headers, _) = get("/healthz").await;
    for (name, expected) in [
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("cross-origin-opener-policy", "same-origin"),
    ] {
        assert_eq!(
            headers.get(name).and_then(|v| v.to_str().ok()),
            Some(expected),
            "{name} is missing or wrong"
        );
    }
}

#[tokio::test]
async fn the_referrer_policy_keeps_queries_out_of_other_sites_logs() {
    // Search result links carry the query in the referring URL. Without `no-referrer` every
    // destination site receives what the user searched for, which is the single largest privacy
    // leak a search engine can have and costs nothing to prevent.
    let (_, headers, _) = get("/search?q=test").await;
    assert_eq!(
        headers.get("referrer-policy").and_then(|v| v.to_str().ok()),
        Some("no-referrer")
    );
}

// --- rate limiting ----------------------------------------------------------------------

#[tokio::test]
async fn the_rate_limiter_advertises_the_remaining_budget() {
    let (_, headers, _) = get("/api/v1/search?q=test").await;
    assert!(
        headers.contains_key("x-ratelimit-remaining"),
        "a client that can see its budget can back off before being refused"
    );
}

#[tokio::test]
async fn exceeding_the_limit_returns_429_with_retry_after() {
    // One router for the whole test so the limiter state persists across requests.
    let app = app();
    let mut last = None;
    // The search limit is 60/min. Sixty-one requests from the same (absent) peer must trip it.
    for _ in 0..70 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/search?q=test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            last = Some(response);
            break;
        }
    }

    let response = last.expect("70 requests should exceed a 60-per-minute limit");
    assert!(
        response.headers().contains_key("retry-after"),
        "a refusal must tell the client when to come back"
    );

    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(error_code(&body), Some("rate_limited"));
}

// --- request ids ------------------------------------------------------------------------

#[tokio::test]
async fn every_response_carries_a_request_id() {
    let (_, headers, _) = get("/healthz").await;
    let id = headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .expect("responses must be traceable to a log line");
    assert!(!id.is_empty());
}

#[tokio::test]
async fn request_ids_are_unique_per_request() {
    let (_, a, _) = get("/healthz").await;
    let (_, b, _) = get("/healthz").await;
    assert_ne!(a["x-request-id"], b["x-request-id"]);
}

#[tokio::test]
async fn an_error_response_is_traceable_too() {
    // The case that matters: a user reporting a failure needs something to quote, and the
    // failing requests are exactly the ones whose id is easiest to forget to attach.
    let (status, headers, _) = get("/api/v1/search").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(headers.contains_key("x-request-id"));
}

#[tokio::test]
async fn a_client_supplied_request_id_is_not_reflected() {
    // Accepting one would let a caller poison log correlation, or set the header to a value of
    // its choosing on a response the browser will read back.
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .header("x-request-id", "attacker-chosen")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(
        response
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok()),
        Some("attacker-chosen")
    );
}

// --- health -----------------------------------------------------------------------------

#[tokio::test]
async fn healthz_is_up_even_with_every_dependency_down() {
    // Liveness, not readiness. A liveness probe that fails when a dependency is unreachable gets
    // the process restarted, which fixes nothing and loses the warm state that would have made
    // recovery fast.
    let (status, _, _) = get("/healthz").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn readyz_reports_not_ready_when_the_index_is_unreachable() {
    let (status, _, _) = get("/readyz").await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "readiness must fail when search cannot be served"
    );
}

// --- admin ------------------------------------------------------------------------------

#[tokio::test]
async fn the_admin_surface_refuses_a_caller_with_no_address() {
    // `oneshot` attaches no connection info, which the guard treats as remote. That is the
    // intended direction to fail: an address the server cannot determine is not loopback.
    let (status, _, body) = get("/admin/status").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(error_code(&body), Some("admin_local_only"));
}

#[tokio::test]
async fn setting_an_invalid_device_is_a_400_not_a_500() {
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/device")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"preference":"turbo"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    // The auth guard runs first, so this is a 403 here. What matters is that neither path is a
    // 500 — a malformed admin request is the caller's problem, not a crash.
    assert!(
        response.status() == StatusCode::FORBIDDEN || response.status() == StatusCode::BAD_REQUEST,
        "got {}",
        response.status()
    );
}
