//! Search backend integration: the Meilisearch client, index settings, and filter construction.

pub mod client;
pub mod filter;
pub mod rank;
pub mod settings;

pub use client::{Hits, MeiliClient, Query, SearchError, TaskStatus};
pub use filter::Filters;
pub use rank::{rerank, Intent, Ranked, Weights};
pub use settings::{COMMENTS, DOCUMENTS, SOURCES};
