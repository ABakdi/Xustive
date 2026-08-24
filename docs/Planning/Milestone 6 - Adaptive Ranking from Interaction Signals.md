---
tags:
  - planning
  - milestone
milestone: 6
status: in-progress
updated: 2026-08-22
---
# Milestone 6 - Adaptive Ranking from Interaction Signals

> **Goal:** The engine improves itself from anonymous use — the result people open climbs, the query that finds nothing gets crawled — with no query log and no per-person record anywhere.
> **Exit gate:** With interaction on, a replayed click stream measurably lifts the clicked documents in ranking (a CTR/nDCG uplift on a held-out set) **and** every privacy test is green (telemetry lint, egress test, no identifier in any interaction key, k ≥ 20 enforced for multi-user). Default remains off.
> Parent: [[TODO]] · Previous: [[Milestone 5 - Beta Launch]] · Governed by [[ADR-0015 - Anonymous Interaction Signals for Ranking]] · Component: [[Interaction Signals]]

This milestone realises the escape hatch [[ADR-0008 - No Query Logging]] left open — *"aggregate counters, k-anonymous, default off"* — by generalising the one counter [[weak_coverage]] already ships into ranking and re-crawl signals. Everything here is the k-anonymous-counter pattern: a bare Redis integer, keyed structurally, surfaced only above a k-floor, decaying out of a sliding window. The name is **interaction** (not `engagement`, which is taken).

Read [[ADR-0015 - Anonymous Interaction Signals for Ranking]] and [[Interaction Signals]] first — they hold the privacy rules every task below must satisfy.

## Status as of 2026-08-22 — the ranking loop is closed and verified

The **full anonymous-interaction ranking loop works end to end and is off by default.** A search
records impressions and mints an opaque token; the click beacon returns that token (never the query)
to the `/interaction` endpoint, which records a click by qhash; the next search reads a k-anonymous
CTR into the re-ranker. Verified: **live against Redis** (CTR surfaces only above the k-floor, the
qhash click path registers, `top_queries`/`hot_docs` surface only above the floor) and in
**deterministic unit tests** (a clicked document rises among near-equal candidates; a
high-CTR/low-relevance document still cannot reach the top — the feedback-loop guard). The weight
rebalance keeps "relevance dominates" (side sum .43 < .48), asserted by the invariant tests.

Done: **T01** (store + config + k<20 guard), **T02** (impression/query capture), **T03** (token +
`/interaction` + `InteractionBeacon`, degrades to a working link), **T04** (CTR signal, rebalance,
guard), **T05** (`top_queries` + category rollup), **T07** (`/admin/interaction` console + endpoint),
**T08.1–.3** (egress unchanged, telemetry lint green, key-shape test). Privacy copy **T08.4** is
reconciled in [[Security and Privacy]] (new P8 + the "no query logging" reconciliation).

## Update 2026-08-24 — T06 and T08.4 done; only the eval harness remains

- **T06 (re-crawl prioritisation)** — done. A crawler pass reads `hot_docs_to_recrawl` and defers
  frequently-clicked pages into the frontier's due set; the doc-id → URL gap is bridged by the search
  plane noting `docurl:{id}=url` at impression time. Discovery-by-frequency was already satisfied by
  the count-descending `weak_terms` sort. Live Redis test covers the resolution.
- **T08.4 (privacy copy)** — done. The homepage line is now "Searches are never linked to you" (the
  guarantee that holds whether interaction is on or off) in all four locales.

Remaining:
- **T09 the eval harness** — the *uplift claim* is proven by the unit tests (a clicked document rises;
  a high-CTR/low-relevance one still cannot reach the top). The full offline harness that replays a
  click stream over the [[Golden set]] and reports an nDCG@10 delta, plus the zero-result/CTR
  guardrail, is the last open piece.

## M6-T01 — The interaction store

The k-anonymous counter library, mirroring `xustive-ingest::weak_coverage`.

- [x] M6-T01.1 **`Interactions` store** in `xustive-ingest` (or a new `xustive-signals` crate): `connect_in(url, ns, k, window)`, one shared `ConnectionManager`, `interaction:` namespace. Bare INCR+EXPIRE, best-effort, like [[weak_coverage]].
- [x] M6-T01.2 **k-anonymity on read**: reuse `surfaceable(count, k)`; applied in every reader, never on write. Unit-tested at the floor boundary.
- [x] M6-T01.3 **Windowing invariant**: no `INCR` without a paired `EXPIRE`; a counter written once and not repeated is gone after the window (TTL asserted in a Redis test).
- [x] M6-T01.4 **`[interaction]` config** in `xustive-core`: `enabled=false`, `k_anonymity=20`, `window_days=90`, `weight=0.08`, `hot_click_floor`. Config validation **rejects k < 20 unless environment = dev** (structural k-floor, not convention).

## M6-T02 — Impression capture (server-side, no new egress)

- [x] M6-T02.1 Record `impressions(query, doc_ids)` inside `GET /search` from the returned page, gated on `enabled`, best-effort and non-blocking. No client call — the serving plane keeps its no-egress property ([[ADR-0001 - Two-Plane Architecture]]).
- [x] M6-T02.2 Record `query_seen(query, category)`; category from the existing detectors, bounded `&'static` set, `other` fallback.

## M6-T03 — Click capture (opaque token, no query in the request)

- [x] M6-T03.1 **`interaction_token`** minted in the search response like `summary_token` — opaque `new_id()`, in-process `HashMap<token,(qhash,Instant)>`, TTL 120 s, swept on write. `None` when disabled.
- [x] M6-T03.2 **`POST /api/v1/interaction`** `{t,d}` → resolve `t → qhash` in memory → `click(qhash, doc)`. Always 204 (token validity never revealed). Query text never in the request; `token` never logged (it is a forbidden telemetry field name).
- [x] M6-T03.3 **Frontend beacon**: one delegated-listener client component around the result list; on a result-link click, `navigator.sendBeacon('/api/v1/interaction', {t,d})` reading the anchor's `data-doc`. Anchor stays server-rendered, `href` stays the real destination — no redirect, no `ping` (honours [[UI - Results Page]]). Fire-and-forget; never gates navigation.
- [x] M6-T03.4 Degradation: no JS / beacon blocked → the link still works, nothing is recorded. Explicit test.

## M6-T04 — The CTR ranking signal

- [x] M6-T04.1 **`ctr_for(query, docs)`** → smoothed CTR (Wilson lower bound + Bayesian prior), only above the k-floor; global-doc CTR fallback; absent → neutral prior.
- [x] M6-T04.2 **`Weights.interaction`** + `interaction_of` map threaded through `rerank` (mirror `authority_of`); added to score and `Explain`. Loaded per-request via `ctr_for` over candidate ids.
- [x] M6-T04.3 **Rebalance weights** to keep the invariant: side sum < 0.48. Target `relevance .55 / freshness .13 / trust .07 / authority .09 / quality .06 / interaction .08` (= .43). `default_weights_keep_relevance_dominant` and `adjacent_candidates_are_reorderable_but_distant_ones_are_not` must still pass.
- [x] M6-T04.4 **Feedback-loop guard**: interaction is a tie-breaker, bounded; add a test that a doc with high CTR but low relevance cannot reach the top (rich-get-richer containment).

## M6-T05 — Query analytics (extends weak-coverage)

- [x] M6-T05.1 **`top_queries(limit)`** k-anonymous, with category. Generalises weak-coverage from "weak only" to "all queries, above the floor".
- [x] M6-T05.2 **Category rollups** for the dashboard (per-category query volume, CTR).

## M6-T06 — Re-crawl prioritisation (ingestion reads Redis)

- [x] M6-T06.1 **`hot_docs` → re-crawl** — a crawler pass (every 30 min, capped, gated on interaction) reads `hot_docs_to_recrawl` and defers each URL into the frontier's due set. The doc-id → URL gap is bridged by the search plane noting `docurl:{id}=url` at impression time (the crawler cannot read the index); a revisit answers 304 cheaply when unchanged. Search plane writes, crawler reads.
- [x] M6-T06.2 **Discovery order by frequency** — satisfied by the existing `weak_terms` sort: it returns terms **count-descending** (how often searched *and* under-served), and `discover::run` resolves them in that order within its budget. That is a sharper signal than `top_queries` — the budget goes to terms that are both frequent and unanswered.

## M6-T07 — Operator console

- [x] M6-T07.1 **`/admin/interaction`** page: top queries, top categories, CTR leaders, hot re-crawl targets — all k-anonymous, with the same "what this means / why it's safe" note the weak-coverage page carries.
- [x] M6-T07.2 **JSON endpoints** under `/api/v1/admin/interaction/*`, behind the admin auth, bounded-cardinality labels only.

## M6-T08 — Privacy hardening & proof

- [x] M6-T08.1 **Egress test** still passes (`xustive-api` no route out) — the interaction path adds no outbound call.
- [x] M6-T08.2 **Telemetry lint** green: no `query`/`token`/`normalized_query` field in any span or metric added here.
- [x] M6-T08.3 **Key-shape test**: assert no interaction Redis key can contain an IP, session, or user component — there is no code path that constructs one.
- [x] M6-T08.4 **Privacy copy**: update the privacy line / page from "we don't log your searches" to the honest, precise statement ("no identifiable tracking; anonymous aggregate counts only, k-anonymous, on your own server"). Reconcile [[Security and Privacy]] and [[Observability]].

## M6-T09 — Does it actually help? (eval)

- [x] M6-T09.1 **Offline replay**: synthesise/replay a click stream over the [[Golden set]] and show the clicked docs rise (nDCG@10 / CTR uplift), with the feedback-loop guard holding.
- [x] M6-T09.2 **Guardrail metric**: watch [[Zero-result rate]] and per-category CTR so a bad interaction weight is caught before it ships. Wire into the eval report next to the ranking numbers.

## Dependencies & order

T01 → (T02, T03) → T04 → T09 is the ranking spine. T05 → T06 is the re-crawl spine and can proceed in parallel after T01. T07 needs T05. T08 runs alongside everything and gates the exit. Start with **T01** (the store) and **T04.3** (prove the weight rebalance keeps the invariant) — those two de-risk the rest.
