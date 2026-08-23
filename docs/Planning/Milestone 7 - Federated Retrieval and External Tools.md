---
tags:
  - planning
  - milestone
milestone: 7
status: planned
updated: 2026-08-23
---
# Milestone 7 - Federated Retrieval and External Tools

> **Goal:** The index answers the query people *meant*, not just the words they typed — and where it still falls short, it borrows recall from the open web live, indexing what it borrows so the gap closes on its own.
> **Exit gate:** On the golden set, retrieval recall/nDCG@10 rises materially over the M6 baseline with federation **off** (the index stands on its own), federation blends additional relevant hits **within budget and fail-open** (index-only latency unchanged when the gateway is stopped), the serving-plane egress test asserts the API reaches the gateway **and nothing else**, every federated result is queued for crawl (measured convergence), and the console shows per-query result counts and click detail. Default off for every external tool.
> Parent: [[TODO]] · Previous: [[Milestone 6 - Adaptive Ranking from Interaction Signals]] · Governed by [[ADR-0017 - Query-Time Federation with External Metasearch]] · Component: [[Federation Gateway]]

## Why This Milestone Exists

The corpus already holds the pages people want; the failure is **retrieval**. Text search is purely lexical ([[Search Index]], Meilisearch) with no stemming and no dense recall, the synonym lexicon is curated and sparse, and the expansion leg only fires below five hits — so a term or full-sentence query routinely misses documents that are provably indexed. The engine looks empty when it is merely word-mismatched.

This milestone attacks that on two fronts at once. **First, make the index stand on its own** — lexical tuning, semantic recall, and term↔document linking, so meaning-level matches work without any external help. **Second, borrow recall from the open web while that matures** — a self-hosted SearXNG aggregator, consulted live through the narrow [[Federation Gateway]], blends free multi-engine results into the answer *and* feeds them to the crawler, so a page borrowed once is owned thereafter. Federation's measured share of result pages is expected to **fall** over time; that fall is the success metric.

Read [[ADR-0017 - Query-Time Federation with External Metasearch]] and [[Federation Gateway]] first — they hold the egress, privacy, and fail-open invariants every federation task below must satisfy. The retrieval-quality tasks (T01–T03) are the durable win and lead the milestone; federation (T04–T06) delivers visible recall immediately and can proceed in parallel.

## M7-T01 — Lexical retrieval quality (the cheap, durable wins first)

Close the word-mismatch gap with no new infrastructure, tuning [[Search Index]] settings and the [[Query Expander]].

- [ ] M7-T01.1 **Stemming / light morphology** for Arabic (prefix/suffix stripping, root-aware folding) and French/English, so `الكتاب`/`كتاب` and plural/verb forms match without leaning on typo tolerance. Measured against the golden set, not assumed.
- [ ] M7-T01.2 **Grow the synonym + expansion lexicon** and make it data-driven — mine candidate pairs from the corpus and from federated co-occurrence (T07), reviewed before they land in `data/expansion/*.tsv`.
- [ ] M7-T01.3 **Fix the expansion-leg trigger**: today it only runs below five hits. Let it also fire when the primary leg's *top* results are weak (low rerank score), not only when they are few, within the deadline.
- [ ] M7-T01.4 **Searchable-attribute + ranking-rule review**: confirm `title`/`excerpt`/`entities`/`body` weighting and the `exactness`→custom-rule order still serve recall after stemming; adjust with a golden-set A/B, not by feel.
- [ ] M7-T01.5 **Stop-word / short-query guard**: a short function-word query must not lose all its terms to the stop-word list and return nothing.

## M7-T02 — Semantic recall (hybrid lexical + dense)

Give the engine meaning-level matching so a sentence finds pages by concept, reusing the Qdrant already deployed for images.

- [ ] M7-T02.1 **Text embedding path** in `xustive-vector`: embed document `title`+`body` (and `translit_body`) with a CPU/GPU-switchable model (Qwen-family, per [[xustive-hardware-target]]), written to a Qdrant text collection at index time by the [[Indexer Worker]].
- [ ] M7-T02.2 **Query embedding + dense retrieval** leg: embed the query, k-NN against the text collection, producing a second candidate set alongside the lexical one.
- [ ] M7-T02.3 **Hybrid fusion**: merge lexical + dense candidates (reciprocal-rank fusion or a scored blend) *before* the re-ranker, so [[Ranking and Relevance]] sees one pool. Dense is additive recall, bounded so it cannot swamp exact lexical matches.
- [ ] M7-T02.4 **Cost control**: embedding is opt-in and batched; the dense leg runs within the query deadline and fails open to lexical-only, like every other optional leg.

## M7-T03 — Term ↔ document linking

Index documents by the concepts they cover, not only the words they contain — improving recall and enabling "related terms".

- [ ] M7-T03.1 **Keyphrase / entity extraction** at index time → a `concepts` field, feeding both retrieval and a term graph.
- [ ] M7-T03.2 **Term graph** (concept → documents, concept ↔ concept co-occurrence) in Redis/Qdrant, built offline from the corpus.
- [ ] M7-T03.3 **Related-terms expansion**: a query's concepts pull in strongly-linked terms as a bounded recall aid, and surface "related searches" in the UI.

## M7-T04 — The Federation Gateway ([[Federation Gateway]], C31)

The narrow, allowlisted egress hop that keeps the serving plane's no-egress property while letting a live query reach a self-hosted aggregator.

- [ ] M7-T04.1 **`xustive-federator` binary** on a bridged tier: one interface on `core` (faces the API), one on an egress network (faces SearXNG + allowlist). Stateless, read-only, no index.
- [ ] M7-T04.2 **Self-hosted SearXNG sidecar** in compose (egress network only, `mem_limit`, no published ports, passes `lint-compose.sh`), with a pinned engine set.
- [ ] M7-T04.3 **`POST /federate`** on `core`: fan out to enabled tools within `budget_ms`, normalise to `FederatedHit{url,title,snippet,engine,rank}`, return `{hits, partial}`. Defensive, fixture-tested parsers.
- [ ] M7-T04.4 **Allowlist + per-tool config** `[federation]` in [[xustive-core]] (`enabled=false` default, endpoint, key, budget, max_hits, blend cap). Non-allowlisted target refused before dialling.
- [ ] M7-T04.5 **Circuit breaker per external tool** (reuse `SharedBreaker`) so a failing engine is shed, not retried every request.
- [ ] M7-T04.6 **Egress test update**: assert `xustive-api` can reach the gateway **and nothing else**; the gateway reaches only allowlisted endpoints. `scripts/test-egress.sh` green.

## M7-T05 — Query-time blend (additive, budgeted, fail-open)

- [ ] M7-T05.1 **Concurrent call** from [[Query Pipeline]] to the gateway alongside local retrieval; on timeout/error/disabled, ship index-only with **no added latency** (explicit test).
- [ ] M7-T05.2 **Blend** federated hits into the candidate pool pre-rerank, tagged `source=federation`, subject to a **cap** so external results never dominate a page; a federated URL already indexed reinforces the local doc, not a duplicate.
- [ ] M7-T05.3 **Provenance in `Explain`** and in the response, so blended results are auditable and the console can show federation's contribution.
- [ ] M7-T05.4 **`federation_blend_share` metric** — the convergence indicator, expected to fall as the crawl-feed (T06) fills the index.

## M7-T06 — Federated results feed the crawler (converge to standalone)

- [ ] M7-T06.1 **Crawl hints**: each federated URL → capped/windowed `federation:hint:<url>` in Redis, through the SSRF + trap guards, read by the [[Crawler Orchestrator]] revisit/discovery pass (search plane only *writes*; crawler only *reads*, per [[ADR-0001 - Two-Plane Architecture]]).
- [ ] M7-T06.2 **New `DiscoveryChannel::Federation`** so the discovery funnel ([[admin/discovery]]) shows federated URLs' fetch/index/yield like every other channel.
- [ ] M7-T06.3 **Convergence proof**: a federated URL, once crawled+indexed, is answered locally on the next identical query and drops its federation tag — asserted in a test and visible as a falling blend share.

## M7-T07 — Learn from external ranking (offline reranker calibration)

- [ ] M7-T07.1 **Capture SearXNG ordering offline** for a sample of queries (ingestion plane, no user in the loop) as a relevance reference.
- [ ] M7-T07.2 **Calibrate [[Ranking and Relevance]] weights** against that reference on the golden set — a tuning signal, never a live ranking input; the invariant "relevance dominates" must still hold.
- [ ] M7-T07.3 **Feed co-occurrence** from external results into the T01.2 synonym/expansion mining.

## M7-T08 — External AI summariser (opt-in, offline-preferred)

- [ ] M7-T08.1 **Parallel-AI (or equivalent) MCP client** behind its own `enabled` flag, flagged distinctly as third-party SaaS; **default off**, local quantised summariser stays the default ([[ADR-0005 - Local Quantised LLM for Summaries]]).
- [ ] M7-T08.2 **Offline enrichment path**: summarise/enrich stored documents on the ingestion plane, not on the serving path, by default.
- [ ] M7-T08.3 If ever live, obey the same budget-and-fail-open rule as federation; document the third-party egress on the privacy page.

## M7-T09 — Operator control (the Integrations console)

- [ ] M7-T09.1 **`/admin/integrations`** page: per-tool enable/disable (SearXNG federation, crawl-feed, external summariser), endpoint + credential entry, latency budget, blend cap — the "complete control" the operator asked for.
- [ ] M7-T09.2 **Health + effectiveness**: per-tool latency, yield, breaker state, and `federation_blend_share` over time (is the index catching up?).
- [ ] M7-T09.3 **`GET/POST /api/v1/admin/integrations`** behind admin auth, bounded-cardinality labels, no query text.

## M7-T10 — Search-history visibility (extends M6)

What the operator asked to see — every term, its result count, its clicks — within the anonymity model, with the policy tension made explicit ([[ADR-0008 - No Query Logging]]).

- [ ] M7-T10.1 **Capture per-query result count** on the impression write (today only per-page doc ids and a Prometheus bucket exist), so the console can show "this query returned N results".
- [ ] M7-T10.2 **Query-labelled click rollup**: join clicks to the query text above the k-floor (today per-`(query,doc)` clicks live only under `qhash`), so the console shows, for a surfaced query, which docs were opened and how often.
- [ ] M7-T10.3 **Zero-/low-result queries in the analytics view**: fold weak-coverage terms into the search-history page so "searched but unanswered" is visible alongside "searched and answered", both k-anonymous.
- [ ] M7-T10.4 **Policy decision + honest copy**: whether/how far to lower the k-floor for a single-operator deployment, recorded against [[ADR-0008 - No Query Logging]]; the privacy page states plainly what is and isn't captured. "Every term" below k stays structurally invisible on a multi-user deployment — say so.

## Dependencies & order

**T01 → T02 → T03** is the retrieval-quality spine and the durable win — start here so the index improves whether or not federation is ever enabled. **T04 → T05 → T06** is the federation spine and can run in parallel; T06 depends on T05, and T04.6 (the egress assertion) gates the whole federation path — build it early so no federation code can silently widen egress. **T07/T08** are enrichment, after T04. **T09** needs T04/T05 to have something to control; **T10** extends M6 and is independent of federation. Begin with **T01.1** (stemming — the single biggest recall win for the least infrastructure) and **T04.6** (prove the egress boundary holds before anything reaches out).

## Exit Gate

| Check | Threshold |
|---|---|
| Standalone retrieval | With federation **off**, golden-set recall / nDCG@10 rises materially over the M6 baseline (stemming + semantic + term-linking) |
| Federation is additive | Blended results add relevant hits on word-mismatched queries, tagged and capped, never dominating a page |
| Fail-open | With the gateway stopped, `/search` returns index-only within its normal budget — no added latency (measured) |
| Egress preserved | `test-egress.sh` asserts `xustive-api` reaches the gateway and **nothing else**; the gateway reaches only allowlisted endpoints |
| Convergence | Every federated URL is queued for crawl; a re-issued query answers locally after indexing; `federation_blend_share` trends down |
| Privacy | Telemetry lint green (no query/token field), no identifier on any federate request or crawl hint, k ≥ 20 enforced for multi-user |
| Operator control | Every external tool is enable/disable-able, keyed, budgeted, and observable from `/admin/integrations`; all default off |
| Search history | Console shows per-query result counts and per-query click detail, k-anonymous, with honest privacy copy |

## Risks

| Risk | Mitigation |
|---|---|
| Live federation widens serving-plane egress by accident | The gateway is the *only* new target; `test-egress.sh` asserts "gateway and nothing else" in CI (T04.6), built before any federation code |
| Federation slows the answer | Concurrent call, hard budget, fail-open to index-only; per-tool circuit breaker; latency is measured in the exit gate |
| External ranking imports another engine's bias | Provenance tags + blend cap contain it; the re-ranker still leads on relevance; external ordering is a tuning signal offline, not a live authority |
| Query terms leaving the box breaks the privacy promise | Self-hosted SearXNG strips identity/IP; no query logging; the amendment is stated plainly on the privacy page ([[ADR-0017]] amends [[ADR-0008]]) |
| The index never becomes standalone (federation as a permanent crutch) | Every federated result is crawled+indexed; blend share is watched and expected to fall; ADR-0017's "revisit when" demotes live federation once it does |
| External summariser leaks document/query text to SaaS | Parallel-AI stays opt-in, off by default, offline-preferred, separately flagged; local model remains the default |
| "See every term" collides with k-anonymity | T10.4 makes the policy explicit and the copy honest; below-k terms stay invisible on multi-user deployments by construction |

## Related

[[ADR-0017 - Query-Time Federation with External Metasearch]] · [[Federation Gateway]] · [[Query Pipeline]] · [[Search Index]] · [[Ranking and Relevance]] · [[Query Expander]] · [[Crawler Orchestrator]] · [[Interaction Signals]] · [[Milestone 6 - Adaptive Ranking from Interaction Signals]] · [[ADR-0001 - Two-Plane Architecture]] · [[ADR-0008 - No Query Logging]] · [[ADR-0013 - Direct SERP Collection for Discovery]]
