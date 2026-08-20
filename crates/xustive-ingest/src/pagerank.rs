//! Domain-level PageRank over the crawl link graph.
//!
//! The hand-curated authority list ([[authority]] in `xustive-search`) is a prior — a guess at which
//! domains people mean. This is the *earned* signal: a domain is authoritative to the degree that
//! other domains link to it. Computing it over domains rather than pages keeps the graph small
//! (thousands of nodes, not millions) and matches how ranking looks authority up — by domain.
//!
//! Two deliberate choices, both standard for web PageRank:
//! - **Cross-domain links only.** A link from a page to another page on the *same* site is
//!   navigation, not an endorsement, and counting it lets a big site vote for itself. [`LinkGraph`]
//!   drops self-edges at insertion.
//! - **Weighted edges.** The weight of A→B is how many of A's pages link to B, so one page linking to
//!   B a hundred times counts far less than a hundred pages each linking once — provided the caller
//!   de-duplicates targets per page before adding them, which is the cheap defence against a single
//!   page stuffing the graph.
//!
//! The result is the raw stationary distribution (sums to 1). Turning it into an authority value in a
//! comparable range is [`to_authority`], which the caller runs with the ranking constants.

use std::collections::{HashMap, HashSet};

/// Standard PageRank damping factor: the probability the random surfer follows a link rather than
/// teleporting. 0.85 is the canonical value.
pub const DEFAULT_DAMPING: f64 = 0.85;

/// A domain→domain link graph with edge weights.
#[derive(Debug, Default, Clone)]
pub struct LinkGraph {
    /// from → (to → weight).
    out: HashMap<String, HashMap<String, u32>>,
    nodes: HashSet<String>,
}

impl LinkGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `weight` links from domain `from` to domain `to`. Self-edges (same domain) are ignored
    /// — intra-site links are navigation, not endorsements. Both endpoints become nodes regardless,
    /// so a domain that is only ever linked *to* still gets a rank.
    pub fn add_edge(&mut self, from: &str, to: &str, weight: u32) {
        let from = from.trim().to_ascii_lowercase();
        let to = to.trim().to_ascii_lowercase();
        if from.is_empty() || to.is_empty() {
            return;
        }
        self.nodes.insert(from.clone());
        self.nodes.insert(to.clone());
        if from == to || weight == 0 {
            return;
        }
        *self.out.entry(from).or_default().entry(to).or_insert(0) += weight;
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.out.values().map(HashMap::len).sum()
    }

    /// Run weighted PageRank to convergence (or `max_iterations`), returning domain→score summing to
    /// ~1. `tolerance` is the L1 change below which iteration stops. Dangling nodes (no out-links)
    /// have their mass redistributed uniformly each step, so the distribution stays normalised.
    pub fn pagerank(
        &self,
        damping: f64,
        max_iterations: usize,
        tolerance: f64,
    ) -> HashMap<String, f64> {
        let n = self.nodes.len();
        if n == 0 {
            return HashMap::new();
        }
        let n_f = n as f64;
        let base = (1.0 - damping) / n_f;

        // Precompute each node's total out-weight, for normalising its outgoing contribution.
        let out_weight: HashMap<&str, f64> = self
            .out
            .iter()
            .map(|(from, tos)| (from.as_str(), tos.values().map(|w| *w as f64).sum()))
            .collect();

        let mut rank: HashMap<&str, f64> =
            self.nodes.iter().map(|d| (d.as_str(), 1.0 / n_f)).collect();

        for _ in 0..max_iterations {
            // Dangling mass: rank held by nodes with no outgoing edges, spread over everyone.
            let dangling: f64 = self
                .nodes
                .iter()
                .filter(|d| !self.out.contains_key(d.as_str()))
                .map(|d| rank[d.as_str()])
                .sum();
            let dangling_share = damping * dangling / n_f;

            let mut next: HashMap<&str, f64> = self
                .nodes
                .iter()
                .map(|d| (d.as_str(), base + dangling_share))
                .collect();

            for (from, tos) in &self.out {
                let ow = out_weight[from.as_str()];
                if ow <= 0.0 {
                    continue;
                }
                let share = damping * rank[from.as_str()] / ow;
                for (to, w) in tos {
                    *next.get_mut(to.as_str()).unwrap() += share * (*w as f64);
                }
            }

            let delta: f64 = self
                .nodes
                .iter()
                .map(|d| (next[d.as_str()] - rank[d.as_str()]).abs())
                .sum();
            rank = next;
            if delta < tolerance {
                break;
            }
        }

        rank.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }
}

/// Map raw PageRank scores into authority values, one per domain.
///
/// PageRank is a tiny long-tailed number; ranking wants something in a comparable band. Each score is
/// normalised by the maximum, softened with a square root (so the long tail is not crushed to zero),
/// and mapped into `[base, cap]` — where `base` is chosen per domain by `base_for` so the `.dz` home
/// floor is preserved (an earned Algerian domain never drops below the floor it would get unlisted).
pub fn to_authority(
    pr: &HashMap<String, f64>,
    cap: f32,
    base_for: impl Fn(&str) -> f32,
) -> HashMap<String, f32> {
    let max = pr.values().cloned().fold(0.0_f64, f64::max);
    if max <= 0.0 {
        return HashMap::new();
    }
    pr.iter()
        .map(|(domain, score)| {
            let normalized = (score / max).sqrt() as f32;
            let base = base_for(domain);
            let authority = (base + (cap - base) * normalized).clamp(base, cap);
            (domain.clone(), authority)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-6, "{a} != {b}");
    }

    #[test]
    fn scores_form_a_distribution_summing_to_one() {
        let mut g = LinkGraph::new();
        g.add_edge("a.com", "b.com", 1);
        g.add_edge("b.com", "c.com", 1);
        g.add_edge("c.com", "a.com", 1);
        let pr = g.pagerank(DEFAULT_DAMPING, 100, 1e-9);
        approx(pr.values().sum(), 1.0);
        // A symmetric 3-cycle: everyone equal.
        approx(pr["a.com"], 1.0 / 3.0);
        approx(pr["b.com"], 1.0 / 3.0);
        approx(pr["c.com"], 1.0 / 3.0);
    }

    #[test]
    fn a_domain_everyone_links_to_wins() {
        // b, c, d all link to hub; hub links back to none (dangling). Hub should rank highest.
        let mut g = LinkGraph::new();
        for src in ["b.com", "c.com", "d.com"] {
            g.add_edge(src, "hub.com", 1);
        }
        let pr = g.pagerank(DEFAULT_DAMPING, 100, 1e-9);
        approx(pr.values().sum(), 1.0);
        let hub = pr["hub.com"];
        assert!(
            pr.values().all(|&v| v <= hub + 1e-12),
            "hub must be the max"
        );
        assert!(
            hub > pr["b.com"],
            "the linked-to domain outranks its linkers"
        );
    }

    #[test]
    fn self_links_are_ignored() {
        let mut g = LinkGraph::new();
        g.add_edge("a.com", "a.com", 100); // navigation, must not count
        g.add_edge("a.com", "b.com", 1);
        assert_eq!(g.edge_count(), 1, "the self-edge must be dropped");
        let pr = g.pagerank(DEFAULT_DAMPING, 100, 1e-9);
        assert!(pr["b.com"] > pr["a.com"], "b is linked, a only self-links");
    }

    #[test]
    fn edge_weight_matters() {
        // hub1 gets many linking pages, hub2 gets one. hub1 should win.
        let mut g = LinkGraph::new();
        g.add_edge("src.com", "hub1.com", 50);
        g.add_edge("src.com", "hub2.com", 1);
        let pr = g.pagerank(DEFAULT_DAMPING, 100, 1e-9);
        assert!(pr["hub1.com"] > pr["hub2.com"]);
    }

    #[test]
    fn empty_graph_is_handled() {
        assert!(LinkGraph::new()
            .pagerank(DEFAULT_DAMPING, 10, 1e-9)
            .is_empty());
    }

    #[test]
    fn authority_mapping_respects_base_and_cap() {
        let mut pr = HashMap::new();
        pr.insert("big.com".to_string(), 0.9_f64);
        pr.insert("small.dz".to_string(), 0.0001_f64);
        // .dz floor 0.62, others 0.35, cap 0.85.
        let auth = to_authority(&pr, 0.85, |d| if d.ends_with(".dz") { 0.62 } else { 0.35 });
        assert!(auth["big.com"] > 0.6 && auth["big.com"] <= 0.85);
        // The tiny-PR .dz domain must not fall below its home floor.
        assert!(
            auth["small.dz"] >= 0.62,
            "the .dz home floor must hold, got {}",
            auth["small.dz"]
        );
    }
}
