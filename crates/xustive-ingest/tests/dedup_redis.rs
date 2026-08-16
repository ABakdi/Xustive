//! Cross-run deduplication against a real Redis (M2-T05.2).
//!
//! Skips without Redis, like every other Redis-backed suite here.

use xustive_ingest::dedup::Dedup;

fn dedup(ns: &str) -> Option<Dedup> {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into());
    Dedup::connect_in(&url, &format!("test:dedup:{ns}:{}", std::process::id()))
}

/// Connect_in is lazy; prove Redis answers before asserting.
async fn require(ns: &str) -> Option<Dedup> {
    let d = dedup(ns)?;
    // A probe write that we then remove: if Redis is dead this returns fail-open true, but the
    // len check below is the real reachability signal.
    d.is_new("__probe__").await;
    if d.len().await == 0 {
        eprintln!("skipping: no Redis");
        return None;
    }
    d.forget("__probe__").await;
    Some(d)
}

#[tokio::test]
async fn the_first_sighting_is_new_and_the_second_is_a_duplicate() {
    let Some(d) = require("basic").await else {
        return;
    };
    let h = "b3:article-one";
    assert!(d.is_new(h).await, "never seen before");
    assert!(
        !d.is_new(h).await,
        "the same hash is a duplicate the second time"
    );
    // A different article is independent.
    assert!(d.is_new("b3:article-two").await);

    d.forget(h).await;
    d.forget("b3:article-two").await;
}

/// The same article at two different URLs is one document. This is what canonicalisation cannot
/// catch — the URLs genuinely differ — and the reason dedup keys on the body hash, not the URL.
#[tokio::test]
async fn the_same_body_from_two_urls_is_deduplicated() {
    let Some(d) = require("twourls").await else {
        return;
    };
    // Two syndications of one wire story hash the same extracted body.
    let hash = "b3:same-wire-story";
    assert!(d.is_new(hash).await, "first URL: index it");
    assert!(
        !d.is_new(hash).await,
        "second URL, same body: already have it"
    );
    d.forget(hash).await;
}

/// Forgetting a hash — for a takedown or a forced reindex — lets its document back in.
#[tokio::test]
async fn a_forgotten_hash_can_be_indexed_again() {
    let Some(d) = require("forget").await else {
        return;
    };
    let h = "b3:to-be-removed";
    assert!(d.is_new(h).await);
    assert!(!d.is_new(h).await);
    d.forget(h).await;
    assert!(d.is_new(h).await, "after forgetting, it is new again");
    d.forget(h).await;
}
