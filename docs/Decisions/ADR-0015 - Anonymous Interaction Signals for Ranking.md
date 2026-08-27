---
tags:
  - adr
adr-id: "0015"
status: implemented
date: 2026-08-20
---
# ADR-0015 - Anonymous Interaction Signals for Ranking

## Status

Implemented (M6); extended by [[ADR-0018 - Anonymous Search History]]. **Amends [[ADR-0008 - No Query Logging]]** (the "No click tracking" and "Aggregate counters … default off" rows). Constrains [[Ranking and Relevance]], [[Query Pipeline]], [[Crawler Orchestrator]], [[Observability]], and the new [[Interaction Signals]] component. Built on the pattern established by [[Interaction Signals|weak_coverage]] and the escape hatch [[ADR-0008 - No Query Logging]] itself named.

## Context

The engine should get better at the thing it exists to do: put the result people actually want first. The only source of truth for "did we get it right" is what people do with the results — which ones they open, which queries return nothing worth opening. Today none of that is captured, and [[ADR-0008 - No Query Logging]] forbids capturing it: *"No click tracking, no redirect interstitials, no `ping`."*

But ADR-0008 did not close the door — it named the key: *"Aggregate counters (if ever enabled) must be k-anonymous, k ≥ 20 … default off,"* and its "Revisit when" clause names k-anonymity and differential privacy as the honest routes. [[Interaction Signals|weak_coverage]] already walks through that door for one signal (which searches came up short), and it is the template: a query-derived signal made **structurally** safe — a bare Redis counter keyed by the normalised term, surfaced only above a k-floor, decaying out of a sliding window — rather than safe by policy.

This ADR extends that one accepted counter into a small family of them, so ranking and re-crawl can learn from behaviour **without ever holding a person's search history**.

The word `engagement` is already taken in this codebase (social like/comment/share counts on a `Document`, and a ranking term of the same name). This signal is called **interaction** throughout to avoid the collision.

## Decision

**Capture interaction as anonymous, aggregate, k-anonymous counters — never as events tied to a person — and feed them into ranking and re-crawl through Redis, the sanctioned cross-plane channel.**

The rules, and how each is enforced structurally rather than by good behaviour:

| Rule | Enforcement |
|---|---|
| **No identifier is ever attached to an interaction** — no IP, no session id, no user id, no cookie, no device fingerprint | The `/interaction` endpoint reads none and stores none. There is no column for one. |
| **Impressions are recorded server-side**, from what the API already returned | No new client call, no new data leaves the browser; the serving plane keeps its no-egress property ([[ADR-0001 - Two-Plane Architecture]]) |
| **A click is bound to its query only through an opaque, single-search, in-process, TTL'd token** — the query text never rides in the click request | Mirrors the `summary_token` pattern; the click carries `{token, doc_id}`, and the server resolves `token → query hash` from memory, never from a durable store |
| **Every interaction counter is k-anonymous on read** — a `(query, doc)` or per-query signal is used or surfaced only once ≥ k **distinct searches** have contributed | The `surfaceable(count, k)` predicate from [[Interaction Signals|weak_coverage]], applied at read time in both the ranker and the dashboard |
| **Counters are windowed** — every write sets a sliding TTL, so an interaction not repeated within the window decays to nothing | Redis `EXPIRE` on every `INCR`, exactly as `weak_coverage` does |
| **Default off**, and off means off — no store connects, no counter is written | An `[interaction] enabled` flag, default `false`; k defaults to the ADR-0008 floor of **20** (the single-user dev config lowers it, as it does for weak-coverage) |
| **Query text never reaches a log, a metric label, or a span** | The [[Observability|telemetry lint]] still runs; the token is opaque and is never logged (note `token` is itself a forbidden field name); metric labels stay `&'static str` and bounded-cardinality |
| **The search plane never calls the crawler** | Popular-query re-crawl deposits a capped "warm this doc" hint in Redis; the revisit scheduler *reads* it, per [[ADR-0001 - Two-Plane Architecture]] |

**What is stored (all in Redis, all windowed, all under an `interaction:` namespace):**
- per-`(query, doc)` impressions and clicks — the strong signal, "for *this* query users open *this* doc";
- per-`doc` impressions and clicks — the fallback, a document's own click-through rate;
- per-query count + coarse category — extends [[Interaction Signals|weak_coverage]] from "weak only" to "all queries, k-anonymously", for the operator dashboard and re-crawl prioritisation.

**How ranking uses it:** a new `interaction` term in the re-ranker ([[Ranking and Relevance]]), a **smoothed** click-through rate (Wilson lower bound with a Bayesian prior, so one click on one impression is not 100 %), applied only above the k-floor, as a **tie-breaker** bounded to keep textual relevance dominant by construction — the same invariant the `authority` term respects.

**How re-crawl uses it:** a doc that is a frequently-opened result for a repeated query gets a **capped** freshness credit that pulls its revisit forward; frequent no-result queries keep feeding [[Interaction Signals|weak_coverage]] → discovery, now prioritised by frequency.

## Consequences

**Good**
- The engine improves from real use — the clicked result climbs, the ignored one sinks — without a query log existing anywhere.
- Nothing captured can be traced to a person: there is no identifier to trace *by*, and the query text is protected by the same k-anonymity + window that already guards weak-coverage.
- Re-crawl spends its budget where attention actually is.
- It is default-off and structurally inert until an operator turns it on.

**Bad**
- It amends a promise. "No click tracking" becomes "no *identifiable* click tracking; anonymous aggregate counters only." The privacy page must say this plainly rather than imply nothing at all is counted.
- k-anonymity is only meaningful with many searchers; on a single-user deployment (`k = 1`) the counters are effectively a personal history held in that person's own Redis — acceptable because it is their own machine, but the k=1 config must be understood as "no anonymity, single operator" not "anonymised".
- Position bias: clicks favour the top result regardless of quality. v1 uses a smoothed CTR with a prior; debiasing by position is a named follow-up, not a launch requirement.
- A click needs client JavaScript, which the results page has so far avoided on the result list. One small delegated-listener component is added, not a handler per result.

**Commits us to**
- k ≥ 20 on any multi-user deployment, enforced in config validation, not convention.
- Keeping the interaction store out of the telemetry/metrics path entirely — it is a ranking input, never observability.

## Alternatives

| Option | Why not |
|---|---|
| Per-user click history (accounts, opt-in) | The honest heavyweight route ADR-0008 names. Real, but it builds the exact profile this engine promises not to build; out of scope and against the product's reason to exist. |
| Differential privacy (noise-added counters) | The other route ADR-0008 names. Stronger math, but at this corpus and traffic scale the noise would swamp a signal that is already sparse; revisit at scale. |
| Nothing — stay fully blind | The status quo. Leaves the engine unable to learn from the one signal that matters, and forgoes the escape hatch ADR-0008 deliberately left open. |
| Name it `engagement` | Collides with the existing social-counts field and ranking term; guaranteed confusion. |

## Revisit when

- Traffic is high enough that differential privacy beats bare k-anonymity — switch the counters to a DP mechanism.
- Position bias visibly distorts ranking — add a position-debiasing model (e.g. a click model) before trusting raw CTR at higher weight.
- A multi-user deployment appears — re-audit that k ≥ 20 holds end to end and that no path can lower it.

## Related

[[ADR-0008 - No Query Logging]] · [[ADR-0001 - Two-Plane Architecture]] · [[ADR-0011 - Adaptive Recrawl over Static Crawling|ADR-0011]] · [[Ranking and Relevance]] · [[Interaction Signals]] · [[Interaction Signals|weak_coverage]] · [[Observability]] · [[Security and Privacy]] · [[Milestone 6 - Adaptive Ranking from Interaction Signals]]

## Where it stands (2026-08-27)

- `[interaction] enabled = false` by default; `k_anonymity` defaults to 20, the dev config lowers it to 1, and `Config::validate` refuses `k < 20` outside `environment = dev` (`crates/xustive-core/src/config.rs`).
- `POST /api/v1/interaction` (`crates/xustive-api/src/interaction.rs`) carries only `{t, d}`; the token is a ULID resolved to a query hash from an in-process map with a 120 s TTL; unknown or expired → nothing recorded, still `204`. The query hash falls back to unsalted FNV when the salt is empty — dev only.
- **Divergence, deliberate:** `EXPIRE` is re-armed only when the remaining TTL has fallen below half the window, not on every `INCR` (`crates/xustive-ingest/src/interaction.rs`, BUG-039) — refreshing on every write made `window − TTL` a per-term last-event timestamp, which ADR-0018 forbids.
- Ranking: Wilson lower bound, `interaction: 0.07`, additive tie-breaker only (`crates/xustive-search/src/rank.rs`). Re-crawl: the hot-doc hint is read by `crawld` (`crates/xustive-ingest/src/interaction.rs`), never pushed.
- Operator surface: `GET /admin/interaction` (`crates/xustive-api/src/lib.rs`).
