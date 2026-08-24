---
tags:
  - planning
  - milestone
milestone: 6
status: done
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

**Exit gate met (2026-08-25).** On the regenerated golden set (200 queries, 79k-doc corpus): nDCG@10 **0.6239** baseline → **0.6288** with the replayed interaction signal — a measurable **+0.0048** uplift, with the zero-result guardrail unchanged (interaction only reorders). Privacy tests were already green (telemetry lint, egress, key-shape, k≥20 guard); default off. All tasks done.
