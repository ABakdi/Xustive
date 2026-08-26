//! The knowledge layer: what the product knows about things, as opposed to about pages.
//!
//! Search returns documents. This returns the thing itself — who a person is, what a film scored,
//! where a place is — so the rail beside the results answers the question rather than pointing at
//! somewhere it might be answered.
//!
//! ## Why this is a store and not a fetch
//!
//! [[ADR-0019 - The Knowledge Layer]] has the argument in full; the short version is that the
//! serving plane has no route to the internet ([[ADR-0001 - Two-Plane Architecture]]) and a cache
//! keyed by a query is *"a query log with extra steps"* ([[ADR-0008 - No Query Logging]]). Both
//! constraints dissolve once the unit of caching is the **entity** rather than the search: `Q42` is
//! enumerable, identical for everyone who asks, and says nothing about who asked. So the ingestion
//! plane harvests entities on a schedule, and the serving plane reads what is there.
//!
//! The consequence is worth stating as plainly as [[Tool Data Plane]] states its own: an entity
//! nobody has harvested has no panel, and that is correct rather than unfortunate.
//!
//! ## Why the type decides the template
//!
//! Which authorities describe a film is not a judgement — it follows from the entity being a film,
//! and Wikidata records that as a fact under `P31`. [`kind::from_instance_of`] is therefore a
//! lookup, and [`kind::Kind`] is a closed enum so that a kind without a template fails to compile
//! rather than rendering an empty card.
//!
//! ## What this crate is not
//!
//! It does not fetch, and it does not render. Parsing lives here so it can be tested against saved
//! documents; the harvester lives on the ingestion plane where egress is allowed; the templates
//! live where the translations do.

pub mod entity;
pub mod index;
pub mod kind;
pub mod wikidata;

pub use entity::{
    Authority, DatePrecision, Entity, Extract, Fact, Image, Names, Provenance, Value,
};
pub use kind::Kind;
