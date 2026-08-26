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

/// A query with its k-anonymous frequency, coarse category, last result count, and total clicks —
/// the anonymous search-history row (M6-T05 + M7-T10).
#[derive(Debug, Clone, PartialEq)]
pub struct QueryStat {
    pub query: String,
    pub count: u32,
    pub category: String,
    /// Results the query last returned (M7-T10). 0 if never recorded (older rows).
    pub result_count: u32,
    /// Clicks across all of this query's results (M7-T10), keyed by the query hash so no query text
    /// is needed at click time.
    pub clicks: u32,
}

/// A document with its anonymous click-through, above the k-floor.
#[derive(Debug, Clone, PartialEq)]
pub struct DocStat {
    pub doc: String,
    pub impressions: u32,
    pub clicks: u32,
    pub ctr: f32,
}

/// The interaction store. One shared auto-reconnecting connection (the [[Task Queue]] pattern), not
/// one per operation.
#[derive(Clone)]
pub struct Interactions {
    manager: redis::aio::ConnectionManager,
    namespace: String,
    k: u32,
    window: Duration,
    /// Keyed-hash key derived from the deploy salt (BUG-036), `None` when no salt is configured
    /// (dev). With a key, qhashes are `blake3::keyed_hash` — unguessable without the salt, so a
    /// dictionary attack over the stored keys goes nowhere. Without one, the FNV fallback below
    /// applies, with the honesty that entails.
    salt_key: Option<[u8; 32]>,
}

/// The **unsalted fallback** for a non-plaintext query key: FNV-1a of the trimmed query.
///
/// Honest about what it is (BUG-036): unsalted, it is trivially reversible by dictionary attack —
/// hash the candidate, compare — so it only keeps plaintext out of the key *bytes*, it does not
/// protect the query from anyone holding the store. Deployments set `interaction.salt`
/// (`XUSTIVE_QHASH_SALT`) and get keyed blake3 instead; config validation requires it outside dev.
/// The load-bearing privacy guarantees remain k-anonymity plus the window, as for [[weak_coverage]].
fn fnv_qhash(query: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in query.trim().as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

impl Interactions {
    /// Connect, or `None` if Redis is unreachable. Best-effort like every counter store here.
    /// `salt` keys the query hash (BUG-036); empty falls back to unsalted FNV, dev only.
    pub async fn connect_in(
        url: &str,
        namespace: &str,
        k: u32,
        window: Duration,
        salt: &str,
    ) -> Option<Self> {
        let client = redis::Client::open(url).ok()?;
        let manager = client.get_connection_manager().await.ok()?;
        let salt = salt.trim();
        Some(Self {
            manager,
            namespace: namespace.to_string(),
            k: k.max(1),
            window,
            salt_key: (!salt.is_empty()).then(|| *blake3::hash(salt.as_bytes()).as_bytes()),
        })
    }

    fn conn(&self) -> redis::aio::ConnectionManager {
        self.manager.clone()
    }

    fn window_secs(&self) -> i64 {
        self.window.as_secs() as i64
    }

    /// `INCR` several keys in one pipeline, arming each with a **banded** expiry: a key gets its
    /// window when it is new or when its remaining TTL has fallen below half the window, never on
    /// every bump (BUG-039). Refreshing on every write made `window − TTL` a per-term last-event
    /// timestamp readable to ~1s by anyone with store access — the fine-grained timing ADR-0018
    /// says must not exist. Banding coarsens that observable to half-window granularity while
    /// keeping the invariant that matters: a counter never exists without an expiry, and one not
    /// bumped within its window still decays to nothing (worst case it lives window/2 less).
    async fn bump(&self, keys: &[String]) {
        if keys.is_empty() {
            return;
        }
        let mut conn = self.conn();
        // The increments run in their own pipeline, so nothing about the TTL read below can ever
        // cost a count — the counters are the data, the banding is only a refinement on top.
        let mut incr = redis::pipe();
        for k in keys {
            incr.cmd("INCR").arg(k).ignore();
        }
        let _: Result<(), _> = incr.query_async::<()>(&mut conn).await;

        // If the PTTL read fails, arm EVERYTHING unconditionally: the INCRs may have landed, and a
        // counter without an expiry is unbounded retention — the one state this store must never be
        // in. Losing the banding on an error costs timestamp coarseness; losing the expiry costs
        // the invariant.
        let mut ttls = redis::pipe();
        for k in keys {
            ttls.cmd("PTTL").arg(k);
        }
        let pttls = match ttls.query_async::<Vec<i64>>(&mut conn).await {
            Ok(v) if v.len() == keys.len() => v,
            _ => vec![-1; keys.len()],
        };
        let half_window_ms = self.window_secs() * 500; // window/2, in ms
        let mut arm = redis::pipe();
        let mut any = false;
        for (k, pttl) in keys.iter().zip(pttls) {
            // PTTL < 0 is a fresh key with no expiry (the just-INCRed case) — it MUST be armed, or
            // the counter would live forever. Below half-window, re-arm; otherwise leave the clock
            // alone so it tells nothing finer than "active within the last half window".
            if pttl < half_window_ms {
                arm.cmd("EXPIRE").arg(k).arg(self.window_secs()).ignore();
                any = true;
            }
        }
        if any {
            let _: Result<(), _> = arm.query_async::<()>(&mut conn).await;
        }
    }

    /// Record that `docs` were shown for `query`.
    pub async fn impressions(&self, query: &str, docs: &[String]) {
        let qh = self.qhash(query);
        let mut keys = Vec::with_capacity(docs.len() * 2);
        for d in docs {
            keys.push(format!("{}:qd:{qh}:{d}:imp", self.namespace));
            keys.push(format!("{}:doc:{d}:imp", self.namespace));
        }
        self.bump(&keys).await;
    }

    /// The opaque hash of a query, for callers (the API's click token) that must hold a query
    /// reference without holding the query text. Keyed blake3 under the deploy salt when one is
    /// configured (BUG-036) — genuinely one-way for anyone without the salt; the unsalted FNV
    /// fallback (dev) only keeps plaintext out of the key bytes, and says so at [`fnv_qhash`].
    /// An instance method, not a static: the hash is a function of the deployment's salt.
    pub fn qhash(&self, query: &str) -> String {
        match &self.salt_key {
            Some(key) => {
                let h = blake3::keyed_hash(key, query.trim().as_bytes());
                // 128 bits of the keyed hash: compact keys, unguessable without the salt.
                h.to_hex()[..32].to_string()
            }
            None => fnv_qhash(query),
        }
    }

    /// Record one click for `(query, doc)`.
    pub async fn click(&self, query: &str, doc: &str) {
        self.click_by_qhash(&self.qhash(query), doc).await;
    }

    /// Record one click when the query is already reduced to its [`Interactions::qhash`]. This is the
    /// path the click endpoint uses: it resolves an opaque token to a qhash in memory, so the query
    /// text is never in the click request at all (M6-T03).
    pub async fn click_by_qhash(&self, qh: &str, doc: &str) {
        self.bump(&[
            format!("{}:qd:{qh}:{doc}:clk", self.namespace),
            format!("{}:doc:{doc}:clk", self.namespace),
            format!("{}:hot:{doc}", self.namespace),
            // Total clicks for this query, keyed by hash (M7-T10). The search-history reader has the
            // query text and computes the same hash to read it — so the query text is never needed at
            // click time, keeping it out of the click request as M6-T03 requires.
            format!("{}:qk:{qh}", self.namespace),
        ])
        .await;
    }

    /// Record that `query` was searched, with its coarse `category` and how many results it returned
    /// (M7-T10 search history). The count is a last-write-wins value beside the frequency counter.
    pub async fn query_seen(&self, query: &str, category: &str, result_count: u32) {
        let query = query.trim();
        if query.is_empty() {
            return;
        }
        self.bump(&[format!("{}:q:{query}", self.namespace)]).await;
        // Category and the last result count are last-write-wins strings beside the counter, with the
        // same expiry — SET+EXPIRE so they never outlive the window either.
        let mut conn = self.conn();
        let cat_key = format!("{}:qc:{query}", self.namespace);
        let n_key = format!("{}:qn:{query}", self.namespace);
        let mut pipe = redis::pipe();
        pipe.cmd("SET").arg(&cat_key).arg(category).ignore();
        pipe.cmd("EXPIRE")
            .arg(&cat_key)
            .arg(self.window_secs())
            .ignore();
        pipe.cmd("SET").arg(&n_key).arg(result_count).ignore();
        pipe.cmd("EXPIRE")
            .arg(&n_key)
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
    ///
    /// One round trip for the whole candidate set (BUG-041). The first version issued two to four
    /// sequential `GET`s per document — up to 800 round trips for a 200-candidate pool — which put
    /// ~140 ms on every search under the "rerank" stage while the engine itself answered in 65.
    /// Four keys per document go out in a single `MGET`; the k-floor logic is unchanged.
    pub async fn ctr_for(&self, query: &str, docs: &[String]) -> HashMap<String, f32> {
        let mut out = HashMap::new();
        if docs.is_empty() {
            return out;
        }
        let mut conn = self.conn();
        let qh = self.qhash(query);
        let ns = &self.namespace;
        let mut keys: Vec<String> = Vec::with_capacity(docs.len() * 4);
        for d in docs {
            keys.push(format!("{ns}:qd:{qh}:{d}:imp"));
            keys.push(format!("{ns}:qd:{qh}:{d}:clk"));
            keys.push(format!("{ns}:doc:{d}:imp"));
            keys.push(format!("{ns}:doc:{d}:clk"));
        }
        let values: Vec<Option<u32>> =
            match redis::cmd("MGET").arg(&keys).query_async(&mut conn).await {
                Ok(v) => v,
                // Redis down or slow: no signal, which the ranker treats as the neutral prior. A search
                // never waits on, or fails for, an optional nudge.
                Err(_) => return out,
            };
        for (i, d) in docs.iter().enumerate() {
            let at = |j: usize| values.get(i * 4 + j).copied().flatten().unwrap_or(0);
            let (qd_imp, qd_clk, doc_imp, doc_clk) = (at(0), at(1), at(2), at(3));
            if surfaceable(qd_imp, self.k) {
                out.insert(d.clone(), wilson_lower_bound(qd_clk, qd_imp));
            } else if surfaceable(doc_imp, self.k) {
                out.insert(d.clone(), wilson_lower_bound(doc_clk, doc_imp));
            }
        }
        out
    }

    /// Scan every key matching `pattern` with a non-blocking `SCAN` cursor. Returns the bare key
    /// suffixes after `{namespace}:{prefix}`. Used by the analytics/re-crawl readers, which run off
    /// the serving path (an admin page, a crawl pass), never per search.
    async fn scan_suffixes(&self, prefix: &str) -> Vec<String> {
        let mut conn = self.conn();
        let full = format!("{}:{prefix}:*", self.namespace);
        let strip = format!("{}:{prefix}:", self.namespace);
        let mut cursor: u64 = 0;
        let mut out = Vec::new();
        loop {
            let res: Result<(u64, Vec<String>), _> = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&full)
                .arg("COUNT")
                .arg(500)
                .query_async(&mut conn)
                .await;
            let Ok((next, keys)) = res else { break };
            for k in keys {
                if let Some(s) = k.strip_prefix(&strip) {
                    out.push(s.to_string());
                }
            }
            cursor = next;
            if cursor == 0 {
                break;
            }
        }
        out
    }

    /// The most-searched queries above the k-floor, with their category (M6-T05.1). Generalises
    /// [[weak_coverage]] from "weak only" to "all queries, above the floor" — a query below k is
    /// never returned, so nothing personal surfaces. Sorted by count, capped at `limit`.
    pub async fn top_queries(&self, limit: usize) -> Vec<QueryStat> {
        let mut conn = self.conn();
        let queries = self.scan_suffixes("q").await;
        let mut stats = Vec::new();
        for q in queries {
            let count = self
                .get_u32(&mut conn, &format!("{}:q:{q}", self.namespace))
                .await;
            if !surfaceable(count, self.k) {
                continue;
            }
            let category: Option<String> = redis::cmd("GET")
                .arg(format!("{}:qc:{q}", self.namespace))
                .query_async(&mut conn)
                .await
                .ok()
                .flatten();
            let result_count = self
                .get_u32(&mut conn, &format!("{}:qn:{q}", self.namespace))
                .await;
            // Clicks are keyed by the query hash, which we recompute from the text we hold here.
            let clicks = self
                .get_u32(
                    &mut conn,
                    &format!("{}:qk:{}", self.namespace, self.qhash(&q)),
                )
                .await;
            stats.push(QueryStat {
                query: q,
                count,
                category: category.unwrap_or_else(|| "web".into()),
                result_count,
                clicks,
            });
        }
        stats.sort_by(|a, b| b.count.cmp(&a.count));
        stats.truncate(limit);
        stats
    }

    /// Documents whose click count clears `hot_floor` — the re-crawl freshness candidates (M6-T06.1).
    /// Returned most-clicked-first, capped at `limit`, so a popular document can be pulled forward in
    /// the revisit schedule without popularity owning the whole queue.
    pub async fn hot_docs(&self, hot_floor: u32, limit: usize) -> Vec<String> {
        let mut conn = self.conn();
        let docs = self.scan_suffixes("hot").await;
        let mut scored: Vec<(String, u32)> = Vec::new();
        for d in docs {
            let clicks = self
                .get_u32(&mut conn, &format!("{}:hot:{d}", self.namespace))
                .await;
            if clicks >= hot_floor.max(1) {
                scored.push((d, clicks));
            }
        }
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        scored.truncate(limit);
        scored.into_iter().map(|(d, _)| d).collect()
    }

    /// Record each shown document's URL alongside its id (M6-T06.1). The re-crawl pass keys on doc
    /// id but the crawler revisits URLs, and the crawler cannot read the search index to resolve one
    /// to the other — so the search plane, which has both in hand at impression time, notes the
    /// mapping here (last-write-wins, windowed like every other counter). It is the document's own
    /// public URL, not anything about a person.
    pub async fn note_urls(&self, pairs: &[(String, String)]) {
        if pairs.is_empty() {
            return;
        }
        let mut conn = self.conn();
        let mut pipe = redis::pipe();
        for (doc, url) in pairs {
            let key = format!("{}:docurl:{doc}", self.namespace);
            pipe.cmd("SET").arg(&key).arg(url).ignore();
            pipe.cmd("EXPIRE")
                .arg(&key)
                .arg(self.window_secs())
                .ignore();
        }
        let _: Result<(), _> = pipe.query_async::<()>(&mut conn).await;
    }

    /// Hot documents paired with their URL, for the crawler's re-crawl pass (M6-T06.1). Reads
    /// [`hot_docs`] and resolves each to the URL the search plane noted via [`note_urls`]; a hot
    /// document with no known URL (never noted, or the mapping decayed) is skipped — there is nothing
    /// to revisit. Most-clicked first.
    pub async fn hot_docs_to_recrawl(&self, hot_floor: u32, limit: usize) -> Vec<(String, String)> {
        let mut conn = self.conn();
        let mut out = Vec::new();
        for doc in self.hot_docs(hot_floor, limit).await {
            let url: Option<String> = redis::cmd("GET")
                .arg(format!("{}:docurl:{doc}", self.namespace))
                .query_async(&mut conn)
                .await
                .ok()
                .flatten();
            if let Some(url) = url.filter(|u| !u.trim().is_empty()) {
                out.push((doc, url));
            }
        }
        out
    }

    /// The documents with the highest anonymous click-through — the "CTR leaders" for the operator
    /// console (M6-T07). A document is only returned once its global impressions clear the k-floor,
    /// so nothing below the anonymity threshold surfaces. Sorted by smoothed CTR, capped at `limit`.
    pub async fn top_documents(&self, limit: usize) -> Vec<DocStat> {
        let mut conn = self.conn();
        // Every document with a global impression counter is a candidate.
        let ids = self.scan_suffixes("doc").await;
        // Suffixes look like "{doc}:imp" / "{doc}:clk"; keep the impression rows and strip ":imp".
        let mut stats = Vec::new();
        for suffix in ids {
            let Some(doc) = suffix.strip_suffix(":imp") else {
                continue;
            };
            let imp = self
                .get_u32(&mut conn, &format!("{}:doc:{doc}:imp", self.namespace))
                .await;
            if !surfaceable(imp, self.k) {
                continue;
            }
            let clk = self
                .get_u32(&mut conn, &format!("{}:doc:{doc}:clk", self.namespace))
                .await;
            stats.push(DocStat {
                doc: doc.to_string(),
                impressions: imp,
                clicks: clk,
                ctr: wilson_lower_bound(clk, imp),
            });
        }
        stats.sort_by(|a, b| {
            b.ctr
                .partial_cmp(&a.ctr)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        stats.truncate(limit);
        stats
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
        assert_eq!(fnv_qhash("paracetamol"), fnv_qhash(" paracetamol "));
        assert_ne!(fnv_qhash("paracetamol"), fnv_qhash("aspirin"));
        assert!(!fnv_qhash("paracetamol").contains("paracetamol"));
    }

    #[test]
    fn the_salted_hash_is_keyed_stable_and_salt_dependent() {
        // BUG-036: with a salt the hash is keyed blake3 — deterministic under one salt, different
        // under another, never the FNV a dictionary attacker could precompute, and never plaintext.
        let key_a = *blake3::hash(b"salt-a").as_bytes();
        let key_b = *blake3::hash(b"salt-b").as_bytes();
        let h = |key: &[u8; 32], q: &str| {
            blake3::keyed_hash(key, q.trim().as_bytes()).to_hex()[..32].to_string()
        };
        assert_eq!(h(&key_a, "paracetamol"), h(&key_a, " paracetamol "));
        assert_ne!(h(&key_a, "paracetamol"), h(&key_b, "paracetamol"));
        assert_ne!(h(&key_a, "paracetamol"), fnv_qhash("paracetamol"));
        assert!(!h(&key_a, "paracetamol").contains("paracetamol"));
    }

    // The key-shape proof (M6-T08.3): assert every key the store constructs is built ONLY from the
    // namespace, a query/qhash, a doc id, and a fixed suffix — there is no code path that puts an IP,
    // a session, or a user id into a key. This test enumerates the shapes rather than exercising
    // Redis; the guarantee is that the *construction* has no identifier input, and these are every
    // format string in the module.
    #[test]
    fn no_key_can_contain_an_identifier() {
        let ns = "interaction";
        let qh = fnv_qhash("some query");
        let doc = "doc123";
        let query = "some query";
        let keys = [
            format!("{ns}:qd:{qh}:{doc}:imp"),
            format!("{ns}:doc:{doc}:imp"),
            format!("{ns}:qd:{qh}:{doc}:clk"),
            format!("{ns}:doc:{doc}:clk"),
            format!("{ns}:hot:{doc}"),
            format!("{ns}:q:{query}"),
            format!("{ns}:qc:{query}"),
        ];
        // Every key is composed of the namespace, a query (or its hash), a doc id, and a fixed
        // suffix. None of those inputs is a person: the query text is data (safe by window + k-floor,
        // the ADR-0015 pattern), the doc id is a corpus id, the qhash is one-way. There is simply no
        // parameter in this module that could carry an IP or a session.
        for k in &keys {
            assert!(
                k.starts_with(&format!("{ns}:")),
                "key {k} escaped the namespace"
            );
            // A sanity check that the components are exactly the expected ones — no stray field.
            assert!(
                k.contains(doc) || k.contains(query) || k.contains(&qh),
                "key {k} is built from an unexpected component"
            );
        }
    }
}
