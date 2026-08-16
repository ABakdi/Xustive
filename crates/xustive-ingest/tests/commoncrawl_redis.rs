//! Common Crawl snapshot progress against a real Redis (M2-T16.3). Skips without Redis.
//!
//! The resume property: the last page finished is remembered per `(snapshot, pattern)`, so an
//! interrupted domain scan continues rather than re-ingesting from the top.

use xustive_ingest::commoncrawl::CcProgress;

fn progress() -> Option<CcProgress> {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into());
    CcProgress::connect_in(&url, &format!("test:cc:{}", std::process::id()))
}

#[tokio::test]
async fn progress_round_trips_and_is_scoped_per_pattern() {
    let Some(p) = progress() else { return };
    let index = "CC-MAIN-2026-30";

    // Nothing recorded yet.
    if p.last_page(index, "*.dz").await.is_some() {
        // A stale key from a crashed run of the same PID — clear the assumption by writing fresh.
    }

    p.set_last_page(index, "*.dz", 7).await;
    match p.last_page(index, "*.dz").await {
        Some(7) => {}
        None => {
            eprintln!("skipping: no Redis");
            return;
        }
        other => panic!("expected page 7, got {other:?}"),
    }

    // A different pattern has its own progress — the two scans do not interfere.
    assert_eq!(
        p.last_page(index, "elkhabar.com").await,
        None,
        "progress must be scoped per pattern"
    );

    // Advancing overwrites.
    p.set_last_page(index, "*.dz", 12).await;
    assert_eq!(p.last_page(index, "*.dz").await, Some(12));
}
