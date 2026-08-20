//! Redis storage for the domain link graph and the authority scores computed from it.
//!
//! The crawl writes edges here as it goes ([`LinkGraphStore::record`]); `xustive-cli pagerank` reads
//! the whole graph back ([`load_graph`](LinkGraphStore::load_graph)), runs [`crate::pagerank`], and
//! writes the resulting per-domain authority ([`store_authority`](LinkGraphStore::store_authority));
//! the API reads that at startup and blends it with the curated prior.
//!
//! Best-effort throughout, exactly like [`crate::crawl_stats`]: a link-graph write that fails is a
//! slightly-less-informed PageRank next time, never a lost document or a stalled crawl.
//!
//! Keys (all under `linkgraph:` / `pagerank:`):
//! - `linkgraph:sources`      SET of domains that have outgoing edges.
//! - `linkgraph:out:<from>`   HASH of `to → count` (how many of `from`'s pages linked to `to`).
//! - `pagerank:authority`     HASH of `domain → authority` (0–1), the computed output.

use std::collections::HashMap;

use crate::pagerank::LinkGraph;

const K_SOURCES: &str = "linkgraph:sources";
const K_OUT_PREFIX: &str = "linkgraph:out:";
const K_AUTHORITY: &str = "pagerank:authority";

#[derive(Clone)]
pub struct LinkGraphStore {
    client: redis::Client,
}

/// A domain key that matches `xustive_core::domain_of`: host, lowercased, `www.` stripped.
fn norm(host: &str) -> String {
    host.trim().trim_start_matches("www.").to_ascii_lowercase()
}

impl LinkGraphStore {
    pub fn connect(url: &str) -> Option<Self> {
        redis::Client::open(url).ok().map(|client| Self { client })
    }

    async fn conn(&self) -> Option<redis::aio::MultiplexedConnection> {
        self.client.get_multiplexed_async_connection().await.ok()
    }

    /// Record the cross-domain links from one page: `from` → each distinct target domain. Same-domain
    /// targets and duplicates are dropped here, so one page counts as at most one vote per target —
    /// the cheap defence against a single link-stuffed page skewing the graph.
    pub async fn record(&self, from_host: &str, target_hosts: &[String]) {
        let from = norm(from_host);
        if from.is_empty() {
            return;
        }
        let mut targets: Vec<String> = target_hosts
            .iter()
            .map(|h| norm(h))
            .filter(|t| !t.is_empty() && *t != from)
            .collect();
        targets.sort();
        targets.dedup();
        if targets.is_empty() {
            return;
        }
        let Some(mut c) = self.conn().await else {
            return;
        };
        let mut pipe = redis::pipe();
        pipe.cmd("SADD").arg(K_SOURCES).arg(&from).ignore();
        let out_key = format!("{K_OUT_PREFIX}{from}");
        for t in &targets {
            pipe.cmd("HINCRBY").arg(&out_key).arg(t).arg(1).ignore();
        }
        let _: Result<(), _> = pipe.query_async::<()>(&mut c).await;
    }

    /// Read the entire graph back into memory for a PageRank run.
    pub async fn load_graph(&self) -> LinkGraph {
        let mut graph = LinkGraph::new();
        let Some(mut c) = self.conn().await else {
            return graph;
        };
        let sources: Vec<String> = redis::cmd("SMEMBERS")
            .arg(K_SOURCES)
            .query_async(&mut c)
            .await
            .unwrap_or_default();
        for from in sources {
            let out: Vec<(String, u32)> = redis::cmd("HGETALL")
                .arg(format!("{K_OUT_PREFIX}{from}"))
                .query_async(&mut c)
                .await
                .unwrap_or_default();
            for (to, weight) in out {
                graph.add_edge(&from, &to, weight);
            }
        }
        graph
    }

    /// Replace the stored authority scores with a freshly computed set.
    pub async fn store_authority(&self, scores: &HashMap<String, f32>) {
        let Some(mut c) = self.conn().await else {
            return;
        };
        let mut pipe = redis::pipe();
        pipe.cmd("DEL").arg(K_AUTHORITY).ignore();
        for (domain, score) in scores {
            pipe.cmd("HSET")
                .arg(K_AUTHORITY)
                .arg(domain)
                .arg(score.to_string())
                .ignore();
        }
        let _: Result<(), _> = pipe.query_async::<()>(&mut c).await;
    }

    /// Blocking read of the computed authority scores, for synchronous startup paths (the API builds
    /// its state before the async runtime is doing anything else). Empty when PageRank has never run
    /// or Redis is unreachable — both mean "fall back to the curated prior", not an error.
    pub fn load_authority_blocking(&self) -> HashMap<String, f32> {
        let Ok(mut c) = self.client.get_connection() else {
            return HashMap::new();
        };
        let pairs: Vec<(String, String)> = redis::cmd("HGETALL")
            .arg(K_AUTHORITY)
            .query(&mut c)
            .unwrap_or_default();
        pairs
            .into_iter()
            .filter_map(|(d, s)| s.parse::<f32>().ok().map(|v| (d, v)))
            .collect()
    }

    /// Read the computed authority scores (domain → 0–1). Empty when PageRank has never run.
    pub async fn load_authority(&self) -> HashMap<String, f32> {
        let Some(mut c) = self.conn().await else {
            return HashMap::new();
        };
        let pairs: Vec<(String, String)> = redis::cmd("HGETALL")
            .arg(K_AUTHORITY)
            .query_async(&mut c)
            .await
            .unwrap_or_default();
        pairs
            .into_iter()
            .filter_map(|(d, s)| s.parse::<f32>().ok().map(|v| (d, v)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_keys_are_normalised_like_domain_of() {
        assert_eq!(norm("www.ElKhabar.com"), "elkhabar.com");
        assert_eq!(norm("EN.Wikipedia.org"), "en.wikipedia.org");
        assert_eq!(norm("  aps.dz "), "aps.dz");
    }
}
