---
tags:
  - component
  - serving
  - ingestion
component-id: C30
binary: xustive-api
status: specified
updated: 2026-08-20
---
# Interaction Signals

> **ID** C30 · **Binaries** `xustive-api` (capture, ranking) + `xustive-cli` (re-crawl) · **Store** Redis (`interaction:` namespace) · **Upstream** [[Query Pipeline]] · **Downstream** [[Ranking and Relevance]], [[Crawler Orchestrator]], the operator dashboard · **Governed by** [[ADR-0015 - Anonymous Interaction Signals for Ranking]], [[ADR-0008 - No Query Logging]]

## 1. Purpose

Turn what people *do* with results — which they open, which queries return nothing worth opening — into an anonymous, aggregate signal that (a) re-ranks documents toward what searchers actually find relevant and (b) points re-crawl at what people actually look for. No identifier is ever attached to an interaction; the design is the k-anonymous-counter pattern of [[weak_coverage]], generalised.

The name is **interaction**, not `engagement` — `engagement` already means social like/comment/share counts on a `Document`.

## 2. Responsibilities

**In scope**
- Record **impressions** (a doc was shown for a query) server-side, from the result set the API already built.
- Record **clicks** (a doc was opened) via an opaque, single-search token, with no query text in the request.
- Maintain windowed, k-anonymous counters: per-`(query, doc)`, per-`doc`, per-`query`.
- Expose a **smoothed CTR** lookup the re-ranker consumes at query time.
- Deposit a capped **"warm this doc"** hint for the revisit scheduler, and a frequency-ranked query feed for discovery.
- Serve the operator dashboard (top queries, top categories, CTR leaders) — all k-anonymous.

**Out of scope**
- Any per-user, per-session, or per-IP record. There is no such column.
- Dwell time, scroll depth, mouse movement (a later milestone may add pogo-stick detection; not here).
- Position debiasing (named follow-up in [[ADR-0015 - Anonymous Interaction Signals for Ranking]]).
- Anything reaching a log, metric label, or span — this is a ranking input, never [[Observability]].

## 3. Interface

### 3.1 Store (mirrors `weak_coverage::WeakCoverage`)

```rust
/// All counters are bare Redis integers with a sliding TTL; k-anonymity is applied on read.
pub struct Interactions {
    client: redis::aio::ConnectionManager, // one shared connection, per ADR (queue pattern)
    namespace: String,                     // "interaction"
    k: u32,                                // k-anonymity floor (>= 20 multi-user; 1 single-user dev)
    window: Duration,                      // sliding retention
}

impl Interactions {
    pub async fn connect_in(url: &str, ns: &str, k: u32, window: Duration) -> Option<Self>;

    /// Record that `docs` were shown for `query` (normalised). One pipeline, best-effort.
    pub async fn impressions(&self, query: &str, docs: &[String]);
    /// Record one click for `(query, doc)`. Best-effort.
    pub async fn click(&self, query: &str, doc: &str);
    /// Record that `query` (normalised) was searched, with its coarse `category`.
    pub async fn query_seen(&self, query: &str, category: &str);

    /// Smoothed CTR for each candidate doc under `query`, for the re-ranker. Returns a value in
    /// [0,1] only where the (query,doc) impressions clear `k`; otherwise the doc's global CTR if it
    /// clears `k`; otherwise `None` (neutral — the ranker treats absent as the prior).
    pub async fn ctr_for(&self, query: &str, docs: &[String]) -> HashMap<String, f32>;

    /// Docs whose (query,doc) clicks are hot enough to warrant a freshness pull, capped per run.
    pub async fn hot_docs(&self, limit: usize) -> Vec<String>;
    /// Top queries by frequency (k-anonymous), for the dashboard and discovery prioritisation.
    pub async fn top_queries(&self, limit: usize) -> Vec<QueryStat>;
}

pub struct QueryStat { pub query: String, pub count: u32, pub category: String }
```

### 3.2 HTTP (serving plane)

```
POST /api/v1/interaction         # a click. Body below. 204 always (never reveals token validity).
  { "t": "<search token>", "d": "<doc id>" }   # no query, no position-as-identity, nothing else
```
Impressions are **not** an endpoint — they are recorded inside `GET /search` from the results it returns. The search response gains one field:
```rust
pub struct SearchResponse { /* … */ pub interaction_token: Option<String> }  // None when disabled
```
minted like `summary_token`: opaque `new_id()`, in-process `Mutex<HashMap<token, (query_hash, Instant)>>`, `TTL 120s`, single-use is **not** required (a page can log several clicks), swept on write.

### 3.3 Ranking hook

`rerank(...)` gains `interaction_of: &HashMap<String, f32>` (doc id → smoothed CTR), built per-request by `Interactions::ctr_for` over the candidate doc ids, exactly as `authority_of` is threaded today. New `Weights.interaction` term; see §4.3.

### 3.4 Re-crawl hook (ingestion plane reads Redis — never a call)

`xustive-cli`'s revisit pass reads `hot_docs()` and, for each, pulls its `Visit` forward (`interval_secs = 0`), the same mechanism `Visits::apply_sitemap` uses. The discovery pass reads `top_queries()` to order weak-term resolution by real frequency.

## 4. Internal Design

### 4.1 Redis keys (all `EXPIRE`'d to the window on every write)

| Key | Type | Meaning |
|---|---|---|
| `interaction:qd:{qhash}:{doc}:imp` | int | impressions of `doc` for query `qhash` |
| `interaction:qd:{qhash}:{doc}:clk` | int | clicks of `doc` for query `qhash` |
| `interaction:doc:{doc}:imp` / `:clk` | int | doc's global impressions / clicks |
| `interaction:q:{query}` | int | query frequency (k-anonymous surface) |
| `interaction:qc:{query}` | str | last seen coarse category for the query |
| `interaction:hot:{doc}` | int | click accumulation used to pick re-crawl targets |

`qhash = HMAC(normalised_query, stable_per_deploy_salt)` — the CTR signal only needs to match the *same* query at read time, so it never needs the text back; hashing keeps the query out of the (query,doc) keys. `interaction:q:{query}` (the dashboard/discovery feed) keeps the normalised text, exactly as [[weak_coverage]] does and under the same guards, because discovery needs the text to act on it.

### 4.2 k-anonymity and windowing

- `surfaceable(count, k) = count >= k.max(1)` — reused from [[weak_coverage]]. Applied in `ctr_for`, `hot_docs`, `top_queries`; **never** on write.
- Every `INCR` is pipelined with `EXPIRE key window_secs`, so a counter can never outlive the window and an interaction not repeated decays out. No counter is ever written without its expiry (the [[weak_coverage]] invariant).

### 4.3 The CTR ranking term (keeps relevance dominant)

Raw CTR is unusable at low counts (1 click / 1 impression = 1.0). Use the **Wilson lower bound** of the click rate at 95 % confidence with a light Bayesian prior, so a doc earns a high score only with both a high rate and enough impressions. Map the result into `[0,1]`; absent (below k) → the neutral prior, so an unseen doc is neither rewarded nor punished.

Weight and invariant: the additive side weights must stay **below the 20-position relevance gap (0.48)** — the invariant `default_weights_keep_relevance_dominant` guards it. Adding `interaction` therefore rebalances:

```
relevance 0.55  freshness 0.13  trust 0.07  authority 0.09  quality 0.06  interaction 0.08
→ side sum = 0.13+0.07+0.09+0.06+0.08 = 0.43  <  0.48   ✓  (spam_penalty 0.15, subtracted, unchanged)
```

`interaction` is a tie-breaker among documents that already match — it reorders neighbours, it cannot float an irrelevant doc up the page. Added to both the score and the `Explain` breakdown.

### 4.4 Impression / click / rank flow

1. `GET /search` builds the ranked page. If `[interaction] enabled`, it calls `impressions(query, doc_ids)` and `query_seen(query, category)` (best-effort, non-blocking), and mints an `interaction_token`.
2. The page renders. One small **delegated-listener** client component wraps the result list; on a click of a result link it reads the anchor's `data-doc` and `navigator.sendBeacon('/api/v1/interaction', {t, d})`. The anchor stays server-rendered; the `href` is still the real destination (no redirect, no `ping`) — the beacon is fire-and-forget and does not gate navigation.
3. `POST /interaction` resolves `t → qhash` in memory and `click(qhash, doc)`. Returns 204 regardless (a caller learns nothing about token validity).
4. Next time that query runs, `ctr_for` returns the doc's smoothed CTR and the re-ranker nudges it.

### 4.5 Category

The coarse category is derived from the query the same way the catalog is organised (news / government / education / health / science-tech / sport / culture / business / reference) via the existing detectors/lexicons, or `other`. It is a bounded, `&'static`-labelled set — safe as a metric-free dashboard facet.

## 5. Configuration (`[interaction]`)

| Key | Default | Notes |
|---|---|---|
| `enabled` | `false` | Off is off — no store connects. |
| `k_anonymity` | `20` | ADR-0008 floor. Single-user dev may set `1` (understood as "no anonymity, single operator"). |
| `window_days` | `90` | Sliding retention; CTR reflects the last quarter. |
| `weight` | `0.08` | The `interaction` ranking weight (see §4.3 rebalance). |
| `hot_click_floor` | `k` | Clicks before a doc is a re-crawl candidate. |

## 6. Failure modes

| Failure | Behaviour |
|---|---|
| Redis unreachable | Every counter op is best-effort; ranking falls back to no interaction term; search is unaffected. |
| Store disabled | No token minted, no field, no beacon target hit; identical to today. |
| Token expired / unknown | `/interaction` returns 204; nothing recorded. |
| Below k | `ctr_for`/`top_queries`/`hot_docs` omit the entry; the signal simply does not exist yet. |
| Click flooding one doc | Windowed + capped `hot_docs` per run + the bounded ranking weight prevent a single actor skewing the page. |

## 7. Tests

- `surfaceable` reused; CTR smoothing: 1/1 scores below 50/100 scores below 500/1000 (monotone in both rate and volume).
- Ranking invariant: with `interaction` added, `default_weights_keep_relevance_dominant` and `adjacent_candidates_are_reorderable_but_distant_ones_are_not` still pass.
- k-floor: a `(query,doc)` with impressions `< k` returns no CTR; at `= k` it appears.
- Windowing: a counter written once and not repeated is gone after the window (TTL asserted).
- Privacy: telemetry lint passes (no `query`/`token` fields logged); no interaction key contains an IP/session/user component (there is no code path that could add one).
- Two-plane: the search plane only writes Redis for the re-crawl hint; the crawler only reads it.

## 8. Related

[[ADR-0015 - Anonymous Interaction Signals for Ranking]] · [[ADR-0008 - No Query Logging]] · [[ADR-0001 - Two-Plane Architecture]] · [[Ranking and Relevance]] · [[Query Pipeline]] · [[Crawler Orchestrator]] · [[weak_coverage]] · [[Milestone 6 - Adaptive Ranking from Interaction Signals]]
