//! Ingestion: turning live web pages into indexable documents.
//!
//! Three concerns, deliberately separated because they fail differently:
//!
//! - [`robots`] — whether we are allowed to fetch, and how fast. Fails **closed**.
//! - [`fetch`] — getting bytes safely and politely.
//! - [`parse`] — bytes to a canonical `Document`, with [`date`] handling the hardest part.
//!
//! This is the minimum path from a seed URL to a searchable document. The full crawler — a
//! persistent frontier, adaptive revisit scheduling, distributed politeness state and the social
//! connectors — is a later milestone.

pub mod date;
pub mod exclusion;
pub mod fetch;
pub mod frontier;
pub mod parse;
pub mod robots;
pub mod robots_cache;
pub mod rules;
pub mod sitemap;

pub use fetch::{FetchConfig, FetchError, Fetched, Fetcher};
pub use parse::{ParseConfig, ParseError, Parsed, Parser};
pub use robots::{Politeness, Robots};
