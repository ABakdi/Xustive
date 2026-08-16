//! The shipped seed registry (`data/sources/registry.jsonl`) must always load and satisfy the
//! registry's invariants (M2-T11.3). This is a regression guard: an operator hand-editing the file
//! and breaking a line — a missing `legal_basis`, a malformed record — is caught by `cargo test`
//! rather than at crawler start-up.

use std::path::PathBuf;

use xustive_core::{Lifecycle, Registry};

fn seed_path() -> PathBuf {
    // CARGO_MANIFEST_DIR is crates/xustive-core; the data lives at the workspace root.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/sources/registry.jsonl")
}

#[test]
fn the_shipped_seed_loads_and_holds_every_invariant() {
    let path = seed_path();
    let reg = Registry::load(path.to_str().unwrap())
        .unwrap_or_else(|e| panic!("seed registry must load: {e}"));

    assert!(
        reg.len() >= 90,
        "seed shrank unexpectedly: {} sources",
        reg.len()
    );

    // Every record parsed, so every record has a legal_basis (serde enforces it). Check the two
    // policy invariants that are ours, not serde's:
    for s in reg.sources() {
        assert!(!s.id.is_empty(), "a source has no id");
        assert!(!s.entry_points.is_empty(), "{} has no entry point", s.id);
        // The seed is unreviewed: nothing is approved or past `proposed` yet, so nothing is
        // crawlable straight out of the file — a human has to advance it first.
        assert!(
            !s.approved,
            "{} is approved in the seed — must be a human decision",
            s.id
        );
        assert_eq!(
            s.lifecycle,
            Lifecycle::Proposed,
            "{} is past proposed in the seed",
            s.id
        );
        assert!(
            !s.is_crawlable(),
            "{} is crawlable straight from the seed",
            s.id
        );
    }

    // Ids are unique — a duplicate would mean upsert silently overwrote a real source.
    let mut ids: Vec<&str> = reg.sources().iter().map(|s| s.id.as_str()).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    assert_eq!(before, ids.len(), "duplicate source id in the seed");
}
