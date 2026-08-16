//! Circuit breakers against a real Redis (M2-T07.7). Skips without Redis.
//!
//! The property that matters: breaker state is shared, so a second replica (a second `Breakers`
//! against the same keys) sees a breaker the first one tripped, and the cooldown doubles across
//! trips rather than resetting.

use std::time::Duration;

use xustive_ingest::proxy::{Breakers, Scope};

fn breakers(ns: &str) -> Option<Breakers> {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into());
    Breakers::connect_in(&url, &format!("test:brk:{ns}:{}", std::process::id()))
}

#[tokio::test]
async fn a_tripped_breaker_is_visible_to_another_replica_and_doubles() {
    let Some(a) = breakers("share") else { return };
    let Some(b) = breakers("share") else { return };
    let host = Scope::Host("down.dz".into());
    let now = 1_000_000;

    // First trip: 60s cooldown. If Redis is absent, trip returns ZERO — treat as skip.
    let first = a.trip(&host, now).await;
    if first == Duration::ZERO {
        eprintln!("skipping: no Redis");
        return;
    }
    assert_eq!(first, Duration::from_secs(60));

    // A different replica sees it open now, and closed after the cooldown passes.
    assert!(
        b.is_open(&host, now + 30_000).await,
        "another replica must see the open breaker"
    );
    assert!(
        !b.is_open(&host, now + 61_000).await,
        "and see it close after the cooldown"
    );

    // Second trip doubles to 120s — the level persisted in Redis, not in either instance.
    assert_eq!(a.trip(&host, now).await, Duration::from_secs(120));

    // A clean success resets it: the next trip starts from the base again.
    a.reset(&host).await;
    assert!(!b.is_open(&host, now + 1).await);
    assert_eq!(a.trip(&host, now).await, Duration::from_secs(60));

    a.reset(&host).await;
}
