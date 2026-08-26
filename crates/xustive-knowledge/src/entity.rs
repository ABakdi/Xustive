//! What the knowledge layer stores about one thing in the world.
//!
//! The shape is decided by two constraints from [[ADR-0019 - The Knowledge Layer]]. Facts carry
//! their **source and licence individually** rather than the entity carrying one for all of them,
//! because an entity's parts genuinely come from different places under different terms — claims
//! are CC0, an encyclopedic extract is CC BY-SA, and every image has its own author. Attribution
//! that travels with the field cannot be forgotten by a renderer that forgot the entity had mixed
//! provenance.
//!
//! And values stay **typed rather than pre-formatted**. A date rendered at harvest time is a date
//! rendered in one language, and this product answers in four.

use serde::{Deserialize, Serialize};

use crate::kind::Kind;

/// The four languages the interface speaks. Darija reads Arabic when it has nothing of its own —
/// never English, which is the rule [[Milestone 1B - Frontend and Instant Answers]] settled.
pub const LANGS: [&str; 4] = ["ar", "ary", "fr", "en"];

/// Labels and aliases per language.
///
/// Aliases are what make resolution work across scripts: `سبيلبرغ`, `Spielberg` and `Steven
/// Spielberg` are one entity, and the store has to hold all three to be found by any of them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Names {
    /// Preferred label per language code. Missing languages are absent, not empty strings — the
    /// difference matters when deciding whether to fall back.
    #[serde(default)]
    pub labels: Vec<(String, String)>,
    /// Every other name this entity is known by, per language.
    #[serde(default)]
    pub aliases: Vec<(String, String)>,
}

impl Names {
    pub fn label(&self, lang: &str) -> Option<&str> {
        self.labels
            .iter()
            .find(|(l, _)| l == lang)
            .map(|(_, v)| v.as_str())
    }

    /// The label to show, falling back the way the product falls back everywhere else: Darija to
    /// Arabic, then the interface language, then any language at all — a name in the wrong script
    /// still identifies the thing, whereas no name identifies nothing.
    pub fn best_label(&self, lang: &str) -> Option<&str> {
        let chain: &[&str] = match lang {
            "ary" => &["ary", "ar", "fr", "en"],
            "ar" => &["ar", "ary", "fr", "en"],
            "fr" => &["fr", "en", "ar"],
            _ => &["en", "fr", "ar"],
        };
        chain
            .iter()
            .find_map(|l| self.label(l))
            .or_else(|| self.labels.first().map(|(_, v)| v.as_str()))
    }

    /// Every distinct string this entity answers to, for indexing. Order is stable so a re-harvest
    /// that changed nothing produces an identical document and no index churn.
    pub fn all_strings(&self) -> Vec<&str> {
        let mut out: Vec<&str> = self
            .labels
            .iter()
            .chain(self.aliases.iter())
            .map(|(_, v)| v.as_str())
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// Where a fact came from and under what terms. Carried per fact, never per entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// The publisher, as a person would name it: `Wikidata`, `Wikipedia`, `Rotten Tomatoes`.
    pub source: String,
    /// An SPDX-ish identifier. Rendered into the attribution the licence actually requires, which
    /// is why it is stored rather than assumed.
    pub licence: String,
    /// Where a reader can check it, when there is such a place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl Provenance {
    /// Wikidata claims are CC0 — the reason the whole design can aggregate freely.
    pub fn wikidata(id: &str) -> Self {
        Self {
            source: "Wikidata".into(),
            licence: "CC0-1.0".into(),
            url: Some(format!("https://www.wikidata.org/wiki/{id}")),
        }
    }
}

/// A typed value. Formatting happens at render time, in the reader's language.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Value {
    Text(String),
    /// Unix seconds plus the precision the publisher actually asserted. A year-precision birth date
    /// rendered as a full date invents two fields nobody claimed.
    Date {
        at: i64,
        precision: DatePrecision,
    },
    Number(f64),
    /// A number with a unit, kept apart from `Number` so a renderer cannot print a runtime as a
    /// population.
    Quantity {
        amount: f64,
        unit: String,
    },
    /// A rating, with the scale it was given on and who gave it. Both are required: `85` means
    /// nothing without `/100`, and means something different from Metacritic than from an audience.
    Score {
        value: f64,
        best: f64,
        reviewer: String,
    },
    /// A reference to another entity we may or may not hold. The label is stored so the panel can
    /// render a director's name without a second lookup.
    Entity {
        id: String,
        label: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatePrecision {
    Year,
    Month,
    Day,
}

/// One statement about an entity.
///
/// `key` is a stable machine key, never display text: the interface holds the translations, so a
/// fact harvested once reads correctly in four languages and in a fifth we add later.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    pub key: String,
    pub value: Value,
    pub provenance: Provenance,
    /// When the fact was true, for facts that expire. A population without a year is a number
    /// pretending to be current.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of: Option<i64>,
}

/// An outbound link to an authority, built from an identifier rather than fetched.
///
/// This is the mechanism that lets the panel show IMDb and Rotten Tomatoes without scraping
/// either: Wikidata records their identifiers under a CC0 licence, and a URL template turns an
/// identifier into a link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Authority {
    /// Machine key: `imdb`, `rotten_tomatoes`, `tmdb`, `metacritic`, `musicbrainz`, `wikipedia`.
    pub key: String,
    /// The publisher's own identifier.
    pub id: String,
    pub url: String,
}

/// An image with the attribution its licence requires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    /// The upstream URL. Never handed to the browser directly — it is proxied same-origin so the
    /// reader's address never reaches the host, the rule ADR-0014 established.
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub licence: String,
    /// The file description page, which is where the licence and author can be verified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_url: Option<String>,
}

/// An encyclopedic paragraph. Separate from `Fact` because it is prose under a share-alike licence
/// rather than a claim under CC0, and the difference is visible in what attribution it needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extract {
    pub lang: String,
    pub text: String,
    pub provenance: Provenance,
}

/// One thing in the world, as the knowledge layer holds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    /// The Wikidata QID. Stable, shared by everyone who asks, and — the point of
    /// [[ADR-0019 - The Knowledge Layer]] — legitimate as a cache key in a way a query never is.
    pub id: String,
    pub kind: Kind,
    pub names: Names,
    /// Wikidata's one-line description per language: "American film director", "wilaya of Algeria".
    #[serde(default)]
    pub descriptions: Vec<(String, String)>,
    #[serde(default)]
    pub facts: Vec<Fact>,
    #[serde(default)]
    pub authorities: Vec<Authority>,
    #[serde(default)]
    pub images: Vec<Image>,
    #[serde(default)]
    pub extracts: Vec<Extract>,
    /// How much of the world's attention this entity has, used to break resolution ties. Sitelink
    /// count is a crude proxy and an honest one: it is the number of language editions that
    /// bothered to write about it.
    #[serde(default)]
    pub prominence: u32,
    /// When we last harvested. Distinct from any fact's `as_of`.
    pub updated_at: i64,
}

impl Entity {
    pub fn new(id: impl Into<String>, kind: Kind, updated_at: i64) -> Self {
        Self {
            id: id.into(),
            kind,
            names: Names::default(),
            descriptions: Vec::new(),
            facts: Vec::new(),
            authorities: Vec::new(),
            images: Vec::new(),
            extracts: Vec::new(),
            prominence: 0,
            updated_at,
        }
    }

    pub fn description(&self, lang: &str) -> Option<&str> {
        self.descriptions
            .iter()
            .find(|(l, _)| l == lang)
            .map(|(_, v)| v.as_str())
    }

    pub fn fact(&self, key: &str) -> Option<&Fact> {
        self.facts.iter().find(|f| f.key == key)
    }

    pub fn authority(&self, key: &str) -> Option<&Authority> {
        self.authorities.iter().find(|a| a.key == key)
    }

    /// The extract to show, with the same fallback chain as a label — except that a share-alike
    /// paragraph in the wrong language is worth less than a label in the wrong language, so this
    /// stops at the languages a reader plausibly reads rather than taking anything at all.
    pub fn extract(&self, lang: &str) -> Option<&Extract> {
        let chain: &[&str] = match lang {
            "ary" | "ar" => &["ar", "fr", "en"],
            "fr" => &["fr", "en", "ar"],
            _ => &["en", "fr", "ar"],
        };
        chain
            .iter()
            .find_map(|l| self.extracts.iter().find(|e| e.lang == *l))
    }

    /// Whether this entity has enough to be worth a panel.
    ///
    /// A bare label and nothing else is not knowledge; it is a name we happen to have seen, and
    /// rendering a card around it wastes the most trusted space on the page.
    pub fn is_renderable(&self) -> bool {
        !self.names.labels.is_empty()
            && (!self.facts.is_empty()
                || !self.extracts.is_empty()
                || !self.descriptions.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Names {
        Names {
            labels: vec![
                ("en".into(), "Steven Spielberg".into()),
                ("ar".into(), "ستيفن سبيلبرغ".into()),
            ],
            aliases: vec![("en".into(), "Spielberg".into())],
        }
    }

    #[test]
    fn darija_falls_back_to_arabic_never_english() {
        // The M1B-T08.4 rule. An Algerian reader who set Darija gets Arabic, which they read, and
        // not English, which the generic "first available" fallback would have handed them.
        assert_eq!(names().best_label("ary"), Some("ستيفن سبيلبرغ"));
    }

    #[test]
    fn a_missing_language_still_yields_a_name() {
        // A name in the wrong script identifies the thing; no name identifies nothing.
        let n = Names {
            labels: vec![("en".into(), "Oran".into())],
            aliases: vec![],
        };
        assert_eq!(n.best_label("ar"), Some("Oran"));
    }

    #[test]
    fn every_name_is_indexed_once_and_in_a_stable_order() {
        // Re-harvesting an unchanged entity must produce an identical document, or the index
        // churns on every pass for no reason.
        let n = names();
        let all = n.all_strings();
        assert_eq!(all, vec!["Spielberg", "Steven Spielberg", "ستيفن سبيلبرغ"]);
        assert_eq!(n.all_strings(), all);
    }

    #[test]
    fn a_bare_name_is_not_renderable() {
        // The panel occupies the most trusted space on the page. A card with a title and nothing
        // under it is worse than no card.
        let mut e = Entity::new("Q1", Kind::Person, 0);
        e.names = names();
        assert!(!e.is_renderable());

        e.descriptions.push(("en".into(), "film director".into()));
        assert!(e.is_renderable());
    }

    #[test]
    fn the_extract_chain_stops_at_languages_a_reader_plausibly_reads() {
        let mut e = Entity::new("Q1", Kind::Person, 0);
        e.extracts.push(Extract {
            lang: "de".into(),
            text: "…".into(),
            provenance: Provenance::wikidata("Q1"),
        });
        // German is not one of ours, so there is no extract to show — unlike a label, where any
        // string beats none.
        assert!(e.extract("ar").is_none());
    }

    #[test]
    fn provenance_for_a_claim_is_cc0_and_carries_a_checkable_url() {
        let p = Provenance::wikidata("Q42");
        assert_eq!(p.licence, "CC0-1.0");
        assert_eq!(p.url.as_deref(), Some("https://www.wikidata.org/wiki/Q42"));
    }
}
