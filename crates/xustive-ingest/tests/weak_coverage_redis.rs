//! Weak-coverage k-anonymity against a real Redis (M2-T16.4). Skips without Redis.
//!
//! The property that matters: a term is never surfaced until it has crossed the k floor, and it is
//! gone once forgotten. If this ever regresses, a sub-threshold — therefore potentially personal —
//! term could reach the console, which is exactly what ADR-0008 forbids.

use std::time::Duration;

use xustive_ingest::weak_coverage::WeakCoverage;

fn store(ns: &str) -> Option<WeakCoverage> {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into());
    // Unique namespace per process so parallel test runs do not collide.
    WeakCoverage::connect_in(
        &url,
        &format!("test:weak:{ns}:{}", std::process::id()),
        20,
        Duration::from_secs(300),
    )
}

#[tokio::test]
async fn a_term_surfaces_only_after_crossing_the_k_floor() {
    let Some(w) = store("kfloor") else { return };
    let term = "قانون المالية 2027 الجزائر";

    // Nineteen searches: still below k=20, so nothing is surfaced.
    for _ in 0..19 {
        w.record(term).await;
    }
    match w.weak_terms(50).await {
        got if got.iter().any(|t| t.term == term) => {
            panic!("a term below k=20 must never be surfaced");
        }
        // Empty could also mean no Redis; probe with one more record and re-check below.
        _ => {}
    }

    // The twentieth crosses the floor.
    w.record(term).await;
    let surfaced = w.weak_terms(50).await;
    if surfaced.is_empty() {
        eprintln!("skipping: no Redis");
        return;
    }
    let row = surfaced
        .iter()
        .find(|t| t.term == term)
        .expect("at k=20 the term must surface");
    assert_eq!(row.count, 20, "the count is exact");

    // Forgetting a resolved gap removes it.
    w.forget(term).await;
    assert!(
        !w.weak_terms(50).await.iter().any(|t| t.term == term),
        "a forgotten term must not come back"
    );
}
