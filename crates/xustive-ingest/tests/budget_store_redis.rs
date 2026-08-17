//! Fail-closed budget accounting against a real Redis (M2-T01a.10). Skips without Redis.
//!
//! The two properties that carry the safety weight: spending stops at the limit, and a flush of the
//! store (the sentinel disappearing) fails **closed** — every spend denied until an operator
//! re-initialises — rather than resetting every identity's budget to zero.

use xustive_ingest::session::{BudgetStore, Decision};

fn store(ns: &str) -> Option<BudgetStore> {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into());
    BudgetStore::connect_in(&url, &format!("test:budget:{ns}:{}", std::process::id()))
}

#[tokio::test]
async fn spending_stops_at_the_hourly_limit() {
    let Some(s) = store("cap") else { return };
    if !s.mark_alive().await {
        eprintln!("skipping: no Redis");
        return;
    }
    let now = 1_000_000_000; // fixed clock so every call lands in the same hour/day bucket
    let (hourly, daily) = (3u32, 100u32);

    // Three spends fit; the fourth is over the hourly cap.
    for expected_remaining in [2u32, 1, 0] {
        assert_eq!(
            s.try_spend("ig-1", hourly, daily, now).await,
            Decision::Allow {
                remaining_hour: expected_remaining
            }
        );
    }
    assert_eq!(
        s.try_spend("ig-1", hourly, daily, now).await,
        Decision::DenyOverBudget
    );

    // A different identity has its own budget — the counters do not bleed across identities.
    assert!(s.try_spend("ig-2", hourly, daily, now).await.is_allowed());
}

#[tokio::test]
async fn a_flush_fails_closed_until_reinitialised() {
    let Some(s) = store("flush") else { return };
    if !s.mark_alive().await {
        eprintln!("skipping: no Redis");
        return;
    }
    let now = 2_000_000_000;
    // A normal spend works while the sentinel is present.
    assert!(s.try_spend("ig-1", 10, 100, now).await.is_allowed());

    // Simulate a flush by deleting the sentinel out from under the store. Without a durable marker,
    // an absent counter would look like a fresh period and reset the budget — the exact failure the
    // sentinel prevents.
    wipe_sentinel(&s).await;

    // Now every spend is denied as unavailable — fail closed, not fail open.
    assert_eq!(
        s.try_spend("ig-1", 10, 100, now).await,
        Decision::DenyUnavailable
    );

    // Re-initialising (an operator action) restores service.
    assert!(s.mark_alive().await);
    assert!(s.try_spend("ig-1", 10, 100, now).await.is_allowed());
}

/// Delete just the sentinel key, mimicking a partial data loss, using a raw connection so the test
/// does not depend on a store method that should not exist in production.
async fn wipe_sentinel(_s: &BudgetStore) {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into());
    if let Ok(client) = redis::Client::open(url) {
        if let Ok(mut conn) = client.get_multiplexed_async_connection().await {
            // The sentinel key mirrors BudgetStore::sentinel_key for this namespace.
            let key = format!("test:budget:flush:{}:budget:alive", std::process::id());
            let _: Result<(), _> = redis::cmd("DEL")
                .arg(key)
                .query_async::<()>(&mut conn)
                .await;
        }
    }
}
