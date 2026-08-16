//! Every shipped fingerprint profile must be coherent (M2-T01b.4).
//!
//! The Fingerprint Engine's §4.2 rule is that a profile hangs together as a unit — the TLS, header,
//! and JS-surface signals all agree. This is the CI test the doc mandates: it loads the whole
//! catalogue and asserts [`check`] finds no incoherence in any profile. An incoherent profile is one
//! that would be pinned to an identity and then flagged on its first request, so a typo in the
//! catalogue is a shipped defect, caught here rather than in the field.

use xustive_ingest::fingerprint::{check, Catalogue};

fn catalogue_dir() -> String {
    format!("{}/../../data/fingerprints", env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn every_shipped_profile_is_coherent() {
    let cat = Catalogue::load_dir(&catalogue_dir())
        .unwrap_or_else(|e| panic!("the fingerprint catalogue must load: {e}"));

    assert!(
        cat.len() >= 4,
        "catalogue shrank unexpectedly: {} profiles",
        cat.len()
    );

    let mut failures = Vec::new();
    for p in cat.profiles() {
        for issue in check(p) {
            failures.push(format!("  {}: {}", p.id, issue.message()));
        }
    }
    assert!(
        failures.is_empty(),
        "incoherent profiles in the catalogue:\n{}",
        failures.join("\n")
    );
}

#[test]
fn every_profile_id_is_unique() {
    let cat = Catalogue::load_dir(&catalogue_dir()).unwrap();
    let mut ids: Vec<&str> = cat.profiles().iter().map(|p| p.id.as_str()).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    assert_eq!(before, ids.len(), "duplicate profile id in the catalogue");
}

#[test]
fn the_catalogue_spans_more_than_one_browser_and_os() {
    // Diversity is the point (§4.4): a pool of clones is itself a signal. Assert the seed is not all
    // one browser/OS, without pinning an exact distribution the curator will tune.
    let cat = Catalogue::load_dir(&catalogue_dir()).unwrap();
    let browsers: std::collections::HashSet<_> = cat.profiles().iter().map(|p| p.browser).collect();
    let oses: std::collections::HashSet<_> = cat.profiles().iter().map(|p| p.os).collect();
    assert!(
        browsers.len() >= 2,
        "the catalogue should span multiple browsers"
    );
    assert!(oses.len() >= 2, "and multiple operating systems");
}
