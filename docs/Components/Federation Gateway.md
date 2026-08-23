---
tags:
  - component
  - ingestion
  - serving
component-id: C31
binary: xustive-federator
status: specified
updated: 2026-08-23
---

# Federation Gateway

> **ID** C31 · **Binary** `xustive-federator` · **Upstream** [[Query Pipeline]] (serving side), self-hosted SearXNG + allowlisted external tools (egress side) · **Downstream** [[Query Pipeline]] (blended results), [[Crawler Orchestrator]] (crawl hints) · **Governed by** [[ADR-0017 - Query-Time Federation with External Metasearch]]

## 1. Why this exists as its own process

The serving plane **has no route to the open internet** — enforced, not aspirational ([[ADR-0001 - Two-Plane Architecture]], `scripts/test-egress.sh`, `core` network `internal: true`). But [[ADR-0017 - Query-Time Federation with External Metasearch]] wants a live user query to borrow recall from a metasearch aggregator and blend it with our own results.

The only way to have both is a **single, narrow, allowlisted hop**. `xustive-federator` is that hop: a stateless sidecar on a bridged tier — one interface on `core` facing [[Query Pipeline]], one on an egress network facing a **self-hosted SearXNG** and a fixed endpoint allowlist. The API gains exactly one new outbound target (this gateway) and still cannot reach anything else. Egress lives here, behind an allowlist we own, so the serving plane's no-egress property survives as *"one allowlisted internal hop"* — provable in CI.

It is its own binary for the same reason [[Tool Data Plane]] is: **so it cannot acquire capabilities the plane it serves must not have.** The API links no HTTP-egress client; the federator holds the only one.

## 2. Responsibilities

**In scope**
- Accept a sanitised query from [[Query Pipeline]] over `core`; fan it out to enabled tools (SearXNG first) within a hard latency budget.
- Normalise each tool's response into a common `FederatedHit { url, title, snippet, engine, rank }`.
- Deposit each hit's URL as a **capped crawl hint** in Redis for the [[Crawler Orchestrator]] (the sanctioned cross-plane channel — the gateway never calls the crawler).
- Enforce the endpoint **allowlist**, per-tool `enabled` flags, budgets, and credentials from config.
- Expose health + per-tool latency/yield for the admin **Integrations** page.

**Out of scope**
- Ranking. The gateway returns tagged candidates; [[Ranking and Relevance]] blends and caps them. **No.** — the federator does not decide final order.
- Storing results. It writes only crawl hints; it holds no index and no durable result cache (a short in-process TTL cache for identical concurrent queries is the only state).
- Fetching page content. It receives URL lists (and snippets), never bodies — content acquisition is the crawler's job, so provenance and politeness stay in one place.
- Being on the answer's critical path. Federation is additive and fail-open (§7); a missing gateway degrades to index-only.

## 3. Interface

- **In (from serving):** `POST /federate {query, lang, budget_ms}` on `core` only. Returns `{hits: [FederatedHit], partial: bool}`; `partial=true` when the budget cut a tool short.
- **Out (egress):** SearXNG JSON API (self-hosted); optionally an external summariser MCP (Parallel-AI) — each behind its own `enabled` flag and the allowlist.
- **To crawler:** `federation:hint:<url>` capped/​windowed Redis keys, read by the revisit/discovery pass (mirrors the `hot_docs` warm-hint mechanism).
- **To admin:** `GET` health/stats consumed by `/api/v1/admin/integrations`.
- Config `[federation]` in [[xustive-core]]: `enabled=false`, per-tool `{ enabled, endpoint, api_key?, budget_ms, max_hits }`, blend `cap`, allowlist.

## 4. Datasets

None owned. Transient inputs (query, tool responses) and transient outputs (crawl hints, which the crawler drains). SearXNG's own engine set and settings are its configuration, not ours to model.

## 5. Validation

- Every outbound host is checked against the allowlist before the request leaves; a non-allowlisted target is refused, not dialled.
- Tool responses are parsed defensively (pure, fixture-tested parsers, like [[Crawler Orchestrator]]'s SERP parsers); a malformed response yields zero hits, never a panic.
- URLs pass the same SSRF guard and crawler-trap detector the force-crawl path uses before becoming crawl hints.

## 6. Provenance

Each hit carries `engine` and is tagged `source=federation` through the pipeline, so a blended result is distinguishable in [[Ranking and Relevance]]'s `Explain` and in the console. A crawl hint records the discovering query's *category* (not text) for the discovery funnel, consistent with [[weak_coverage]].

## 7. Failure

**Fail-open, always.** The gateway runs concurrently with local retrieval; on timeout, error, rate-limit, or disabled tool the pipeline ships index-only results — today's behaviour. A [[circuit breaker|circuit breaker (SharedBreaker)]] wraps each external tool so a persistently failing engine is shed fast rather than eating the budget every request. The gateway being **down** is indistinguishable to the user from federation being **off**.

## 8. Security

- One interface on `core`, one on egress; the API can reach the gateway and nothing beyond it (`test-egress.sh` asserts *only* the gateway is reachable from `xustive-api`).
- No query text logged; `query`/`token` stay forbidden telemetry field names. Query terms transit **through our SearXNG** to engines with no client IP, cookie, session, or identifier — the exposure [[ADR-0013 - Direct SERP Collection for Discovery]] accepted, now at request time and reconciled in [[ADR-0008 - No Query Logging]].
- The external summariser (Parallel-AI) is opt-in, offline-preferred, and flagged separately because it is genuine third-party SaaS.

## 9. Observability

- **Metrics:** `federation_requests_total{tool,outcome}`, `federation_latency_ms{tool}` (histogram), `federation_hits{tool}`, `federation_blend_share` (fraction of result page from federation — the convergence metric, expected to fall), `federation_budget_exceeded_total`. Bounded-cardinality labels only; **no query label**.
- **Log events:** tool enabled/disabled, breaker open/close, allowlist refusal — never the query.

## 10. Open questions

- Blend order: interleave federated hits into the candidate pool pre-rerank (current plan) vs. a separate "from the web" strip. Pre-rerank keeps one ranked list; measure whether provenance-tagged blending beats a labelled strip for trust.
- Live external summarisation vs. offline-only: default is offline enrichment; revisit if a live path proves fast and cheap enough under budget.
- Whether the crawl-hint feed should prioritise by federation frequency the way [[Interaction Signals]] prioritises re-crawl.

## 11. Test plan

- Egress test: `xustive-api` reaches the gateway and no other host; the gateway reaches only allowlisted endpoints.
- Fail-open: with the gateway stopped/timing out, `/search` returns index-only within its normal budget (no added latency) — explicit test.
- Parser fixtures: recorded SearXNG responses → expected `FederatedHit`s; malformed input → zero hits, no panic.
- Convergence: a federated URL, once crawled+indexed, is answered locally on the next identical query and its federation tag disappears.
- Privacy: no code path attaches an identifier to a federate request or a crawl hint; no query text reaches a log/metric/span.

## 12. Decisions

- [[ADR-0017 - Query-Time Federation with External Metasearch]] — why this exists, and the invariants it must keep (serving-plane no-egress preserved as one allowlisted hop; fail-open; self-hosted SearXNG; default off; converge to standalone).

## Related

[[ADR-0017 - Query-Time Federation with External Metasearch]] · [[Query Pipeline]] · [[Crawler Orchestrator]] · [[Ranking and Relevance]] · [[Tool Data Plane]] · [[weak_coverage]] · [[Security and Privacy]] · [[Milestone 7 - Federated Retrieval and External Tools]]
