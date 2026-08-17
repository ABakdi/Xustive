//! Bandwidth meter against a real Redis (M2-T07.10). Skips without Redis.

use xustive_ingest::proxy::{cost_per_1k_docs, BandwidthMeter, SourceUsage};

fn meter(ns: &str) -> Option<BandwidthMeter> {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into());
    BandwidthMeter::connect_in(&url, &format!("test:bw:{ns}:{}", std::process::id()))
}

#[tokio::test]
async fn usage_accumulates_per_pool_and_per_source_and_yields_a_cost() {
    let Some(m) = meter("acc") else { return };
    let month = "2026-08";

    m.record(month, "residential", "aps-dz", 500_000_000, 800)
        .await;
    // Probe reachability: after a record, the source should show bytes.
    if m.source_usage(month, "aps-dz").await == SourceUsage::default() {
        eprintln!("skipping: no Redis");
        return;
    }
    m.record(month, "residential", "aps-dz", 500_000_000, 200)
        .await;
    m.record(month, "direct", "elkhabar-com", 100_000_000, 400)
        .await;

    // aps-dz: 1 GB over 1000 docs, all residential.
    let aps = m.source_usage(month, "aps-dz").await;
    assert_eq!(aps.bytes, 1_000_000_000);
    assert_eq!(aps.docs, 1_000);

    // Pool totals separate residential from direct.
    assert_eq!(m.pool_bytes(month, "residential").await, 1_000_000_000);
    assert_eq!(m.pool_bytes(month, "direct").await, 100_000_000);

    // A real cost figure at $8/GB: ~0.93 GB → ~$7.45 per 1k docs.
    let cost = cost_per_1k_docs(aps.bytes, aps.docs, 8.0).unwrap();
    assert!(cost > 7.0 && cost < 8.0, "cost per 1k = {cost}");
}
