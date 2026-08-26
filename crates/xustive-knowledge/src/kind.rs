//! What sort of thing an entity is, and how that is decided.
//!
//! The closed enum is the load-bearing part. [[ADR-0019 - The Knowledge Layer]] chooses the panel
//! template by exhaustive match on `Kind`, so adding a kind without giving it a template is a
//! compile error rather than a blank card in the rail. That is the whole reason this is not a
//! string.
//!
//! Deciding the kind is a lookup on Wikidata's *instance of* (`P31`), which is a fact rather than a
//! judgement — the observation that made a model-based router unnecessary.

use serde::{Deserialize, Serialize};

/// The kinds the panel knows how to describe.
///
/// `Concept` is the deliberate floor: anything unrecognised renders a description and an extract,
/// which is exactly the Wikipedia panel that ships today. The feature can therefore never be worse
/// than what it replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Person,
    Film,
    Series,
    Place,
    Organisation,
    Product,
    Book,
    Music,
    Event,
    Species,
    Concept,
}

impl Kind {
    /// The stable string used in the index and on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Person => "person",
            Kind::Film => "film",
            Kind::Series => "series",
            Kind::Place => "place",
            Kind::Organisation => "organisation",
            Kind::Product => "product",
            Kind::Book => "book",
            Kind::Music => "music",
            Kind::Event => "event",
            Kind::Species => "species",
            Kind::Concept => "concept",
        }
    }
}

/// `P31` targets that decide a kind, most specific first.
///
/// Not exhaustive and never will be — Wikidata has tens of thousands of classes and a real
/// resolution would walk `subclass of` upward. This table covers what people search for, and
/// everything it misses lands on [`Kind::Concept`], which still renders. Growing the table is how
/// this improves; guessing is not.
const P31_KINDS: &[(&str, Kind)] = &[
    // People
    ("Q5", Kind::Person),
    // Film and television
    ("Q11424", Kind::Film),       // film
    ("Q24869", Kind::Film),       // feature film
    ("Q506240", Kind::Film),      // television film
    ("Q93204", Kind::Film),       // documentary film
    ("Q202866", Kind::Film),      // animated film
    ("Q5398426", Kind::Series),   // television series
    ("Q581714", Kind::Series),    // animated series
    ("Q1259759", Kind::Series),   // miniseries
    ("Q117467246", Kind::Series), // streaming television series
    // Places
    ("Q515", Kind::Place),      // city
    ("Q486972", Kind::Place),   // human settlement
    ("Q3957", Kind::Place),     // town
    ("Q532", Kind::Place),      // village
    ("Q6256", Kind::Place),     // country
    ("Q3624078", Kind::Place),  // sovereign state
    ("Q10864048", Kind::Place), // first-level administrative division
    ("Q192287", Kind::Place),   // wilaya of Algeria
    ("Q5119", Kind::Place),     // capital city
    ("Q8502", Kind::Place),     // mountain
    ("Q4022", Kind::Place),     // river
    // Organisations
    ("Q43229", Kind::Organisation),
    ("Q4830453", Kind::Organisation), // business
    ("Q783794", Kind::Organisation),  // company
    ("Q476028", Kind::Organisation),  // association football club
    ("Q3918", Kind::Organisation),    // university
    ("Q7278", Kind::Organisation),    // political party
    ("Q11032", Kind::Organisation),   // newspaper
    ("Q1616075", Kind::Organisation), // television station
    // Products
    ("Q2424752", Kind::Product),
    ("Q7397", Kind::Product),    // software
    ("Q620615", Kind::Product),  // mobile app
    ("Q1194058", Kind::Product), // video game — a product before it is a work
    ("Q7889", Kind::Product),    // video game
    // Written works
    ("Q571", Kind::Book),
    ("Q7725634", Kind::Book),  // literary work
    ("Q47461344", Kind::Book), // written work
    ("Q49084", Kind::Book),    // short story
    // Music
    ("Q482994", Kind::Music), // album
    ("Q7366", Kind::Music),   // song
    ("Q134556", Kind::Music), // single
    ("Q215380", Kind::Music), // musical group
    // Events
    ("Q1656682", Kind::Event),
    ("Q13406554", Kind::Event), // sports competition
    ("Q16510064", Kind::Event), // sporting event
    ("Q198", Kind::Event),      // war
    ("Q180684", Kind::Event),   // conflict
    // Life
    ("Q16521", Kind::Species), // taxon
];

/// Decide a kind from an entity's `P31` values.
///
/// Takes them all rather than the first, because Wikidata routinely asserts several — a film is
/// often both `film` and `animated film`, and a person is only ever `human`. The first value with
/// a mapping wins, in the order the caller supplies, which for Wikidata is the order of preference
/// the editors set.
pub fn from_instance_of<S: AsRef<str>>(values: &[S]) -> Kind {
    for v in values {
        let v = v.as_ref();
        if let Some((_, kind)) = P31_KINDS.iter().find(|(qid, _)| *qid == v) {
            return *kind;
        }
    }
    Kind::Concept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_human_is_a_person() {
        assert_eq!(from_instance_of(&["Q5"]), Kind::Person);
    }

    #[test]
    fn an_unmapped_class_lands_on_concept_rather_than_failing() {
        // The floor that makes the whole feature safe to ship: an unknown kind still renders a
        // description and an extract, which is today's Wikipedia panel.
        assert_eq!(from_instance_of(&["Q99999999"]), Kind::Concept);
        assert_eq!(from_instance_of::<&str>(&[]), Kind::Concept);
    }

    #[test]
    fn the_first_recognised_class_wins_when_several_are_asserted() {
        // Wikidata asserts several classes routinely. Taking the first *recognised* one, not the
        // first one, is what stops an unmapped lead value from demoting a film to a concept.
        assert_eq!(from_instance_of(&["Q99999999", "Q11424"]), Kind::Film);
    }

    #[test]
    fn a_wilaya_is_a_place() {
        // The kind an Algeria-first product had better get right.
        assert_eq!(from_instance_of(&["Q192287"]), Kind::Place);
    }

    #[test]
    fn every_kind_has_a_distinct_stable_string() {
        // The strings reach the index and the wire, so a collision would merge two templates and a
        // rename would orphan every stored document.
        let all = [
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
            Kind::Concept,
        ];
        let mut seen: Vec<&str> = all.iter().map(|k| k.as_str()).collect();
        let before = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), before);
    }

    #[test]
    fn the_table_has_no_contradictory_duplicates() {
        // Two rows for one QID mapping to different kinds would make the result depend on table
        // order, which is exactly the kind of silent drift a growing lookup table invites.
        for (i, (qid, kind)) in P31_KINDS.iter().enumerate() {
            for (other, other_kind) in P31_KINDS.iter().skip(i + 1) {
                if qid == other {
                    assert_eq!(kind, other_kind, "{qid} is mapped to two different kinds");
                }
            }
        }
    }
}
