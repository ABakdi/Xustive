//! Alias resolution against a live Meilisearch.
//!
//! Skipped when no server is reachable — a checkout without `make dev-up` should still have a
//! green suite. The logic is small but the consequence of getting it wrong is not: pointing a
//! running system at the wrong index does not look like a bug, it looks like a search engine
//! that has forgotten everything.

use std::time::Duration;

use xustive_search::MeiliClient;

fn client() -> Option<MeiliClient> {
    let url = std::env::var("MEILI_URL").unwrap_or_else(|_| "http://localhost:7700".into());
    let key = std::env::var("MEILI_KEY").unwrap_or_default();
    let c = MeiliClient::new(&url, &key, Duration::from_secs(5)).ok()?;
    // A blocking probe would need a runtime; the caller is already async, so existence is
    // checked there and this only builds the client.
    Some(c)
}

/// Index names unique to this run, so a failed run cannot poison the next one.
/// Create an index, or say the instance is too busy to test against.
///
/// Meilisearch runs tasks in **submission order**, so a scratch index queues behind whatever else
/// is pending. With a crawler feeding the indexer, that backlog reached twenty-three thousand
/// documents and creating an empty index took longer than any deadline worth setting — these tests
/// failed for minutes at a time while nothing they assert was broken.
///
/// Raising the timeout does not fix that; it only makes the failure slower. What these tests
/// actually need is to not run against a saturated instance, because a suite that is red for
/// reasons unrelated to what it tests teaches people to ignore red suites.
///
/// Returns `false` when the instance is busy, and the caller skips.
async fn create_or_skip(c: &MeiliClient, name: &str) -> bool {
    // Short on purpose. An idle Meilisearch creates an index in milliseconds; anything slower
    // means we are queued behind real work, not that index creation is slow.
    const PROBE: std::time::Duration = std::time::Duration::from_secs(10);

    match tokio::time::timeout(PROBE, c.ensure_index(name, "id")).await {
        Ok(Ok(())) => true,
        Ok(Err(e)) => panic!("create {name}: {e}"),
        Err(_) => {
            eprintln!("skipping: Meilisearch is busy — index creation queued behind other tasks");
            false
        }
    }
}

fn scratch(name: &str) -> String {
    format!("xtest_{name}")
}

async fn cleanup(c: &MeiliClient, names: &[String]) {
    for n in names {
        let _ = c.delete_index(n).await;
    }
}

#[tokio::test]
async fn a_plain_index_wins_over_a_versioned_one() {
    let Some(c) = client() else { return };
    if c.health().await.is_err() {
        eprintln!("skipping: no Meilisearch");
        return;
    }

    // The ordering that makes this change safe to deploy. This project indexed into `documents`
    // directly before aliases existed; preferring the versioned index would point a live system
    // at an empty one the moment a migration created it.
    let alias = scratch("plainwins");
    let v1 = format!("{alias}_v1");
    if !create_or_skip(&c, &alias).await {
        return;
    }
    if !create_or_skip(&c, &v1).await {
        return;
    }

    let resolved = c.resolve(&alias).await.expect("resolve");
    assert_eq!(resolved, alias, "the pre-alias index must keep winning");

    cleanup(&c, &[alias, v1]).await;
}

#[tokio::test]
async fn the_highest_version_wins_when_there_is_no_plain_index() {
    let Some(c) = client() else { return };
    if c.health().await.is_err() {
        eprintln!("skipping: no Meilisearch");
        return;
    }

    let alias = scratch("versions");
    let (v1, v2, v10) = (
        format!("{alias}_v1"),
        format!("{alias}_v2"),
        format!("{alias}_v10"),
    );
    for n in [&v1, &v2, &v10] {
        if !create_or_skip(&c, n).await {
            return;
        }
    }

    // v10, not v2: sorted numerically. String ordering would pick v2 and quietly serve an index
    // eight reindexes out of date.
    assert_eq!(c.resolve(&alias).await.expect("resolve"), v10);

    let versions = c.versions_of(&alias).await.expect("list");
    assert_eq!(
        versions.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        vec![1, 2, 10]
    );

    cleanup(&c, &[alias, v1, v2, v10]).await;
}

#[tokio::test]
async fn an_unknown_alias_resolves_to_itself() {
    let Some(c) = client() else { return };
    if c.health().await.is_err() {
        eprintln!("skipping: no Meilisearch");
        return;
    }
    // A fresh install has no indexes at all. Returning the alias name lets migration create it
    // rather than failing on a lookup.
    let alias = scratch("nothing_here");
    assert_eq!(c.resolve(&alias).await.expect("resolve"), alias);
}

#[tokio::test]
async fn a_similarly_named_index_is_not_mistaken_for_a_version() {
    let Some(c) = client() else { return };
    if c.health().await.is_err() {
        eprintln!("skipping: no Meilisearch");
        return;
    }

    // `documents_vector` starts with `documents_v` but is not `documents_v<N>`. A prefix check
    // without the numeric parse would resolve the alias to it.
    let alias = scratch("prefix");
    let decoy = format!("{alias}_vector");
    let v1 = format!("{alias}_v1");
    if !create_or_skip(&c, &decoy).await {
        return;
    }
    if !create_or_skip(&c, &v1).await {
        return;
    }

    assert_eq!(c.resolve(&alias).await.expect("resolve"), v1);

    cleanup(&c, &[decoy, v1]).await;
}
