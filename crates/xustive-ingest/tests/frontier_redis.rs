//! The frontier against a real Redis.
//!
//! The property under test is the one that cannot be checked in a single process: **two workers
//! must not claim the same host at once.** Politeness is per host, so two workers each holding
//! their own idea of when a host is due will between them hit it twice as often as either intends
//! — and neither can tell that it happened.
//!
//! That failure appears only under concurrency, which is where it is least visible, so it is
//! asserted with concurrency rather than reasoned about.
//!
//! Skips rather than fails without Redis: this suite must run on a machine with no infrastructure.

use std::collections::HashSet;
use std::time::Duration;

use xustive_ingest::frontier::{Frontier, Pending};

/// A frontier in its own key namespace.
///
/// Per test, because these run in parallel against one Redis. Sharing a namespace made them wipe
/// each other's state and fail in ways that looked exactly like concurrency bugs in the frontier —
/// which is what this suite exists to detect, so a false positive here is expensive.
fn frontier(namespace: &str) -> Option<Frontier> {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into());
    Frontier::connect_in(&url, &format!("test:{namespace}")).ok()
}

macro_rules! require_redis {
    ($ns:expr) => {
        match frontier($ns) {
            Some(f) => {
                // Every test starts from empty, or a previous run's state decides the result.
                f.clear().await;
                f
            }
            None => {
                eprintln!("skipping: no Redis");
                return;
            }
        }
    };
}

fn pending(host: &str, url: &str, priority: i64) -> Pending {
    Pending {
        url: url.to_string(),
        host: host.to_string(),
        source_id: "test".into(),
        depth: 0,
        priority,
    }
}

#[tokio::test]
async fn a_url_is_queued_once_however_many_times_it_is_discovered() {
    // A popular article is linked from every listing page on the site. Queued once per link, the
    // politeness budget goes on re-reading what we already hold.
    let f = require_redis!("a_url_is_queued_once_however_many_times_it_is_discovered");
    let p = pending("example.dz", "https://example.dz/a", 0);

    assert_eq!(f.add(&p).await, Ok(true));
    assert_eq!(
        f.add(&p).await,
        Ok(false),
        "the second discovery queued it again"
    );
    assert_eq!(f.add(&p).await, Ok(false));

    let (waiting, _) = f.depth().await;
    assert_eq!(waiting, 1);
}

#[tokio::test]
async fn the_cheapest_url_is_claimed_first() {
    let f = require_redis!("the_cheapest_url_is_claimed_first");
    f.add(&pending("example.dz", "https://example.dz/deep", 5_000))
        .await
        .expect("add");
    f.add(&pending("example.dz", "https://example.dz/shallow", 0))
        .await
        .expect("add");

    let claim = f.claim(now_ms(), Duration::ZERO).await.expect("a claim");
    assert_eq!(claim.url, "https://example.dz/shallow");
}

#[tokio::test]
async fn a_host_is_not_claimed_twice_before_its_delay_elapses() {
    // The core politeness guarantee. One host, two URLs, a delay: the second claim must come back
    // empty even though there is obviously work available.
    let f = require_redis!("a_host_is_not_claimed_twice_before_its_delay_elapses");
    f.add(&pending("example.dz", "https://example.dz/a", 0))
        .await
        .expect("add");
    f.add(&pending("example.dz", "https://example.dz/b", 1))
        .await
        .expect("add");

    let now = now_ms();
    assert!(f.claim(now, Duration::from_secs(10)).await.is_some());
    assert!(
        f.claim(now, Duration::from_secs(10)).await.is_none(),
        "the host was claimed twice inside its crawl-delay"
    );
    // Once the delay has passed it becomes available again.
    assert!(f
        .claim(now + 11_000, Duration::from_secs(10))
        .await
        .is_some());
}

#[tokio::test]
async fn concurrent_workers_never_claim_the_same_url() {
    // The reason claiming is one Lua script rather than several round trips. Done as separate
    // commands, two workers routinely pick the same host between the read and the write and both
    // fetch it — a failure that appears only under concurrency.
    let f = require_redis!("concurrent_workers_never_claim_the_same_url");
    for i in 0..40 {
        f.add(&pending(
            &format!("host{}.dz", i % 8),
            &format!("https://host{}.dz/page-{i}", i % 8),
            i,
        ))
        .await
        .expect("add");
    }

    let now = now_ms();
    // Sixteen workers racing, no crawl-delay so every host is immediately due again — the
    // arrangement most likely to produce a duplicate if claiming is not atomic.
    let mut set = tokio::task::JoinSet::new();
    for _ in 0..16 {
        let f = f.clone();
        set.spawn(async move {
            let mut got = Vec::new();
            for _ in 0..10 {
                if let Some(c) = f.claim(now, Duration::ZERO).await {
                    got.push(c.url);
                }
            }
            got
        });
    }

    let mut all = Vec::new();
    while let Some(r) = set.join_next().await {
        all.extend(r.expect("task"));
    }

    let unique: HashSet<&String> = all.iter().collect();
    assert_eq!(
        unique.len(),
        all.len(),
        "{} of {} claims were duplicates",
        all.len() - unique.len(),
        all.len()
    );
    assert!(!all.is_empty(), "nothing was claimed at all");
}

#[tokio::test]
async fn a_dead_workers_claim_comes_back() {
    // A worker that dies mid-fetch cannot release anything. Without a sweep the URL is lost and
    // nothing indicates it — the frontier just gets quietly smaller.
    let f = require_redis!("a_dead_workers_claim_comes_back");
    f.add(&pending("example.dz", "https://example.dz/a", 0))
        .await
        .expect("add");

    let now = now_ms();
    assert!(f.claim(now, Duration::ZERO).await.is_some());
    let (_, inflight) = f.depth().await;
    assert_eq!(inflight, 1, "the claim was not recorded");

    // Nothing expires while the worker could still be alive.
    assert_eq!(f.reclaim(now + 1_000).await, 0);
    // Well past the claim TTL, it is swept.
    assert_eq!(f.reclaim(now + 10 * 60 * 1_000).await, 1);
    let (_, inflight) = f.depth().await;
    assert_eq!(inflight, 0);
}

#[tokio::test]
async fn completing_a_claim_clears_it() {
    let f = require_redis!("completing_a_claim_clears_it");
    f.add(&pending("example.dz", "https://example.dz/a", 0))
        .await
        .expect("add");
    let claim = f.claim(now_ms(), Duration::ZERO).await.expect("claim");

    f.complete(&claim.url).await;
    let (_, inflight) = f.depth().await;
    assert_eq!(inflight, 0);
    // Idempotent: an at-least-once pipeline will complete the same URL twice.
    f.complete(&claim.url).await;
}

#[tokio::test]
async fn promoting_moves_a_url_to_the_front() {
    // What the admin console's "priority" control does. Ordering only.
    let f = require_redis!("promoting_moves_a_url_to_the_front");
    f.add(&pending("example.dz", "https://example.dz/first", 0))
        .await
        .expect("add");
    f.add(&pending("example.dz", "https://example.dz/later", 9_000))
        .await
        .expect("add");

    f.promote("example.dz", "https://example.dz/later").await;

    let claim = f.claim(now_ms(), Duration::ZERO).await.expect("claim");
    assert_eq!(claim.url, "https://example.dz/later");
}

#[tokio::test]
async fn promoting_does_not_add_a_url_that_was_never_queued() {
    // `promote` reorders; it must not become a back door for queueing something that never passed
    // the checks on the way in.
    let f = require_redis!("promoting_does_not_add_a_url_that_was_never_queued");
    f.promote("example.dz", "https://example.dz/never-queued")
        .await;
    let (waiting, _) = f.depth().await;
    assert_eq!(waiting, 0, "promote queued an unchecked URL");
}

#[tokio::test]
async fn an_empty_host_stops_being_considered() {
    // Otherwise the scheduler returns to a host with nothing left on every tick, and the loop
    // spins on it instead of doing work elsewhere.
    let f = require_redis!("an_empty_host_stops_being_considered");
    f.add(&pending("example.dz", "https://example.dz/only", 0))
        .await
        .expect("add");

    let now = now_ms();
    assert!(f.claim(now, Duration::ZERO).await.is_some());
    assert!(f.claim(now, Duration::ZERO).await.is_none());
    assert!(f.claim(now + 60_000, Duration::ZERO).await.is_none());
}

fn now_ms() -> i64 {
    // Fixed base plus nothing — these tests pass time explicitly so they do not depend on the
    // wall clock, which would make the delay assertions flaky under load.
    1_786_000_000_000
}
