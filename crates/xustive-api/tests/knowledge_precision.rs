//! The entity-panel precision corpus (M8-T02.5, M8-T10.5).
//!
//! Gated: skips (does not fail) when Meilisearch or the knowledge index is absent, so a checkout
//! without `make dev-up` stays green.
//!
//! The rule this enforces is the one M8-T02 is built around: **a panel on the wrong entity is
//! worse than no panel**, because it is a confident wrong answer in the position readers trust
//! most. So the negative half of this corpus is the important half — ordinary searches that must
//! resolve to nothing — and it is deliberately larger than the positive half.
//!
//! Modelled on the M1B-T04.6 matcher corpus, which exists for the same reason on the tool side.

use serde_json::Value;
use xustive_knowledge::index;
use xustive_knowledge::resolve::{self, Candidate};

fn meili_url() -> String {
    std::env::var("MEILI_URL").unwrap_or_else(|_| "http://127.0.0.1:7700".into())
}

async fn client() -> Option<xustive_search::MeiliClient> {
    let c = xustive_search::MeiliClient::new(
        &meili_url(),
        &std::env::var("MEILI_KEY").unwrap_or_default(),
        std::time::Duration::from_secs(10),
    )
    .ok()?;
    // Prove both the server and the index are actually there.
    c.health().await.ok()?;
    c.index_exists(index::INDEX).await.ok()?.then_some(c)
}

macro_rules! require {
    () => {
        match client().await {
            Some(c) => c,
            None => {
                eprintln!(
                    "skipping: no Meilisearch with a `{}` index at {}",
                    index::INDEX,
                    meili_url()
                );
                return;
            }
        }
    };
}

/// Resolve one query exactly as the endpoint does, minus the corpus-agreement signal — which
/// needs the documents index and only ever breaks ties between same-named entities.
async fn resolve_query(c: &xustive_search::MeiliClient, q: &str) -> Option<String> {
    if !resolve::is_panel_shaped(q) {
        return None;
    }
    let query = xustive_search::Query::new(q).limit(10);
    let hits = c.search::<Value>(index::INDEX, &query).await.ok()?.hits;
    let candidates: Vec<Candidate> = hits
        .iter()
        .filter_map(index::from_document)
        .map(|entity| Candidate {
            entity,
            corpus_mentions: 0,
        })
        .collect();
    resolve::choose(q, &candidates).map(|r| r.entity.id)
}

/// Ordinary searches that must produce **no panel at all**.
///
/// Every one of these would return hits from the index — the engine always returns something —
/// and the resolver's job is to know that a hit is not an answer.
const MUST_NOT_RESOLVE: &[&str] = &[
    // Questions, in all four languages. These belong to the summariser.
    "how to cook couscous",
    "what is the capital of algeria",
    "pourquoi le ciel est bleu",
    "comment faire un cv",
    "كيف أطبخ الكسكس",
    "كيفاش ندير حساب بريدي",
    "علاش السما زرقة",
    "وين نلقى الصيدلية",
    // Sentences and intents, not names.
    "best restaurants to visit in oran this summer",
    "cheap flights from algiers to paris",
    "recette gateau algerien facile",
    "أسعار السيارات في الجزائر",
    // Ordinary nouns that overlap entity vocabulary.
    "weather",
    "football",
    "news",
    "الطقس",
    "أخبار",
    // Transactional and navigational.
    "facebook login",
    "traduction francais arabe",
    "convertisseur devise",
    // Nonsense and noise.
    "asdkjhqwe",
    "zzzz",
    "1234567",
    // Too short or too long to be a name.
    "a",
];

/// Names that must resolve, and to what. Kept small: the negative half is what protects readers.
const MUST_RESOLVE: &[(&str, &str)] = &[
    ("Oran", "Q131818"),
    ("وهران", "Q131818"),
    ("Algeria", "Q262"),
    ("Riyad Mahrez", "Q8338725"),
    ("The Battle of Algiers", "Q784812"),
    ("Assia Djebar", "Q157313"),
];

#[tokio::test]
async fn no_ordinary_search_resolves_to_an_entity() {
    let c = require!();
    let mut wrong = Vec::new();
    for q in MUST_NOT_RESOLVE {
        if let Some(id) = resolve_query(&c, q).await {
            wrong.push(format!("{q:?} → {id}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "these ordinary searches produced a panel, which is a confident wrong answer in the most \
         trusted place on the page:\n  {}",
        wrong.join("\n  ")
    );
}

#[tokio::test]
async fn the_names_in_the_seed_list_resolve_to_themselves() {
    let c = require!();
    let mut missed = Vec::new();
    for (q, expected) in MUST_RESOLVE {
        match resolve_query(&c, q).await {
            Some(id) if id == *expected => {}
            other => missed.push(format!("{q:?} → {other:?}, expected {expected}")),
        }
    }
    assert!(missed.is_empty(), "{}", missed.join("\n  "));
}

#[tokio::test]
async fn resolution_is_stable_across_repeated_calls() {
    // The panel must not flicker between two entities on reload, which is what an unstable
    // tie-break would produce and what a single run would never reveal.
    let c = require!();
    for q in ["Oran", "Algeria"] {
        let first = resolve_query(&c, q).await;
        for _ in 0..3 {
            assert_eq!(
                resolve_query(&c, q).await,
                first,
                "{q} resolved inconsistently"
            );
        }
    }
}
