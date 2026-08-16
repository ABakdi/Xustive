//! SimHash near-duplicate detection against a real Redis (M2-T05.3).
//!
//! Skips without Redis, like every other Redis-backed suite here.

use xustive_core::hash;
use xustive_ingest::simhash_index::{SimHashIndex, NEAR_DISTANCE};

fn index(ns: &str) -> Option<SimHashIndex> {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into());
    SimHashIndex::connect_in(&url, &format!("test:sim:{ns}:{}", std::process::id()))
}

/// Probe that Redis actually answers, since connect_in is lazy.
async fn require(ns: &str) -> Option<SimHashIndex> {
    let idx = index(ns)?;
    idx.insert(0xFFFF_FFFF_FFFF_FFFF, "__probe__").await;
    if idx.find_near(0xFFFF_FFFF_FFFF_FFFF).await.is_none() {
        eprintln!("skipping: no Redis");
        return None;
    }
    Some(idx)
}

#[tokio::test]
async fn a_near_hash_is_found_and_a_far_one_is_not() {
    let Some(idx) = require("near").await else {
        return;
    };
    let original = 0x1234_5678_9ABC_DEF0u64;
    idx.insert(original, "article-1").await;

    // A body reworded a little: three bits flipped, within the "same story" band.
    let reworded = original ^ 0b111;
    assert_eq!(hash::hamming(original, reworded), 3);
    assert_eq!(
        idx.find_near(reworded).await.as_deref(),
        Some("article-1"),
        "a hash within {NEAR_DISTANCE} bits should be found as a near-duplicate"
    );

    // A different article: many bits apart, no shared band, not found.
    let unrelated = original ^ 0xFFFF_0000_0000_0000;
    assert!(
        idx.find_near(unrelated).await.is_none(),
        "an unrelated hash must not be reported as a near-duplicate"
    );
}

/// The real payload: two different SimHashes of genuinely similar text are detected, and two
/// unrelated texts are not. This exercises `simhash` and the index together.
#[tokio::test]
async fn similar_text_is_caught_where_exact_hashing_would_miss_it() {
    let Some(idx) = require("text").await else {
        return;
    };

    let a = "the ministry announced the launch of a new integrated phosphate project in tebessa \
             with an investment of seven billion dollars creating thousands of jobs";
    // The same story, a few words changed — an exact content hash would differ completely.
    let b = "the ministry has announced the launch of a new integrated phosphate project in \
             tebessa with investment of seven billion dollars creating thousands of jobs";
    let c = "the national football team qualified for the next round after winning two to one \
             and the coach said the lineup will change for the coming match";

    let (Some(ha), Some(hb), Some(hc)) = (hash::simhash(a), hash::simhash(b), hash::simhash(c))
    else {
        eprintln!("skipping: text too short for a meaningful simhash");
        return;
    };

    idx.insert(ha, "story-a").await;

    // b is a reworded a: should be found as a near-duplicate of story-a.
    if hash::hamming(ha, hb) <= NEAR_DISTANCE {
        assert_eq!(idx.find_near(hb).await.as_deref(), Some("story-a"));
    }
    // c is a different story: must not match.
    assert!(
        idx.find_near(hc).await.is_none(),
        "an unrelated story must not collapse into story-a"
    );
}
