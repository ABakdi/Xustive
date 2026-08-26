//! Which facts each kind of thing shows, and in what order.
//!
//! [[ADR-0019 - The Knowledge Layer]] promised that a kind without a template would be a compile
//! error rather than a blank card, and [`template`] is where that promise is kept: an exhaustive
//! match over [`Kind`], with no wildcard arm. Adding a kind stops the build until someone decides
//! what it shows.
//!
//! Only the *selection* lives here. The labels are translations and belong with the other
//! translations, so a fact travels as a machine key — `birth_date`, not "Born" — and reads
//! correctly in four languages.

use crate::entity::{Fact, Value};
use crate::kind::Kind;
use crate::Entity;

/// One row of a panel: a fact key and how many values of it are worth showing.
///
/// The cap is not cosmetic. A film's `cast` has forty entries and its `release_date` has one per
/// country; rendering everything Wikidata holds turns a panel into a data dump and buries the four
/// facts a person came for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub key: &'static str,
    pub max: usize,
}

const fn one(key: &'static str) -> Row {
    Row { key, max: 1 }
}
const fn upto(key: &'static str, max: usize) -> Row {
    Row { key, max }
}

const PERSON: &[Row] = &[
    upto("occupation", 3),
    one("birth_date"),
    one("death_date"),
    one("birth_place"),
    upto("citizenship", 2),
    // Ahead of notable works for footballers and office-holders, because for those two
    // groups — which is most of who gets searched here — it is the answer to the question.
    upto("member_of_team", 2),
    upto("position_held", 2),
    upto("notable_work", 3),
];
const FILM: &[Row] = &[
    one("release_date"),
    upto("director", 2),
    upto("genre", 3),
    one("duration"),
    upto("cast", 4),
    upto("screenwriter", 2),
    upto("original_language", 2),
    upto("review_score", 3),
];
const SERIES: &[Row] = &[
    one("release_date"),
    upto("genre", 3),
    upto("cast", 4),
    upto("original_language", 2),
    upto("review_score", 3),
];
const PLACE: &[Row] = &[
    one("country"),
    one("located_in"),
    one("population"),
    one("area"),
];
const ORGANISATION: &[Row] = &[
    one("inception"),
    one("headquarters"),
    one("country"),
    upto("founder", 2),
    one("chief_executive"),
];
const PRODUCT: &[Row] = &[one("inception"), upto("founder", 2), one("publisher")];
const BOOK: &[Row] = &[
    upto("author", 2),
    one("release_date"),
    one("publisher"),
    upto("genre", 2),
    upto("original_language", 1),
];
const MUSIC: &[Row] = &[
    upto("performer", 2),
    one("release_date"),
    upto("genre", 2),
    one("duration"),
];
const EVENT: &[Row] = &[one("release_date"), one("country"), one("located_in")];
const SPECIES: &[Row] = &[one("taxon_name")];
const CONCEPT: &[Row] = &[];

/// The rows for each kind, in the order a reader wants them.
///
/// Exhaustive by construction — there is deliberately no `_` arm, so adding a kind stops the build
/// until someone decides what it shows.
pub fn template(kind: Kind) -> &'static [Row] {
    match kind {
        Kind::Person => PERSON,
        Kind::Film => FILM,
        Kind::Series => SERIES,
        Kind::Place => PLACE,
        Kind::Organisation => ORGANISATION,
        Kind::Product => PRODUCT,
        Kind::Book => BOOK,
        Kind::Music => MUSIC,
        Kind::Event => EVENT,
        Kind::Species => SPECIES,
        Kind::Concept => CONCEPT,
    }
}

/// The facts to render, in template order, capped per key.
///
/// Values are ordered before capping rather than taken as they arrive. Wikidata's statement order
/// is editorial, not chronological, so a film with three national release dates would otherwise
/// show whichever one an editor happened to list first — and "released 1967" for a 1966 film is
/// wrong in the quiet way that is hardest to notice.
pub fn select(entity: &Entity) -> Vec<&Fact> {
    let mut out = Vec::new();
    for row in template(entity.kind) {
        let mut matching: Vec<&Fact> = entity.facts.iter().filter(|f| f.key == row.key).collect();
        matching.sort_by(|a, b| {
            order_key(a)
                .partial_cmp(&order_key(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out.extend(matching.into_iter().take(row.max));
    }
    out
}

/// How to order several values of the same key.
///
/// Dates ascend, so the earliest wins a cap of one — a film's release date is its first release.
/// Scores descend, so a capped list keeps the highest rather than the first. Everything else keeps
/// the order the publisher gave, which is the order its editors chose.
fn order_key(f: &Fact) -> f64 {
    match &f.value {
        Value::Date { at, .. } => *at as f64,
        Value::Score { value, best, .. } if *best > 0.0 => -(value / best),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::{DatePrecision, Provenance};

    fn fact(key: &str, value: Value) -> Fact {
        Fact {
            key: key.into(),
            value,
            provenance: Provenance::wikidata("Q1"),
            as_of: None,
        }
    }

    fn date(at: i64) -> Value {
        Value::Date {
            at,
            precision: DatePrecision::Day,
        }
    }

    #[test]
    fn every_kind_has_a_template_and_only_concept_is_empty() {
        // The exhaustive match is the compile-time half of the guarantee; this is the other half,
        // catching a kind given an empty row list by accident rather than by decision.
        for kind in [
            Kind::Person,
            Kind::Film,
            Kind::Series,
            Kind::Place,
            Kind::Organisation,
            Kind::Product,
            Kind::Book,
            Kind::Music,
            Kind::Event,
            Kind::Species,
        ] {
            assert!(!template(kind).is_empty(), "{kind:?} has no rows");
        }
        assert!(
            template(Kind::Concept).is_empty(),
            "concept is the description-only floor"
        );
    }

    #[test]
    fn the_earliest_release_date_wins_a_cap_of_one() {
        // Wikidata's order is editorial, not chronological. A film with three national release
        // dates must not show whichever one an editor listed first — "released 1967" for a 1966
        // film is wrong in the quiet way that is hardest to notice.
        let mut e = Entity::new("Q1", Kind::Film, 0);
        e.facts = vec![
            fact("release_date", date(1_000_000)),
            fact("release_date", date(10)),
            fact("release_date", date(500_000)),
        ];
        let picked = select(&e);
        assert_eq!(picked.len(), 1);
        assert!(matches!(picked[0].value, Value::Date { at: 10, .. }));
    }

    #[test]
    fn a_capped_score_list_keeps_the_highest_not_the_first() {
        let mut e = Entity::new("Q1", Kind::Series, 0);
        let score = |v: f64| Value::Score {
            value: v,
            best: 100.0,
            reviewer: "Rotten Tomatoes".into(),
        };
        e.facts = vec![
            fact("review_score", score(50.0)),
            fact("review_score", score(99.0)),
            fact("review_score", score(70.0)),
            fact("review_score", score(80.0)),
        ];
        let picked: Vec<f64> = select(&e)
            .iter()
            .filter_map(|f| match f.value {
                Value::Score { value, .. } => Some(value),
                _ => None,
            })
            .collect();
        assert_eq!(picked, vec![99.0, 80.0, 70.0]);
    }

    #[test]
    fn facts_come_back_in_template_order_not_storage_order() {
        // The panel's reading order is a decision; the order facts happened to be parsed in is not.
        let mut e = Entity::new("Q1", Kind::Person, 0);
        e.facts = vec![
            fact("birth_date", date(100)),
            fact("occupation", Value::Text("footballer".into())),
        ];
        let keys: Vec<&str> = select(&e).iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, vec!["occupation", "birth_date"]);
    }

    #[test]
    fn a_cast_of_forty_is_capped_to_something_readable() {
        let mut e = Entity::new("Q1", Kind::Film, 0);
        e.facts = (0..40)
            .map(|i| fact("cast", Value::Text(format!("actor {i}"))))
            .collect();
        assert_eq!(select(&e).len(), 4);
    }

    #[test]
    fn a_fact_the_template_does_not_name_is_not_rendered() {
        // The parser stores every property it recognises; the template decides what a card shows.
        // A place does not show its taxon name.
        let mut e = Entity::new("Q1", Kind::Place, 0);
        e.facts = vec![
            fact("taxon_name", Value::Text("Homo sapiens".into())),
            fact("population", Value::Number(800_000.0)),
        ];
        let keys: Vec<&str> = select(&e).iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, vec!["population"]);
    }

    #[test]
    fn a_concept_renders_no_facts_which_is_the_wikipedia_panel_we_already_ship() {
        let mut e = Entity::new("Q1", Kind::Concept, 0);
        e.facts = vec![fact("population", Value::Number(1.0))];
        assert!(select(&e).is_empty());
    }
}
