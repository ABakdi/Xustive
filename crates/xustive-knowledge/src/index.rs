//! How an entity is stored in, and read back from, the search index.
//!
//! [[ADR-0019 - The Knowledge Layer]] chose Meilisearch over Redis for this, and the reason is the
//! hard part of the problem: resolving `سبيلبرغ`, `Spielberg` and `spielberg` to one entity is
//! fuzzy multi-script name matching with aliases and typos. That is a search problem, in an engine
//! already running and already tuned for exactly these four languages. Building the same matcher
//! by hand over a key-value store would be reimplementing the thing we run.
//!
//! The document is deliberately thin on top and fat underneath: a few flat fields carry everything
//! the *query* touches, and the whole entity rides along in one nested field that nothing searches.
//! One round trip therefore returns a complete panel, and the matchable surface stays exactly the
//! names and descriptions a person could have typed — an entity is never found by the contents of
//! its own image credit.

use serde_json::{json, Value as Json};

use crate::entity::Entity;

/// Field names, shared with the index settings so the two cannot drift. A searchable attribute
/// naming a field the document does not emit fails silently and forever — it simply matches
/// nothing, which looks like bad relevance rather than a typo.
pub const F_ID: &str = "id";
pub const F_KIND: &str = "kind";
pub const F_NAMES: &str = "names";
pub const F_DESCRIPTIONS: &str = "descriptions";
pub const F_PROMINENCE: &str = "prominence";
pub const F_UPDATED_AT: &str = "updated_at";
/// The nested entity. Never searchable: it holds image credits, licence strings and authority
/// identifiers, none of which anyone types into a search box.
pub const F_ENTITY: &str = "entity";

/// The index this lives in. Follows the `documents` / `comments` / `sources` naming already in use.
pub const INDEX: &str = "knowledge";

/// Flatten an entity into an index document.
///
/// The nested entity is serialised defensively. A shape that will not serialise is a bug, but it
/// is a bug that would otherwise take down a whole harvest pass from inside a loop — and an entity
/// that writes as null is read back as absent, which costs one panel instead of every panel.
pub fn to_document(entity: &Entity) -> Json {
    json!({
        F_ID: entity.id,
        F_KIND: entity.kind.as_str(),
        F_NAMES: entity.names.all_strings(),
        F_DESCRIPTIONS: entity
            .descriptions
            .iter()
            .map(|(_, v)| v.as_str())
            .collect::<Vec<_>>(),
        F_PROMINENCE: entity.prominence,
        F_UPDATED_AT: entity.updated_at,
        F_ENTITY: serde_json::to_value(entity).unwrap_or(Json::Null),
    })
}

/// Read an entity back out of a hit.
///
/// A document whose nested entity will not deserialise is treated as absent rather than as an
/// error: it means an older build wrote a shape this one no longer understands, and the honest
/// response is no panel until the harvester rewrites it — not a failed search.
pub fn from_document(doc: &Json) -> Option<Entity> {
    serde_json::from_value(doc.get(F_ENTITY)?.clone()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::{Names, Value};
    use crate::kind::Kind;

    fn entity() -> Entity {
        use crate::entity::{DatePrecision, Fact, Provenance};
        let mut e = Entity::new("Q83495", Kind::Film, 1_700_000_000);
        // Every `Value` variant, because the first version of this test used an entity with no
        // facts at all — so nothing exercised the enum, and an unserialisable tagging reached a
        // live harvest and panicked mid-pass.
        let prov = Provenance::wikidata("Q83495");
        for value in [
            Value::Text("text".into()),
            Value::Date {
                at: 922_838_400,
                precision: DatePrecision::Day,
            },
            Value::Number(136.0),
            Value::Quantity {
                amount: 136.0,
                unit: "Q7727".into(),
            },
            Value::Score {
                value: 83.0,
                best: 100.0,
                reviewer: "Q105584".into(),
            },
            Value::Entity {
                id: "Q510034".into(),
                label: "Lana Wachowski".into(),
            },
        ] {
            e.facts.push(Fact {
                key: "sample".into(),
                value,
                provenance: prov.clone(),
                as_of: None,
            });
        }
        e.names = Names {
            labels: vec![
                ("en".into(), "The Matrix".into()),
                ("ar".into(), "المصفوفة".into()),
            ],
            aliases: vec![("en".into(), "Matrix".into())],
        };
        e.descriptions = vec![("en".into(), "1999 film".into())];
        e.prominence = 3;
        e
    }

    #[test]
    fn a_document_round_trips_through_the_index_shape() {
        let e = entity();
        let back = from_document(&to_document(&e)).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn every_name_in_any_script_is_matchable() {
        // The point of the whole index choice: one entity, found by any of its names.
        let doc = to_document(&entity());
        let names: Vec<&str> = doc[F_NAMES]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(names.contains(&"The Matrix"));
        assert!(names.contains(&"Matrix"));
        assert!(names.contains(&"المصفوفة"));
    }

    #[test]
    fn the_primary_key_is_the_qid_so_a_reharvest_replaces_rather_than_duplicates() {
        assert_eq!(to_document(&entity())[F_ID], json!("Q83495"));
    }

    #[test]
    fn a_document_this_build_cannot_read_is_absent_rather_than_an_error() {
        // An older build's shape must degrade to "no panel until it is rewritten", not to a
        // failed search for everyone.
        assert!(from_document(&json!({ F_ENTITY: {"id": "Q1"} })).is_none());
        assert!(from_document(&json!({})).is_none());
    }
}
