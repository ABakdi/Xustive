//! Harvesting entities into the knowledge index (M8-T01.2).
//!
//! This lives in `toold` rather than in a new binary because `toold` already *is* the sanctioned
//! bridge: on `ingest` for egress and on `core` for storage, taking no user input at all. That
//! last property is the security argument, and duplicating it in a second process would mean
//! duplicating the argument too.
//!
//! It differs from the weather dataset in where it writes — Meilisearch rather than the Redis tool
//! cache — because resolving names across four scripts is a search problem
//! ([[ADR-0019 - The Knowledge Layer]]). Everything else is the same discipline: a fixed list, a
//! fixed cadence, paced requests, and no query anywhere near it.

use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::Value as Json;
use xustive_knowledge::{index, wikidata, Entity, Value};

use crate::FetchError;

const WIKIDATA_API: &str = "https://www.wikidata.org/w/api.php";

/// `wbgetentities` accepts fifty ids per call, and asking for fifty is the difference between one
/// request and fifty for a page's worth of entities.
const BATCH: usize = 50;

/// Between requests. Wikimedia asks for at most a few concurrent connections and a user agent that
/// identifies the caller; we are sequential, which is politer than we need to be and costs nothing
/// on a job with no deadline.
const PACE: Duration = Duration::from_millis(300);

/// Language editions we take extracts from, in the order a reader would prefer them.
const WIKIS: [(&str, &str); 3] = [("ar", "arwiki"), ("fr", "frwiki"), ("en", "enwiki")];

/// One entity plus the article titles needed to fetch its extracts.
pub struct Harvested {
    pub entity: Entity,
    /// `(lang, article title)` for the editions this entity actually has.
    pub articles: Vec<(String, String)>,
}

/// Fetch a batch of entities by id.
///
/// Ids are requested in chunks of [`BATCH`]; a chunk that fails is reported and the rest continue.
/// One unreachable batch should cost its own entities, not the whole pass.
pub async fn fetch_entities(
    client: &reqwest::Client,
    ids: &[String],
    now: i64,
) -> Result<Vec<Harvested>, FetchError> {
    let mut out = Vec::new();
    for chunk in ids.chunks(BATCH) {
        let url = format!(
            "{WIKIDATA_API}?action=wbgetentities&ids={}&props=labels|descriptions|aliases|claims|sitelinks&languages=ar|ary|fr|en&format=json",
            chunk.join("|")
        );
        let body: Json = get_json(client, &url).await?;
        let Some(entities) = body.get("entities").and_then(Json::as_object) else {
            return Err(FetchError::Parse("no entities in response".into()));
        };
        for (_, doc) in entities {
            // A redirected or deleted id comes back flagged rather than absent.
            if doc.get("missing").is_some() {
                continue;
            }
            if let Some(entity) = wikidata::parse(doc, now) {
                out.push(Harvested {
                    articles: articles(doc),
                    entity,
                });
            }
        }
        tokio::time::sleep(PACE).await;
    }
    Ok(out)
}

/// Article titles per language edition, from the entity's sitelinks.
fn articles(doc: &Json) -> Vec<(String, String)> {
    let Some(links) = doc.get("sitelinks").and_then(Json::as_object) else {
        return Vec::new();
    };
    WIKIS
        .iter()
        .filter_map(|(lang, wiki)| {
            let title = links.get(*wiki)?.get("title")?.as_str()?;
            Some((lang.to_string(), title.to_string()))
        })
        .collect()
}

/// Resolve the labels of entities referenced by facts — a film's director, a person's occupation.
///
/// Batched across the whole harvest rather than per entity: a page of films references a few dozen
/// distinct people, and asking once for all of them is one request instead of a few dozen. A
/// reference whose label never resolves keeps an empty one and the renderer drops it, because a
/// director shown as `Q8877` is worse than a director not shown.
pub async fn resolve_labels(
    client: &reqwest::Client,
    ids: &[String],
) -> Result<Vec<(String, String)>, FetchError> {
    let mut out = Vec::new();
    for chunk in ids.chunks(BATCH) {
        let url = format!(
            "{WIKIDATA_API}?action=wbgetentities&ids={}&props=labels&languages=ar|ary|fr|en&format=json",
            chunk.join("|")
        );
        let body: Json = get_json(client, &url).await?;
        let Some(entities) = body.get("entities").and_then(Json::as_object) else {
            continue;
        };
        for (id, doc) in entities {
            // English first here, unlike a panel title: this is a director's name or an
            // occupation, and the alternative to a Latin-script label is usually no label at all.
            let label = ["en", "fr", "ar", "ary"].iter().find_map(|l| {
                doc.pointer(&format!("/labels/{l}/value"))
                    .and_then(Json::as_str)
            });
            if let Some(label) = label {
                out.push((id.clone(), label.to_string()));
            }
        }
        tokio::time::sleep(PACE).await;
    }
    Ok(out)
}

/// Every entity id referenced by a fact whose label is still unknown.
pub fn unresolved_references(harvested: &[Harvested]) -> Vec<String> {
    let mut ids: BTreeSet<String> = BTreeSet::new();
    for h in harvested {
        for fact in &h.entity.facts {
            if let Value::Entity { id, label } = &fact.value {
                if label.is_empty() {
                    ids.insert(id.clone());
                }
            }
        }
    }
    ids.into_iter().collect()
}

/// Fetch one Wikipedia summary and attach it.
///
/// A missing or disambiguation-shaped page is skipped rather than treated as an error: plenty of
/// entities have no article in a given language, and that is ordinary rather than exceptional.
pub async fn attach_extracts(
    client: &reqwest::Client,
    harvested: &mut Harvested,
) -> Result<(), FetchError> {
    let articles = harvested.articles.clone();
    for (lang, title) in articles {
        let url = format!(
            "https://{lang}.wikipedia.org/api/rest_v1/page/summary/{}",
            urlencode(&title.replace(' ', "_"))
        );
        let Ok(body) = get_json(client, &url).await else {
            continue;
        };
        // `standard` excludes disambiguation and redirect stubs, the same filter the existing
        // web-tier panel applies (ADR-0014) and for the same reason: a disambiguation extract is a
        // list of things this is not.
        if body.get("type").and_then(Json::as_str) != Some("standard") {
            continue;
        }
        let Some(text) = body.get("extract").and_then(Json::as_str) else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        let article_url = body
            .pointer("/content_urls/desktop/page")
            .and_then(Json::as_str)
            .unwrap_or("")
            .to_string();
        wikidata::attach_extract(&mut harvested.entity, &lang, text.to_string(), article_url);
        tokio::time::sleep(PACE).await;
    }
    Ok(())
}

/// The documents to write, dropping entities too thin to be worth a panel.
///
/// A bare label and nothing else is a name we happen to have seen, not knowledge, and a card
/// around it wastes the most trusted space on the page.
pub fn documents(harvested: &[Harvested]) -> Vec<Json> {
    harvested
        .iter()
        .filter(|h| h.entity.is_renderable())
        .map(|h| index::to_document(&h.entity))
        .collect()
}

async fn get_json(client: &reqwest::Client, url: &str) -> Result<Json, FetchError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| FetchError::Http(e.without_url().to_string()))?;
    if !response.status().is_success() {
        return Err(FetchError::Http(format!("status {}", response.status())));
    }
    response
        .json()
        .await
        .map_err(|e| FetchError::Parse(e.without_url().to_string()))
}

/// Percent-encode a path segment. Titles carry spaces, apostrophes and non-Latin scripts, all of
/// which break a URL built by concatenation.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// One line of the seed list: an id, the name we expect it to have, and why it is here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seed {
    pub id: String,
    pub expect: String,
    pub note: String,
}

/// Parse the seed list. Comments and blank lines are skipped; a malformed line is skipped rather
/// than fatal, because one bad row should not stop a harvest.
pub fn parse_seeds(text: &str) -> Vec<Seed> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut parts = l.split('\t');
            let id = parts.next()?.trim();
            let expect = parts.next().unwrap_or("").trim();
            if !id.starts_with('Q') || id[1..].parse::<u64>().is_err() {
                return None;
            }
            Some(Seed {
                id: id.to_string(),
                expect: expect.to_string(),
                note: parts.next().unwrap_or("").trim().to_string(),
            })
        })
        .collect()
}

/// Whether a harvested entity is the one the seed meant.
///
/// A hand-written QID is exactly the kind of thing that is wrong without failing: the first draft
/// of the type table used a QID for "wilaya of Algeria" that turned out to be Russia's
/// administrative divisions. Comparing against the name the author expected turns that class of
/// mistake into a log line instead of a wrong panel. An empty expectation waves it through, for
/// seeds added from the demand queue where no human wrote a name.
pub fn matches_expectation(seed: &Seed, entity: &Entity) -> bool {
    if seed.expect.is_empty() {
        return true;
    }
    let wanted = seed.expect.to_lowercase();
    entity
        .names
        .all_strings()
        .iter()
        .any(|n| n.to_lowercase() == wanted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use xustive_knowledge::entity::Names;
    use xustive_knowledge::Kind;

    #[test]
    fn seeds_skip_comments_blanks_and_malformed_rows() {
        let seeds = parse_seeds(
            "# a comment\n\nQ262\tAlgeria\tthe country\nnot-an-id\tx\nQ3561\tAlgiers\n",
        );
        assert_eq!(seeds.len(), 2);
        assert_eq!(seeds[0].id, "Q262");
        assert_eq!(seeds[0].note, "the country");
        assert_eq!(seeds[1].expect, "Algiers");
    }

    #[test]
    fn a_seed_pointing_at_the_wrong_entity_is_caught_by_its_expected_name() {
        // The check that would have caught Q192287 — "administrative divisions of Russia" — sitting
        // in a row labelled as the Algerian wilaya.
        let seed = Seed {
            id: "Q192287".into(),
            expect: "province of Algeria".into(),
            note: String::new(),
        };
        let mut wrong = Entity::new("Q192287", Kind::Place, 0);
        wrong.names = Names {
            labels: vec![("en".into(), "administrative divisions of Russia".into())],
            aliases: vec![],
        };
        assert!(!matches_expectation(&seed, &wrong));

        let mut right = Entity::new("Q240601", Kind::Place, 0);
        right.names = Names {
            labels: vec![("en".into(), "province of Algeria".into())],
            aliases: vec![],
        };
        assert!(matches_expectation(&seed, &right));
    }

    #[test]
    fn a_seed_with_no_expected_name_is_waved_through() {
        // Seeds promoted from the demand queue have no human-written name to check against.
        let seed = Seed {
            id: "Q1".into(),
            expect: String::new(),
            note: String::new(),
        };
        assert!(matches_expectation(
            &seed,
            &Entity::new("Q1", Kind::Concept, 0)
        ));
    }

    #[test]
    fn an_alias_satisfies_the_expectation() {
        // Wikidata's preferred label is not always the name a person would write down.
        let seed = Seed {
            id: "Q83495".into(),
            expect: "Matrix".into(),
            note: String::new(),
        };
        let mut e = Entity::new("Q83495", Kind::Film, 0);
        e.names = Names {
            labels: vec![("en".into(), "The Matrix".into())],
            aliases: vec![("en".into(), "Matrix".into())],
        };
        assert!(matches_expectation(&seed, &e));
    }

    #[test]
    fn article_titles_are_read_from_sitelinks_in_reader_preference_order() {
        let doc = json!({
            "sitelinks": {
                "enwiki": {"title": "The Matrix"},
                "arwiki": {"title": "المصفوفة"},
                "dewiki": {"title": "Matrix"}
            }
        });
        // Arabic first, and the German edition is not one we take extracts from.
        assert_eq!(
            articles(&doc),
            vec![
                ("ar".to_string(), "المصفوفة".to_string()),
                ("en".to_string(), "The Matrix".to_string())
            ]
        );
    }

    #[test]
    fn unresolved_references_are_collected_once_across_the_whole_harvest() {
        // A page of films references the same few dozen people. Asking once for all of them is one
        // request instead of a few dozen.
        use xustive_knowledge::entity::{Fact, Provenance};
        let mk = |id: &str, refs: &[&str]| Harvested {
            entity: {
                let mut e = Entity::new(id, Kind::Film, 0);
                e.facts = refs
                    .iter()
                    .map(|r| Fact {
                        key: "director".into(),
                        value: Value::Entity {
                            id: (*r).to_string(),
                            label: String::new(),
                        },
                        provenance: Provenance::wikidata(id),
                        as_of: None,
                    })
                    .collect();
                e
            },
            articles: vec![],
        };
        let refs = unresolved_references(&[mk("Q1", &["Q9", "Q8"]), mk("Q2", &["Q9"])]);
        assert_eq!(refs, vec!["Q8".to_string(), "Q9".to_string()]);
    }

    #[test]
    fn a_thin_entity_is_not_written() {
        let bare = Harvested {
            entity: {
                let mut e = Entity::new("Q1", Kind::Concept, 0);
                e.names = Names {
                    labels: vec![("en".into(), "Thing".into())],
                    aliases: vec![],
                };
                e
            },
            articles: vec![],
        };
        assert!(documents(&[bare]).is_empty());
    }

    #[test]
    fn a_title_with_spaces_and_arabic_becomes_a_usable_path() {
        assert_eq!(urlencode("The_Matrix"), "The_Matrix");
        assert!(urlencode("المصفوفة").starts_with('%'));
        assert!(!urlencode("L'Oran").contains('\''));
    }
}
