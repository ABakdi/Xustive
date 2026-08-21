//! Live round-trip against a real Qdrant.
//!
//! Gated: if Qdrant is not reachable at `XUSTIVE_QDRANT_URL` (default the dev port), the test skips
//! rather than fails, so CI without the service stays green — the same posture as the media crate's
//! OCR fixture test. It uses a throwaway collection and synthetic vectors, so it proves the client
//! wire format end-to-end **without needing a CLIP model**.

use std::time::Duration;

use xustive_vector::{Payload, Point, SearchFilter, Store};

fn qdrant_url() -> String {
    std::env::var("XUSTIVE_QDRANT_URL").unwrap_or_else(|_| "http://127.0.0.1:6333".into())
}

async fn reachable(store: &Store) -> bool {
    // `count` on a fresh collection would 404; `ensure_collection` is the real reachability probe.
    store.ensure_collection().await.is_ok()
}

fn unit_vector(seed: u64) -> Vec<f32> {
    // A deterministic, L2-normalised 512-d vector. Two seeds that are close produce close vectors.
    let mut v: Vec<f32> = (0..xustive_vector::DIM)
        .map(|i| ((seed.wrapping_add(i as u64) % 97) as f32) - 48.0)
        .collect();
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
    for x in &mut v {
        *x /= norm;
    }
    v
}

#[tokio::test]
async fn upsert_search_delete_round_trip() {
    let collection = "image_clip_test_roundtrip";
    let store =
        Store::new(&qdrant_url(), None, collection, Duration::from_secs(10)).expect("build store");

    if !reachable(&store).await {
        eprintln!("skipping: Qdrant not reachable at {}", qdrant_url());
        return;
    }

    // Clean slate for this document id.
    let doc = "roundtrip-doc";
    store.delete_by_document(doc).await.expect("pre-clean");

    let a = "https://example.dz/a.jpg";
    let b = "https://example.dz/b.jpg";
    let points = vec![
        Point {
            id: xustive_vector::point_id(a),
            vector: unit_vector(1),
            payload: Payload {
                document_id: doc.into(),
                media_url: a.into(),
                source_type: Some("web".into()),
                is_nsfw: false,
                ..Default::default()
            },
        },
        Point {
            id: xustive_vector::point_id(b),
            vector: unit_vector(500),
            payload: Payload {
                document_id: doc.into(),
                media_url: b.into(),
                source_type: Some("web".into()),
                is_nsfw: true, // will be filtered out by the safe filter
                ..Default::default()
            },
        },
    ];
    store.upsert(&points).await.expect("upsert");

    // Query with the exact vector of `a`: it must come back first, and `b` (NSFW) must be filtered.
    let hits = store
        .search(&unit_vector(1), 10, 64, 0.0, &SearchFilter::safe())
        .await
        .expect("search");
    assert!(!hits.is_empty(), "expected at least the exact match");
    assert_eq!(hits[0].payload.media_url, a, "exact match ranks first");
    assert!(
        hits.iter().all(|h| !h.payload.is_nsfw),
        "safe filter must exclude the NSFW point"
    );
    // Self-similarity of a normalised vector is ~1.0.
    assert!(hits[0].score > 0.99, "exact match score {}", hits[0].score);

    // Takedown: deleting the document removes both points.
    store.delete_by_document(doc).await.expect("delete");
    let after = store
        .search(&unit_vector(1), 10, 64, 0.0, &SearchFilter::default())
        .await
        .expect("search after delete");
    assert!(
        after.iter().all(|h| h.payload.document_id != doc),
        "no points for the deleted document remain"
    );
}
