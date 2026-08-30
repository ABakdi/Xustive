//! Signal stores: small, Redis-backed, k-anonymous by construction.
//!
//! Split out of `xustive-ingest` in 2026-08-30 so the tool fetcher — which needs one of these and
//! nothing else from the crawl pipeline — stops linking the OCR stack. That dependency is why the
//! `toold` image failed to build for two weeks, which is why world-city weather stopped being
//! refreshed and `weather dubai` went quiet three hours after every restart.

pub mod weak_coverage;
