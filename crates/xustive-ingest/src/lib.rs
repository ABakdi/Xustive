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

pub mod brave;
pub mod commoncrawl;
pub mod crawl_stats;
pub mod date;
pub mod dedup;
pub mod embed_cache;
pub mod enrichment;
pub mod exclusion;
/// The SearXNG federation client, extracted into the leaf crate [`xustive_federation`] so the
/// gateway binary can depend on it without pulling in the crawler's native deps (OCR/leptonica).
/// Re-exported here as `xustive_ingest::federation` so existing call sites keep resolving.
pub use xustive_federation as federation;
pub mod fetch;
pub mod fingerprint;
pub mod frontier;
pub mod gazetteer;
pub mod interaction;
pub mod link_graph;
pub mod media_embed;
pub mod media_ocr;
pub mod orchestrator;
pub mod pagerank;
pub mod parse;
pub mod proxy;
pub mod raw_store;
pub mod revisit;
pub mod robots;
pub mod robots_cache;
pub mod rules;
pub mod serp;
pub mod session;
pub mod simhash_index;
pub mod sitemap;
pub mod sitemap_poll;
pub mod spam;
pub mod topics;
pub mod weak_coverage;

pub use fetch::{FetchConfig, FetchError, Fetched, Fetcher};
pub use parse::{ParseConfig, ParseError, Parsed, Parser};
pub use robots::{Politeness, Robots};
