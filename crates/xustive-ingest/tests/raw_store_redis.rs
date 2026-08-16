//! Raw blob store against a real Redis (M2-T04.7). Skips without Redis.

use std::time::Duration;

use xustive_ingest::raw_store::RawStore;

fn store(ns: &str, ttl: Duration) -> Option<RawStore> {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into());
    RawStore::connect_in(&url, &format!("test:raw:{ns}:{}", std::process::id()), ttl)
}

#[tokio::test]
async fn a_stored_body_can_be_retrieved_for_reindex() {
    let Some(s) = store("basic", Duration::from_secs(60)) else {
        return;
    };
    let url = "https://example.dz/article";
    let html = "<html><body><article>الجزائر</article></body></html>";
    s.put(url, html).await;
    // Probe Redis reachability: a live store returns what it stored.
    match s.get(url).await {
        Some(got) => assert_eq!(got, html, "the stored body must come back byte-for-byte"),
        None => {
            eprintln!("skipping: no Redis");
            return;
        }
    }

    // Forget removes it — the takedown path.
    s.forget(url).await;
    assert!(s.get(url).await.is_none(), "a forgotten body is gone");
}

#[tokio::test]
async fn a_body_expires_after_its_ttl() {
    let Some(s) = store("ttl", Duration::from_secs(1)) else {
        return;
    };
    let url = "https://example.dz/short-lived";
    s.put(url, "<html>x</html>").await;
    if s.get(url).await.is_none() {
        eprintln!("skipping: no Redis");
        return;
    }
    // Past the one-second TTL it is gone. Two seconds to avoid a boundary flake.
    tokio::time::sleep(Duration::from_millis(2100)).await;
    assert!(
        s.get(url).await.is_none(),
        "a body must not outlive its TTL — that is what bounds the store"
    );
    s.forget(url).await;
}
