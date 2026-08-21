//! Live state-machine test for the shared Redis breaker.
//!
//! Gated: skips (does not fail) when Redis is unreachable at `XUSTIVE_REDIS_URL`, so CI without the
//! service stays green. Uses short cooldowns and a throwaway namespace so it runs in ~2 seconds.

use std::time::Duration;

use xustive_queue::breaker::{Config, RedisBreaker};

fn redis_url() -> String {
    std::env::var("XUSTIVE_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into())
}

#[tokio::test]
async fn trips_cools_probes_and_closes() {
    let cfg = Config {
        failure_threshold: 3,
        cooldown: Duration::from_millis(600),
        max_cooldown: Duration::from_secs(5),
        window: Duration::from_secs(10),
        probe_ttl: Duration::from_millis(500),
    };
    let Some(cb) = RedisBreaker::connect_in(&redis_url(), "test_breaker", cfg).await else {
        eprintln!("skipping: Redis not reachable at {}", redis_url());
        return;
    };
    let name = "dep";
    // Clean slate.
    cb.on_success(name).await;

    // Closed: allowed, and below-threshold failures do not trip it.
    assert!(cb.allow(name).await, "closed breaker allows");
    cb.on_failure(name).await;
    cb.on_failure(name).await;
    assert!(
        cb.allow(name).await,
        "two failures below the threshold of three"
    );
    assert_eq!(cb.state(name).await.unwrap(), "closed");

    // The third failure trips it.
    cb.on_failure(name).await;
    assert_eq!(cb.state(name).await.unwrap(), "open");
    assert!(!cb.allow(name).await, "open breaker fails fast");
    assert!(!cb.allow(name).await, "still fast-failing during cooldown");

    // After the cooldown, exactly one probe is let through.
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert!(cb.allow(name).await, "first request after cooldown probes");
    assert!(
        !cb.allow(name).await,
        "no second probe while the first holds the lock"
    );

    // A successful probe closes it.
    cb.on_success(name).await;
    assert_eq!(cb.state(name).await.unwrap(), "closed");
    assert!(cb.allow(name).await, "closed again");

    // Trip once more, then let a probe fail — the cooldown must lengthen (exponential backoff).
    for _ in 0..3 {
        cb.on_failure(name).await;
    }
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert!(cb.allow(name).await, "probe after first cooldown");
    cb.on_failure(name).await; // probe fails → reopen, cooldown doubles to ~1.2 s
    assert_eq!(cb.state(name).await.unwrap(), "open");
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert!(
        !cb.allow(name).await,
        "700 ms < the doubled ~1.2 s cooldown, still open"
    );
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert!(
        cb.allow(name).await,
        "past the doubled cooldown, probes again"
    );

    cb.on_success(name).await; // leave it clean
}
