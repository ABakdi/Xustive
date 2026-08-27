---
tags:
  - component
  - serving
  - ingestion
component-id: C30
binary: xustive-api
status: built
updated: 2026-08-27
---
# Interaction Signals

> **ID** C30 · **Binaries** `xustive-api` (capture, ranking, admin view) + `xustive-cli crawld`
> (re-crawl) · **Store** Redis (`interaction:` namespace, the behavioural-signals Redis) ·
> **Upstream** [[Query Pipeline]] · **Downstream** [[Ranking and Relevance]],
> [[Crawler Orchestrator]], the admin *Search history* page · **Governed by**
> [[ADR-0015 - Anonymous Interaction Signals for Ranking]], [[ADR-0018 - Anonymous Search History]],
> [[ADR-0008 - No Query Logging]]

Built in [[Milestone 6 - Adaptive Ranking from Interaction Signals]] (M6-T02…T06), extended for
search history in M7-T10, and hardened by the post-M7 audit (BUG-024, BUG-036, BUG-039, BUG-041).
Off by default; `config/dev.toml` turns it on with `k_anonymity = 1` for the single-operator box.

## 1. Purpose

Turn what people *do* with results — which they open, which queries return nothing worth opening —
into an anonymous, aggregate signal that (a) re-ranks documents toward what searchers actually find
relevant and (b) points re-crawl at what people actually look for. No identifier is ever attached
to an interaction; the design is the k-anonymous-counter pattern of [[Interaction Signals|weak_coverage]], generalised.

The name is **interaction**, not `engagement` — `engagement` already means social
like/comment/share/view counts on a `Document` (`xustive_core::model::Engagement`).

## 2. Responsibilities

**In scope**
- Record **impressions** (a doc was shown for a query) server-side, from the page the API built.
- Record **clicks** (a doc was opened) via an opaque, per-search token, with no query text in the
  request.
- Record **searches** with their coarse category and result count (the anonymous search history
  of ADR-0018).
- Maintain windowed, k-anonymous counters: per-`(query, doc)`, per-`doc`, per-`query`.
- Expose a **smoothed CTR** lookup the re-ranker consumes at query time.
- Note each shown doc's URL so the crawler, which cannot read the index, can re-fetch **hot
  docs**; serve those to `crawld`'s re-crawl pass.
- Serve the admin *Search history* page (top queries, categories, CTR leaders, hot docs) — all
  k-anonymous.

**Out of scope**
- Any per-user, per-session, or per-IP record. There is no such column.
- Dwell time, scroll depth, mouse movement (pogo-stick detection is a possible later milestone).
- Position debiasing (named follow-up in ADR-0015; still not built 2026-08-27).
- A frequency-ranked query feed for *discovery* — the ADR named it, but nothing in
  `crates/xustive-cli` reads `top_queries` today (2026-08-27); only the admin page does.
- Anything reaching a log, metric label, or span — this is a ranking input, never
  [[Observability]]. `token` and `query` are forbidden telemetry field names.

## 3. Interface

### 3.1 Store — `xustive_ingest::interaction::Interactions`

```rust
/// All counters are bare Redis integers with a banded sliding TTL; k-anonymity is applied on read.
pub struct Interactions {
    manager: redis::aio::ConnectionManager, // one shared auto-reconnecting connection (Task Queue pattern)
    namespace: String,                      // "interaction"
    k: u32,                                 // k-anonymity floor, never below 1
    window: Duration,                       // sliding retention
    salt_key: Option<[u8; 32]>,             // blake3(salt); None = unsalted dev fallback (BUG-036)
}

impl Interactions {
    pub async fn connect_in(url, namespace, k, window, salt: &str) -> Option<Self>;

    pub async fn impressions(&self, query: &str, docs: &[String]);
    pub async fn click(&self, query: &str, doc: &str);
    pub async fn click_by_qhash(&self, qhash: &str, doc: &str);       // the click endpoint's path
    pub async fn query_seen(&self, query: &str, category: &str, result_count: u32);
    pub async fn note_urls(&self, docs: &[(String, String)]);          // (doc id, URL), for re-crawl
    pub fn qhash(&self, query: &str) -> String;                        // salted; an instance method

    /// Smoothed CTR per candidate. (query,doc) above k, else the doc's global CTR above k, else
    /// absent — the ranker treats absent as the neutral prior. One MGET for the whole pool.
    pub async fn ctr_for(&self, query: &str, docs: &[String]) -> HashMap<String, f32>;

    pub async fn hot_docs(&self, hot_floor: u32, limit: usize) -> Vec<String>;
    pub async fn hot_docs_to_recrawl(&self, hot_floor: u32, limit: usize) -> Vec<(String, String)>;
    pub async fn top_queries(&self, limit: usize) -> Vec<QueryStat>;
    pub async fn top_documents(&self, limit: usize) -> Vec<DocStat>;
}

pub struct QueryStat { query, count, category, result_count, clicks }
pub struct DocStat   { doc, impressions, clicks, ctr }
pub fn surfaceable(count: u32, k: u32) -> bool;              // shared with weak_coverage
pub fn wilson_lower_bound(clicks: u32, impressions: u32) -> f32;
```

### 3.2 HTTP (serving plane)

```
POST /api/v1/interaction         # a click. 204 always (never reveals token validity).
  { "t": "<interaction_token>", "d": "<doc id>" }   # no query, nothing else; unknown fields dropped
GET  /api/admin/interaction      # the search-history view; { "enabled": false } when off
```

Impressions are **not** an endpoint — they are recorded inside `GET /search` from the page it
returns. The search response carries `interaction_token: Option<String>` (`None` when disabled),
minted like `summary_token` in `search.rs::mint_interaction_token`: a fresh ULID held in
`AppState.interaction_tokens: RwLock<HashMap<token, (qhash, Instant)>>`, TTL 120 s, swept on every
mint, capped at `MAX_TOKENS = 4096` with oldest-first eviction (BUG-024). Not single-use — a page
can log several clicks.

### 3.3 Ranking hook

`rerank(...)` takes `interaction_of: &HashMap<String, f32>` (doc id → smoothed CTR), built per
request by `Interactions::ctr_for` over the fused candidate ids, threaded exactly like
`authority_of`. `Weights.interaction` is the term; see §4.3. The weight lives in the ranker's
`Weights` (`config/ranking.toml`), not in `[interaction]`.

### 3.4 Re-crawl hook (ingestion plane reads Redis — never a call)

`crawld` (`crates/xustive-cli/src/crawld.rs`) connects its own `Interactions` with the same
config and, every `HOT_RECRAWL_EVERY = 30 min`, reads `hot_docs_to_recrawl(hot_floor, 200)` and
`defer`s each URL into the frontier as a `Pending { source_id: "hot", depth: 1, trust: 60,
channel: Link }`. It is the frontier, not the `Visits` table, that carries the pull-forward.

## 4. Internal Design

### 4.1 Redis keys (all expire to the window)

| Key | Type | Meaning |
|---|---|---|
| `interaction:qd:{qhash}:{doc}:imp` / `:clk` | int | impressions / clicks of `doc` for query `qhash` |
| `interaction:doc:{doc}:imp` / `:clk` | int | doc's global impressions / clicks |
| `interaction:hot:{doc}` | int | click accumulation used to pick re-crawl targets |
| `interaction:docurl:{doc}` | str | the doc's public URL, noted at impression time (M6-T06.1) |
| `interaction:q:{query}` | int | query frequency (k-anonymous surface) |
| `interaction:qc:{query}` | str | last seen coarse category |
| `interaction:qn:{query}` | int | last result count (M7-T10, last-write-wins) |
| `interaction:qk:{qhash}` | int | total clicks across the query's results (M7-T10) |

`qhash` is **keyed blake3** of the normalised query under the deploy salt (`interaction.salt` /
`XUSTIVE_QHASH_SALT`, BUG-036) — one-way for anyone without the salt. With no salt it falls back to
unsalted FNV-1a, which only keeps plaintext out of the key bytes and is trivially reversible by
dictionary; config validation refuses an empty salt outside `dev`. `interaction:q:{query}` keeps
the normalised text, exactly as [[Interaction Signals|weak_coverage]] does and under the same guards, because the
admin view and any future discovery use need the text to act on.

### 4.2 k-anonymity and windowing

- `surfaceable(count, k) = count >= k.max(1)`. Applied in `ctr_for`, `hot_docs`, `top_queries`,
  `top_documents`; **never** on write.
- Expiry is **banded** (BUG-039): `bump` arms a key with the window when it is new or when its
  remaining TTL has fallen below half the window, never on every write. Refreshing on every
  write made `window − TTL` a per-term last-event timestamp readable to ~1 s by anyone with store
  access — the fine-grained timing ADR-0018 forbids. A counter still never exists without an
  expiry, and one not bumped decays out (worst case window/2 sooner).

### 4.3 The CTR ranking term (keeps relevance dominant)

Raw CTR is unusable at low counts (1 click / 1 impression = 1.0). `wilson_lower_bound` is the
Wilson score lower bound at 95 % (z = 1.96) with no extra prior — it already rewards a doc only
with both a high rate and enough impressions, and 0 impressions scores 0. Absent (below k) → the
ranker treats the doc as neutral.

Weight and invariant: the additive side weights must stay **below the 20-position relevance gap
(0.48)** — `default_weights_keep_relevance_dominant` guards it. Interaction entered at 0.08 and
was rebalanced to **0.07** when [[ADR-0026 - The Reader's Language as a Bounded Ranking Signal]]
added `ui_language`:

```
relevance 0.55  freshness 0.10  trust 0.06  authority 0.09  quality 0.05
interaction 0.07  ui_language 0.10   → side sum 0.47 < 0.48 ✓   (spam_penalty 0.15 subtracted)
```

`interaction` is a tie-breaker among documents that already match — it reorders neighbours, it
cannot float an irrelevant doc up the page. It appears in the score and the `Explain` breakdown.

### 4.4 Impression / click / rank flow

1. `GET /search` fuses candidates, calls `ctr_for` over their ids, and re-ranks. After the page
   is built it calls `impressions`, `note_urls`, `query_seen(query, category, result_count)`
   (all best-effort) and mints `interaction_token`.
2. The page renders. `web/components/search/InteractionBeacon.tsx` wraps the result list with one
   delegated listener; a click on a result link reads the anchor's `data-doc`
   (`ResultCard.tsx`) and fires `navigator.sendBeacon('/api/v1/interaction', {t, d})`. The `href`
   is still the real destination — no redirect, no `ping`; the beacon never gates navigation, and
   without JavaScript the link simply works and nothing is recorded.
3. `POST /interaction` (`crates/xustive-api/src/interaction.rs`) resolves `t → qhash` in memory
   and calls `click_by_qhash`. 204 regardless, even for a missing or malformed body.
4. Next time that query runs, `ctr_for` returns the doc's smoothed CTR and the re-ranker nudges it.

### 4.5 Category

Simpler than first specified: `search.rs::interaction_category` maps the requested vertical to
`news`, `files` or `web`. The catalog-style taxonomy (government / education / health …) was not
built (2026-08-27); a bounded `&'static` set is what makes the admin facet metric-safe.

## 5. Configuration (`[interaction]`, `xustive_core::config::InteractionConfig`)

| Key | Default | Notes |
|---|---|---|
| `enabled` | `false` | Off is off — no store connects, no token, no beacon target. |
| `k_anonymity` | `20` | ADR-0008 floor; `< 20` is refused outside `dev`. Dev uses `1`. |
| `window_days` | `90` | Sliding retention; CTR reflects the last quarter. |
| `hot_click_floor` | `0` | Clicks before a doc is a re-crawl candidate; `0` = use k. |
| `salt` | `""` | Keys the query hash; required outside dev; env `XUSTIVE_QHASH_SALT`. Redacted in the admin config view. |

Rotating the salt orphans the `qd:`/`qk:` counters, which decay within the window — rotation costs
at most one window of click signal.

## 6. Failure modes

| Failure | Behaviour |
|---|---|
| Redis unreachable at startup | `connect_interactions` warns and search runs without the term. |
| Redis slow/down per request | `ctr_for` returns empty; the ranker sees the neutral prior. |
| Store disabled | No token minted, no field, endpoint is a silent 204. |
| Token expired / unknown / body malformed | 204; nothing recorded. |
| Below k | `ctr_for`/`top_queries`/`hot_docs` omit the entry; the signal does not exist yet. |
| Token flood | Map capped at 4096, oldest evicted (BUG-024). |
| Click flooding one doc | Window + `hot_docs` cap (200/pass) + the bounded weight limit the skew. |

## 7. Tests

- Unit (`interaction.rs`): the k floor is at least one; Wilson rewards both rate and volume; no
  clicks is a low but defined score; the query hash hides the text and is salt-dependent; no key
  can contain an identifier.
- Redis integration (`crates/xustive-ingest/tests/interaction_redis.rs`): CTR surfaces only above
  k; a click by qhash matches a click by query; analytics readers surface only above the floor.
- Ranking: `default_weights_keep_relevance_dominant` and
  `adjacent_candidates_are_reorderable_but_distant_ones_are_not` hold with the term present.
- API: the beacon decodes the minimal shape and drops a smuggled `query` field.
- Config: non-dev refuses `k < 20` and an empty salt.

## 8. Related

[[ADR-0015 - Anonymous Interaction Signals for Ranking]] · [[ADR-0018 - Anonymous Search History]]
· [[ADR-0008 - No Query Logging]] · [[ADR-0001 - Two-Plane Architecture]] ·
[[ADR-0026 - The Reader's Language as a Bounded Ranking Signal]] · [[Ranking and Relevance]] ·
[[Query Pipeline]] · [[Crawler Orchestrator]] · [[Task Queue]] · [[Interaction Signals|weak_coverage]] ·
[[Milestone 6 - Adaptive Ranking from Interaction Signals]]
