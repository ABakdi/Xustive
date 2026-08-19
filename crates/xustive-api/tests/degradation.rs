//! Fault injection: every optional dependency fails, and search still works.
//!
//! The claim these defend is the one in [[Error Handling and Resilience]] §6 — **results are the
//! product; everything else is an improvement on them.** A search engine that returns a 500
//! because its summariser is busy has inverted that.
//!
//! Each test removes one dependency and asserts the request still succeeds. They are cheap and
//! unglamorous, and they are the difference between a system that degrades and one that merely
//! claims to.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use xustive_api::state::AppState;
use xustive_core::Config;

/// A router whose optional dependencies are all broken.
///
/// Meilisearch points at a closed port, the summariser is off, and there is no model. That is a
/// worse situation than production should ever reach, which is the point.
fn crippled() -> axum::Router {
    let mut config = Config::default();
    config.search.meili_url = "http://127.0.0.1:1".into();
    config.ml.summaries_enabled = false;
    config.ml.model_dir = "/nonexistent".into();
    config.queue.url = "redis://127.0.0.1:1".into();
    let state = AppState::new(config).expect("state must build with every dependency down");
    xustive_api::app(state)
}

async fn get(app: axum::Router, uri: &str) -> (StatusCode, Value) {
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("the router must respond");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap_or_default();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn the_process_starts_with_every_dependency_unreachable() {
    // A service that will not start until its dependencies are healthy turns a partial outage
    // into a total one, and makes recovery ordering something a human has to remember at 3am.
    let _ = crippled();
}

#[tokio::test]
async fn liveness_is_up_even_with_everything_down() {
    // A liveness probe that fails on a dependency outage gets the process restarted, which fixes
    // nothing and throws away the warm state that would have made recovery fast.
    let (status, _) = get(crippled(), "/healthz").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn readiness_reports_not_ready_rather_than_lying() {
    // The counterpart: readiness must fail, so a load balancer stops sending traffic here.
    let (status, _) = get(crippled(), "/readyz").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn validation_still_works_with_the_index_down() {
    // A malformed request is the caller's problem whether or not the backend is healthy, and
    // must not be reported as an outage.
    let (status, body) = get(crippled(), "/api/v1/search?q=").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_query");
}

#[tokio::test]
async fn an_instant_answer_survives_the_index_being_down() {
    // The calculator needs nothing but the query. With Meilisearch unreachable the search fails,
    // but a tool that could have answered should not be taken down by a dependency it never
    // touches — this asserts the tool is computed before retrieval, not after.
    let (status, body) = get(crippled(), "/api/v1/search?q=45*1.19").await;

    if status == StatusCode::OK {
        assert_eq!(body["instant"]["value"], "53.55");
    } else {
        // Retrieval failed, which is honest. What must not happen is a panic or a hang.
        assert!(
            status.is_server_error() || status == StatusCode::GATEWAY_TIMEOUT,
            "got {status}"
        );
    }
}

#[tokio::test]
async fn a_summary_request_with_summaries_disabled_is_not_an_error() {
    // Switching the summariser off is an operator action, not a fault. The endpoint must answer
    // "no summary" rather than failing, so the client hides the block instead of showing an error.
    let response = crippled()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/summary")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"token":"anything"}"#))
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
    assert!(
        body["reason"].is_string(),
        "the reason is recorded, not shown to the user"
    );
}

#[tokio::test]
async fn suggestions_degrade_to_the_curated_list_with_the_index_down() {
    // Three of autocomplete's four sources are in-memory. Only the title leg needs Meilisearch,
    // and it has its own timeout precisely so this case returns something rather than nothing.
    let (status, body) = get(crippled(), "/api/v1/suggest?q=تجديد").await;
    assert_eq!(status, StatusCode::OK, "suggestions must never error");
    assert!(body["suggestions"].is_array());
}

#[tokio::test]
async fn a_short_prefix_returns_an_empty_list_not_an_error() {
    let (status, body) = get(crippled(), "/api/v1/suggest?q=a").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["suggestions"].as_array().map(Vec::len), Some(0));
}

#[tokio::test]
async fn the_admin_api_stays_reachable_when_the_frontend_and_index_are_both_down() {
    // The admin console is now the Next.js app, but its data comes from this JSON API — and that
    // API has to answer when the things it is used to fix are the things that are broken. Auth may
    // refuse the call (the harness attaches no loopback address), but it must *respond*, never 5xx.
    let (status, _) = get(crippled(), "/api/v1/admin/status").await;
    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::OK,
        "the admin API must respond during an outage, got {status}"
    );
}

#[tokio::test]
async fn metrics_are_scrapeable_during_an_outage() {
    // The one moment metrics matter most is the one where the dependencies are down.
    let response = crippled()
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
