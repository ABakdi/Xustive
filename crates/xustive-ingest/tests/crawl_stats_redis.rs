//! Reproduce the admin "Live" page's read path against the real Redis.

use xustive_ingest::crawl_stats::CrawlStats;

#[tokio::test]
async fn snapshot_connects_and_reads_when_redis_is_up() {
    let url =
        std::env::var("XUSTIVE_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into());
    let Some(stats) = CrawlStats::connect(&url).await else {
        eprintln!("skipping: could not open a client for {url}");
        return;
    };
    let snap = stats.snapshot().await;
    eprintln!(
        "unavailable={} state={} fetched={} indexed={}",
        snap.unavailable, snap.state, snap.fetched, snap.indexed
    );
    // If Redis is reachable at all, the snapshot must NOT report unavailable.
    assert!(
        !snap.unavailable,
        "snapshot reported Redis unavailable, but a raw connection to {url} works — the multiplexed \
         connection setup is failing"
    );
}
