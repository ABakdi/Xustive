//! Live end-to-end test of the interaction store's ranking loop against a real Redis.
//!
//! Gated: skips when Redis is unreachable at `XUSTIVE_REDIS_URL`, so CI without the service stays
//! green. This is the correctness that matters for M6 — that impressions + clicks turn into a CTR
//! only above the k-anonymity floor, and that the click path works from a bare qhash (the shape the
//! API's opaque token uses, so no query text is needed to attribute a click).

use std::time::Duration;

use xustive_ingest::interaction::Interactions;

fn redis_url() -> String {
    std::env::var("XUSTIVE_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into())
}

#[tokio::test]
async fn ctr_surfaces_only_above_the_k_floor() {
    // k = 3: a (query, doc) needs at least 3 impressions before its CTR is surfaced.
    let Some(store) =
        Interactions::connect_in(&redis_url(), "test_interaction", 3, Duration::from_secs(60))
            .await
    else {
        eprintln!("skipping: Redis not reachable at {}", redis_url());
        return;
    };

    // Unique query so the test is isolated from any prior run.
    let query = format!("m6-test-{}", std::process::id());
    let doc = "doc-A".to_string();

    // Two impressions + a click: still below the k-floor of 3, so CTR must NOT be surfaced.
    store.impressions(&query, &[doc.clone()]).await;
    store.impressions(&query, &[doc.clone()]).await;
    store.click(&query, &doc).await;
    let below = store.ctr_for(&query, &[doc.clone()]).await;
    assert!(
        !below.contains_key(&doc),
        "CTR surfaced below the k-floor — k-anonymity breached"
    );

    // A third impression reaches the floor; now the CTR is surfaced and positive (there was a click).
    store.impressions(&query, &[doc.clone()]).await;
    let at_floor = store.ctr_for(&query, &[doc.clone()]).await;
    let ctr = at_floor
        .get(&doc)
        .copied()
        .expect("CTR should surface at the k-floor");
    assert!(ctr > 0.0 && ctr <= 1.0, "CTR out of range: {ctr}");
}

#[tokio::test]
async fn a_click_by_qhash_matches_a_click_by_query() {
    // The API attributes a click from an opaque token → qhash, never the query text. This asserts
    // that path records the same counter the query-text path would.
    let Some(store) = Interactions::connect_in(
        &redis_url(),
        "test_interaction_qhash",
        1,
        Duration::from_secs(60),
    )
    .await
    else {
        eprintln!("skipping: Redis not reachable at {}", redis_url());
        return;
    };
    let query = format!("m6-qhash-{}", std::process::id());
    let doc = "doc-B".to_string();

    store.impressions(&query, &[doc.clone()]).await;
    // Click via the qhash path (what POST /interaction does).
    store
        .click_by_qhash(&Interactions::qhash(&query), &doc)
        .await;

    let ctr = store.ctr_for(&query, &[doc.clone()]).await;
    assert!(
        ctr.get(&doc).copied().unwrap_or(0.0) > 0.0,
        "a qhash click did not register against the query"
    );
}

#[tokio::test]
async fn analytics_readers_surface_above_the_floor() {
    // top_queries (M6-T05) and hot_docs (M6-T06): a query/doc appears only once it clears the floor.
    let Some(store) = Interactions::connect_in(
        &redis_url(),
        "test_interaction_analytics",
        2,
        Duration::from_secs(60),
    )
    .await
    else {
        eprintln!("skipping: Redis not reachable at {}", redis_url());
        return;
    };
    let query = format!("m6-analytics-{}", std::process::id());
    let doc = format!("hotdoc-{}", std::process::id());

    // One search + one click: below the k-floor of 2, so neither reader surfaces it.
    store.query_seen(&query, "news", 42).await;
    store.click(&query, &doc).await;
    assert!(
        !store.top_queries(50).await.iter().any(|s| s.query == query),
        "a below-floor query surfaced in top_queries"
    );

    // A second search reaches the floor: the query now appears, with its category.
    store.query_seen(&query, "news", 42).await;
    let top = store.top_queries(50).await;
    let stat = top
        .iter()
        .find(|s| s.query == query)
        .expect("query should surface at the floor");
    assert_eq!(stat.category, "news");
    assert!(stat.count >= 2);
    // M7-T10: the row carries the last result count and this query's total clicks.
    assert_eq!(stat.result_count, 42, "result count not recorded");
    assert_eq!(
        stat.clicks, 1,
        "the one click was not attributed to the query"
    );

    // hot_docs with floor 1 surfaces the clicked doc.
    let hot = store.hot_docs(1, 50).await;
    assert!(hot.contains(&doc), "clicked doc missing from hot_docs");
}
