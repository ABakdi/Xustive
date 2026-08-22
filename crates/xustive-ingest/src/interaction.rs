//! Anonymous interaction signals ([[ADR-0015]], [[Interaction Signals]]).
//!
//! Impressions and clicks as k-anonymous, windowed Redis counters — never tied to a person — that
//! ranking and re-crawl learn from. This is the [[weak_coverage]] pattern generalised: a bare integer
//! counter, keyed structurally, surfaced only above a k-floor, decaying out of a sliding window. There
//! is no identifier anywhere in this module — no IP, no session, no user — because there is no field
//! that could hold one.
//!
//! **The word `engagement` is taken** (social like/comment/share counts on a `Document`); this signal
//! is called *interaction* throughout.
//!
//! Off unless enabled by config. When on, a `(query, doc)` or per-query signal is *used* only above
//! the k floor (`surfaceable`), and every write pairs its `INCR` with an `EXPIRE`, so a counter can
//! never outlive the window and an interaction not repeated decays to nothing.

use std::collections::HashMap;
use std::time::Duration;

/// A query-derived signal is used only once this many distinct searches have contributed. Shared with
/// [[weak_coverage]]; the guarantee is structural — the counter exists below the floor, it is just
/// never read out.
pub fn surfaceable(count: u32, k: u32) -> bool {
    count >= k.max(1)
}

/// Wilson score lower bound of the click-through rate at ~95 % confidence.
///
/// Raw CTR is unusable at low volume — one click on one impression is 1.0. The Wilson lower bound
/// rewards a document only when it has *both* a high rate and enough impressions to trust it, and it
/// grows toward the raw rate as volume grows. Returns a value in `[0, 1]`.
pub fn wilson_lower_bound(clicks: u32, impressions: u32) -> f32 {
    if impressions == 0 {
        return 0.0;
    }
    let n = impressions as f64;
    let p = clicks as f64 / n;
    // z for 95 % confidence.
    const Z: f64 = 1.959_963_984_540_054;
    let z2 = Z * Z;
    let denom = 1.0 + z2 / n;
    let centre = p + z2 / (2.0 * n);
    let margin = Z * ((p * (1.0 - p) + z2 / (4.0 * n)) / n).sqrt();
    (((centre - margin) / denom).clamp(0.0, 1.0)) as f32
}

/// A query with its k-anonymous frequency and coarse category.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryStat {
    pub query: String,
    pub count: u32,
    pub category: String,
}

/// The interaction store. One shared auto-reconnecting connection (the [[Task Queue]] pattern), not
/// one per operation.
#[derive(Clone)]
pub struct Interactions {
    manager: redis::aio::ConnectionManager,
    namespace: String,
    k: u32,
    window: Duration,
}

/// A stable, non-plaintext key for a normalised query.
///
/// FNV-1a here keeps the query text out of the `(query, doc)` counter keys — a scaffold stand-in for
/// the HMAC-with-a-deploy-salt the spec calls for. The real privacy guarantee is k-anonymity plus the
/// window, exactly as for [[weak_coverage]]; the hash only avoids storing the plaintext in these keys.
fn qhash(query: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in query.trim().as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

impl Interactions {
    /// Connect, or `None` if Redis is unreachable. Best-effort like every counter store here.
    pub async fn connect_in(url: &str, namespace: &str, k: u32, window: Duration) -> Option<Self> {
        let client = redis::Client::open(url).ok()?;
        let manager = client.get_connection_manager().await.ok()?;
        Some(Self {
            manager,
            namespace: namespace.to_string(),
            k: k.max(1),
            window,
        })
    }

    fn conn(&self) -> redis::aio::ConnectionManager {
        self.manager.clone()
    }

    fn window_secs(&self) -> i64 {
        self.window.as_secs() as i64
    }

    /// `INCR key; EXPIRE key window` for several keys in one pipeline — the counter never outlives the
    /// window, and never exists without an expiry (the [[weak_coverage]] invariant).
    async fn bump(&self, keys: &[String]) {
        if keys.is_empty() {
            return;
        }
        let mut conn = self.conn();
        let mut pipe = redis::pipe();
        for k in keys {
            pipe.cmd("INCR").arg(k).ignore();
            pipe.cmd("EXPIRE").arg(k).arg(self.window_secs()).ignore();
        }
        let _: Result<(), _> = pipe.query_async::<()>(&mut conn).await;
    }

    /// Record that `docs` were shown for `query`.
    pub async fn impressions(&self, query: &str, docs: &[String]) {
        let qh = qhash(query);
        let mut keys = Vec::with_capacity(docs.len() * 2);
        for d in docs {
            keys.push(format!("{}:qd:{qh}:{d}:imp", self.namespace));
            keys.push(format!("{}:doc:{d}:imp", self.namespace));
        }
        self.bump(&keys).await;
    }

    /// The opaque hash of a query, for callers (the API's click token) that must hold a query
    /// reference without holding the query text. FNV-1a of the trimmed query — one-way and
    /// fixed-width, so it can never be reversed to the words someone typed.
    pub fn qhash(query: &str) -> String {
        qhash(query)
    }

    /// Record one click for `(query, doc)`.
    pub async fn click(&self, query: &str, doc: &str) {
        self.click_by_qhash(&qhash(query), doc).await;
    }

    /// Record one click when the query is already reduced to its [`Interactions::qhash`]. This is the
    /// path the click endpoint uses: it resolves an opaque token to a qhash in memory, so the query
    /// text is never in the click request at all (M6-T03).
    pub async fn click_by_qhash(&self, qh: &str, doc: &str) {
        self.bump(&[
            format!("{}:qd:{qh}:{doc}:clk", self.namespace),
            format!("{}:doc:{doc}:clk", self.namespace),
            format!("{}:hot:{doc}", self.namespace),
        ])
        .await;
    }

    /// Record that `query` was searched, with its coarse `category`.
    pub async fn query_seen(&self, query: &str, category: &str) {
        let query = query.trim();
        if query.is_empty() {
            return;
        }
        self.bump(&[format!("{}:q:{query}", self.namespace)]).await;
        // Category is a last-write-wins string beside the counter, with the same expiry.
        let mut conn = self.conn();
        let key = format!("{}:qc:{query}", self.namespace);
        let mut pipe = redis::pipe();
        pipe.cmd("SET").arg(&key).arg(category).ignore();
        pipe.cmd("EXPIRE")
            .arg(&key)
            .arg(self.window_secs())
            .ignore();
        let _: Result<(), _> = pipe.query_async::<()>(&mut conn).await;
    }

    async fn get_u32(&self, conn: &mut redis::aio::ConnectionManager, key: &str) -> u32 {
        let v: Option<u32> = redis::cmd("GET")
            .arg(key)
            .query_async(conn)
            .await
            .ok()
            .flatten();
        v.unwrap_or(0)
    }

    /// Smoothed CTR for each candidate doc under `query`. A `(query, doc)` signal is used only above
    /// the k floor; below it, the doc's global CTR (if *that* clears k); otherwise the doc is absent
    /// and the ranker treats it as the neutral prior.
    pub async fn ctr_for(&self, query: &str, docs: &[String]) -> HashMap<String, f32> {
        let mut out = HashMap::new();
        let mut conn = self.conn();
        let qh = qhash(query);
        for d in docs {
            let qd_imp = self
                .get_u32(&mut conn, &format!("{}:qd:{qh}:{d}:imp", self.namespace))
                .await;
            if surfaceable(qd_imp, self.k) {
                let qd_clk = self
                    .get_u32(&mut conn, &format!("{}:qd:{qh}:{d}:clk", self.namespace))
                    .await;
                out.insert(d.clone(), wilson_lower_bound(qd_clk, qd_imp));
                continue;
            }
            let doc_imp = self
                .get_u32(&mut conn, &format!("{}:doc:{d}:imp", self.namespace))
                .await;
            if surfaceable(doc_imp, self.k) {
                let doc_clk = self
                    .get_u32(&mut conn, &format!("{}:doc:{d}:clk", self.namespace))
                    .await;
                out.insert(d.clone(), wilson_lower_bound(doc_clk, doc_imp));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_k_floor_is_at_least_one() {
        assert!(!surfaceable(0, 20));
        assert!(!surfaceable(19, 20));
        assert!(surfaceable(20, 20));
        assert!(surfaceable(1, 0)); // k clamped to 1
    }

    #[test]
    fn wilson_rewards_both_rate_and_volume() {
        // A perfect rate on tiny volume must score below a good rate on large volume.
        let one_of_one = wilson_lower_bound(1, 1);
        let fifty_of_hundred = wilson_lower_bound(50, 100);
        let five_hundred_of_thousand = wilson_lower_bound(500, 1000);
        assert!(
            one_of_one < fifty_of_hundred,
            "{one_of_one} !< {fifty_of_hundred}"
        );
        assert!(
            fifty_of_hundred < five_hundred_of_thousand,
            "same rate, more volume must score higher"
        );
        // Bounded to [0,1], and zero impressions is zero.
        assert_eq!(wilson_lower_bound(0, 0), 0.0);
        assert!((0.0..=1.0).contains(&wilson_lower_bound(1000, 1000)));
    }

    #[test]
    fn no_clicks_is_a_low_but_defined_score() {
        // Impressions but no clicks: a real, low, non-negative score, not a panic or a NaN.
        let s = wilson_lower_bound(0, 100);
        assert!(s >= 0.0 && s < 0.05, "got {s}");
    }

    #[test]
    fn the_query_hash_is_stable_and_hides_the_text() {
        assert_eq!(qhash("paracetamol"), qhash(" paracetamol "));
        assert_ne!(qhash("paracetamol"), qhash("aspirin"));
        assert!(!qhash("paracetamol").contains("paracetamol"));
    }
}
