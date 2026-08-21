//! Live round-trip of the phash → vector reuse cache against a real Redis.
//!
//! Gated: skips (does not fail) when Redis is unreachable at `XUSTIVE_REDIS_URL` (default the dev
//! port), so CI without the service stays green — the same posture as the vector crate's Qdrant
//! round-trip. Uses a throwaway namespace so it cannot collide with real data.

use std::time::Duration;

use xustive_ingest::embed_cache::EmbedCache;

fn redis_url() -> String {
    std::env::var("XUSTIVE_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into())
}

#[tokio::test]
async fn put_then_get_returns_the_same_vector() {
    let Some(cache) =
        EmbedCache::connect_in(&redis_url(), "test_embed_cache", Duration::from_secs(60)).await
    else {
        eprintln!("skipping: Redis not reachable at {}", redis_url());
        return;
    };

    let phash = "0123456789abcdef";
    // A 512-d vector, the real embedding width, with values that must round-trip bit-for-bit.
    let vector: Vec<f32> = (0..512).map(|i| (i as f32) * 0.001 - 0.25).collect();

    // A miss before anything is written.
    assert!(cache.get(phash).await.is_none(), "cache should start empty");

    cache.put(phash, &vector).await;
    let back = cache.get(phash).await.expect("value present after put");
    assert_eq!(back, vector, "vector must round-trip exactly");

    // A different hash is still a miss — the key is content-specific.
    assert!(cache.get("ffffffffffffffff").await.is_none());
}
