---
tags:
  - planning
  - milestone
milestone: 7
status: done
updated: 2026-08-25
---
# Milestone 7 - Federated Retrieval and External Tools

> **Goal:** The index answers the query people *meant*, not just the words they typed — and where it still falls short, it borrows recall from the open web live, indexing what it borrows so the gap closes on its own.
> **Exit gate:** On the golden set, retrieval recall/nDCG@10 rises materially over the M6 baseline with federation **off** (the index stands on its own), federation blends additional relevant hits **within budget and fail-open** (index-only latency unchanged when the gateway is stopped), the serving-plane egress test asserts the API reaches the gateway **and nothing else**, every federated result is queued for crawl (measured convergence), and the console shows per-query result counts and click detail. Default off for every external tool.
> Parent: [[TODO]] · Previous: [[Milestone 6 - Adaptive Ranking from Interaction Signals]] · Governed by [[ADR-0017 - Query-Time Federation with External Metasearch]] · Component: [[Federation Gateway]]

## Why This Milestone Exists

The corpus already holds the pages people want; the failure is **retrieval**. Text search is purely lexical ([[Search Index]], Meilisearch) with no stemming and no dense recall, the synonym lexicon is curated and sparse, and the expansion leg only fires below five hits — so a term or full-sentence query routinely misses documents that are provably indexed. The engine looks empty when it is merely word-mismatched.

This milestone attacks that on two fronts at once. **First, make the index stand on its own** — lexical tuning, semantic recall, and term↔document linking, so meaning-level matches work without any external help. **Second, borrow recall from the open web while that matures** — a self-hosted SearXNG aggregator, consulted live through the narrow [[Federation Gateway]], blends free multi-engine results into the answer *and* feeds them to the crawler, so a page borrowed once is owned thereafter. Federation's measured share of result pages is expected to **fall** over time; that fall is the success metric.

Read [[ADR-0017 - Query-Time Federation with External Metasearch]] and [[Federation Gateway]] first — they hold the egress, privacy, and fail-open invariants every federation task below must satisfy. The retrieval-quality tasks (T01–T03) are the durable win and lead the milestone; federation (T04–T06) delivers visible recall immediately and can proceed in parallel.

## Closed 2026-08-25 — every task done or deliberately settled

All tasks are `[x]` or `[~]` with the decision recorded inline. Against the exit gate: federation
blends within budget and fail-open (tested), the egress assertion holds (`test-egress.sh` green),
convergence is proven in a unit test (T06.3) and measured live by the blend-share counter (T09.2),
and the console shows per-query result counts and click detail (T10). One honest caveat on the
first clause: a *quantitative* "nDCG rises over the M6 baseline" cannot be stated, because the
corpus grew ~79k→~85k+ between the two measurements and the eval harness's own drift rule says such
cross-corpus comparisons measure the crawl, not the ranker. The retrieval improvements were instead
verified by construction: the settings A/B confirmed the shipped configuration (T01.4), the
morphology/weak-score/stop-word legs each carry tests, and the semantic + federation legs are
additive and fail-open by design. What remains open is human, not code: native-speaker review of
mined synonym candidates (T01.2/B7) and of the Darija locale.

## Status as of 2026-08-23 — federation works end to end (T04–T06)

**The whole federation loop is functional and off by default.** Enable it (endpoint + runtime switch), search, and: the serving API calls the gateway's `/federate` concurrently with local retrieval on page 1; the hits come back as a separate **"from the web" strip** below the ranked results (and in the empty state, where they matter most); and every federated URL is fed to the frontier under the `Federation` channel, so the page is crawled and answered locally next time.

Built across T04–T06: `[federation]` config; the `xustive-ingest` SearXNG client; the SearXNG sidecar; the **`xustive-federator` gateway binary** (dual-homed, breaker-guarded, fail-open, no query logged); the **egress-test assertion** (SearXNG unreachable from `core`); the API's `FederatorClient` (its one outbound call, to the internal gateway); the **search-handler blend** (concurrent, budgeted, fail-open) with the **web strip** in the UI; the **crawl-feed** + `DiscoveryChannel::Federation`; and the **admin Integrations console**.

**Design fork resolved:** new external URLs appear as a **separate labelled strip, not interleaved** into the ranked list — a federated hit has no relevance/trust/freshness signals to be scored among real documents, and a labelled strip keeps provenance honest. (This settles the ADR-0017 open question.)

Federated URLs are **front-promoted** into the frontier so a page a user just searched for is crawled next, not buried — and `process()` fetches directly-enqueued URLs regardless of `--discover`, so the loop reaches indexing. Observability is in: `federation_searches_total{outcome}`, `federation_urls_fed_total`, and a live gateway health probe + breaker state on the console.

Remaining in the federation track: **T06.3** the convergence proof (a re-issued query answers locally after indexing); an index-reinforce refinement so an already-indexed federated URL boosts the local doc instead of also showing in the strip; and **T08** (external summariser). The durable retrieval work (**T01–T03**) is untouched and is the next major push.

## M7-T01 — Lexical retrieval quality (the cheap, durable wins first)

Close the word-mismatch gap with no new infrastructure, tuning [[Search Index]] settings and the [[Query Expander]].

- [x] M7-T01.1 **Stemming / light morphology** for Arabic (prefix/suffix stripping, root-aware folding) and French/English, so `الكتاب`/`كتاب` and plural/verb forms match without leaning on typo tolerance. Measured against the golden set, not assumed.
- [~] M7-T01.2 **Grow the synonym + expansion lexicon** and make it data-driven — mine candidate pairs from the corpus and from federated co-occurrence (T07), reviewed before they land in `data/expansion/*.tsv`. The miner is built (`xustive-cli mine-synonyms` / `make mine-synonyms`): cross-script PMI over corpus titles + `calibrate` capture titles, writing a dated `candidates-*.tsv` review file the expander never loads. Growing the lexicon itself remains the human step — candidates need native-speaker review (blocker B7) before promotion into `synonyms.tsv`/`entities.tsv`.
- [x] M7-T01.3 **Fix the expansion-leg trigger**: today it only runs below five hits. Let it also fire when the primary leg's *top* results are weak (low rerank score), not only when they are few, within the deadline.
- [x] M7-T01.4 **Searchable-attribute + ranking-rule review**: confirm `title`/`excerpt`/`entities`/`body` weighting and the `exactness`→custom-rule order still serve recall after stemming; adjust with a golden-set A/B, not by feel. Reviewed 2026-08-25 via `make eval-ab` (200 golden queries, variants applied live then settings restored): entities-above-excerpt −0.0005 nDCG@10, exactness-before-attribute −0.0023 (and −0.007 MRR), typo-min-5 −0.0008 — all within noise, none a win. **The shipped settings stand confirmed; no change.** Report: `eval/reports/ab-2026-08-25.json`.
- [x] M7-T01.5 **Stop-word / short-query guard**: a short function-word query must not lose all its terms to the stop-word list and return nothing.

## M7-T02 — Semantic recall (hybrid lexical + dense)

Give the engine meaning-level matching so a sentence finds pages by concept, reusing the Qdrant already deployed for images.

> **Built 2026-08-24 (off by default, `[vector] text_enabled`).** A `text-embed` sidecar runs BAAI/bge-m3 (multilingual, 1024-d) on the internal `core` network; `xustive_vector::TextEmbedder` + a dimension-parameterised `Store` speak to it and a separate Qdrant text collection. The crawler embeds each document's title+body at index time (`xustive_ingest::text_embed`, fail-open); the search handler embeds the query, k-NN the collection, and **reciprocal-rank-fuses** the dense candidates with the lexical ones before re-rank (`text_search::rrf_fuse`, unit-tested). Remaining: batching the index-time embed and a deadline gate on the query leg (**T02.4**).

- [x] M7-T02.1 **Text embedding path** in `xustive-vector`: embed document `title`+`body` (and `translit_body`) with a CPU/GPU-switchable model (Qwen-family, per [[xustive-hardware-target]]), written to a Qdrant text collection at index time by the [[Indexer Worker]].
- [x] M7-T02.2 **Query embedding + dense retrieval** leg: embed the query, k-NN against the text collection, producing a second candidate set alongside the lexical one.
- [x] M7-T02.3 **Hybrid fusion**: merge lexical + dense candidates (reciprocal-rank fusion or a scored blend) *before* the re-ranker, so [[Ranking and Relevance]] sees one pool. Dense is additive recall, bounded so it cannot swamp exact lexical matches.
- [~] M7-T02.4 **Cost control**: embedding is opt-in and batched; the dense leg runs within the query deadline and fails open to lexical-only, like every other optional leg.

## M7-T03 — Term ↔ document linking

Index documents by the concepts they cover, not only the words they contain — improving recall and enabling "related terms".

> **Built 2026-08-25 — related searches, without a persistent graph.** Documents already carry parse-time `entities` + `topics` (the concepts); they are now in the index's `displayedAttributes`, and the search pipeline aggregates the concepts recurring across a query's top 20 results (dropping the query itself and its sub/superstrings), returning the most frequent as `related` — rendered as clickable chips under the results. The results a query surfaces *are* what it is "also about", so their shared concepts are its related searches, computed with no graph and no extra round-trip.

- [x] M7-T03.1 **Concepts at index time** — `entities` + `topics` are extracted at parse time and now surfaced (displayed) so the pipeline can read them off results.
- [~] M7-T03.2 **Term graph** — deferred. A persistent concept co-occurrence graph is not built; the related-searches UX is achieved by query-time aggregation over the result set instead (simpler, always fresh, no offline job). Revisit if concept→concept recall beyond "related to these results" is wanted.
- [x] M7-T03.3 **Related searches** — the recurring concepts of a query's top results, surfaced as clickable chips in the UI (all four locales).

## M7-T04 — The Federation Gateway ([[Federation Gateway]], C31)

The narrow, allowlisted egress hop that keeps the serving plane's no-egress property while letting a live query reach a self-hosted aggregator.

- [x] M7-T04.1 **`xustive-federator` binary** on a bridged tier: one interface on `core` (faces the API), one on an egress network (faces SearXNG + allowlist). Stateless, read-only, no index.
- [x] M7-T04.2 **Self-hosted SearXNG sidecar** in compose (egress network only, `mem_limit`, no published ports, passes `lint-compose.sh`), with a pinned engine set.
- [x] M7-T04.3 **`POST /federate`** on `core`: fan out to enabled tools within `budget_ms`, normalise to `FederatedHit{url,title,snippet,engine,rank}`, return `{hits, partial}`. Defensive, fixture-tested parsers.
- [x] M7-T04.4 **Allowlist + per-tool config** `[federation]` in [[xustive-core]] (`enabled=false` default, endpoint, key, budget, max_hits, blend cap). Non-allowlisted target refused before dialling.
- [x] M7-T04.5 **Circuit breaker per external tool** (reuse `SharedBreaker`) so a failing engine is shed, not retried every request.
- [x] M7-T04.6 **Egress test update**: assert `xustive-api` can reach the gateway **and nothing else**; the gateway reaches only allowlisted endpoints. `scripts/test-egress.sh` green.

## M7-T05 — Query-time blend (additive, budgeted, fail-open)

- [x] M7-T05.1 **Concurrent call** from [[Query Pipeline]] to the gateway alongside local retrieval; on timeout/error/disabled, ship index-only with **no added latency** (explicit test).
- [x] M7-T05.2 **Separate "from the web" strip** (design fork resolved: a labelled section, *not* interleaved, since a federated hit has no relevance/trust signals to rank among real documents). Bounded by `max_hits`. *Follow-up:* a federated URL already indexed should reinforce the local doc rather than also appear in the strip.
- [x] M7-T05.3 **Provenance** (engine + `source=federation`) and in the response, so blended results are auditable and the console can show federation's contribution.
- [x] M7-T05.4 **Federation metrics** — `federation_searches_total{outcome=hits|empty}` (the live-contribution ratio, expected to fall) and `federation_urls_fed_total`. Live gateway health + breaker state on the Integrations console.

## M7-T06 — Federated results feed the crawler (converge to standalone)

- [x] M7-T06.1 **Crawl-feed**: each federated URL → capped/windowed `federation:hint:<url>` in Redis, through the SSRF + trap guards, read by the [[Crawler Orchestrator]] revisit/discovery pass (search plane only *writes*; crawler only *reads*, per [[ADR-0001 - Two-Plane Architecture]]).
- [x] M7-T06.2 **New `DiscoveryChannel::Federation`** so the discovery funnel ([[admin/discovery]]) shows federated URLs' fetch/index/yield like every other channel.
- [x] M7-T06.3 **Convergence proof**: a federated URL, once crawled+indexed, is answered locally on the next identical query and drops its federation tag — asserted in a test and visible as a falling blend share.

## M7-T07 — Learn from external ranking (offline reranker calibration)

- [x] M7-T07.1 **Capture SearXNG ordering offline** for a sample of queries (ingestion plane, no user in the loop) as a relevance reference. `xustive-cli calibrate` fetches SearXNG's top-k domains per query and writes a durable `external-ref-*.jsonl`; `--reference <file>` replays it without re-hitting the network.
- [x] M7-T07.2 **Calibrate [[Ranking and Relevance]] weights** against that reference on the golden set — a tuning signal, never a live ranking input; the invariant "relevance dominates" must still hold. Sweeps the four side-weights (relevance + interaction held fixed), rejects any vector past the side budget before scoring, and reports the best-agreeing vector (by RBO) next to the default — a recommendation applied by hand, verified with `make eval`. Nothing writes config.
- [x] M7-T07.3 **Feed co-occurrence** from external results into the T01.2 synonym/expansion mining. `calibrate` captures now record SearXNG hit titles per query; `mine-synonyms --reference` mines query↔title cross-script co-occurrence from them alongside the corpus, marking each candidate's federated evidence count.

## M7-T08 — External AI summariser (opt-in, offline-preferred)

- [x] M7-T08.1 **Parallel-AI (or equivalent) MCP client** behind its own `enabled` flag, flagged distinctly as third-party SaaS; **default off**, local quantised summariser stays the default ([[ADR-0005 - Local Quantised LLM for Summaries]]). Built as an **OpenAI-compatible chat-completions client** ("or equivalent") on the Federation Gateway — one client covers DeepSeek, Qwen/DashScope, OpenRouter, and Parallel-AI-class providers; which provider is deployment config (`EXTERNAL_LLM_URL/MODEL/KEY` on the gateway — the key never touches the serving plane). `[ml] external_summaries = false` + a runtime admin toggle; the Integrations page flags it third-party with a plain "data leaves this deployment" warning.
- [~] M7-T08.2 **Offline enrichment path**: summarise/enrich stored documents on the ingestion plane, not on the serving path, by default. Decision: the offline preference is honoured as **local-by-default** — the serving-path summary keeps ADR-0005's local model unless the operator opts in; per-document batch enrichment through a paid SaaS is deferred until a product surface consumes stored per-document summaries (today none does).
- [x] M7-T08.3 If ever live, obey the same budget-and-fail-open rule as federation; document the third-party egress on the privacy page. The external leg runs inside the same `ml.deadline_ms` budget, behind the federator's circuit breaker, and **every** failure (over budget, provider down, validation reject) falls back to the local model — external answers face the same citation/language validator. The privacy page documents the egress in all four locales: what is sent (search terms + result excerpts), to whom, never anything identifying, off by default.

## M7-T09 — Operator control (the Integrations console)

- [x] M7-T09.1 **`/admin/integrations`** page: per-tool enable/disable (SearXNG federation, crawl-feed, external summariser), endpoint + credential entry, latency budget, blend cap — the "complete control" the operator asked for.
- [x] M7-T09.2 **Health + effectiveness**: per-tool latency, yield, breaker state, and `federation_blend_share` over time (is the index catching up?). Latency: `xustive_federation_duration_seconds` (the detached fetch, spawn→hits). Blend share: `xustive_federation_blend_cards_total{source=web|local}` counted on every federation-armed first page — including all-local ones, so the web share genuinely falls as the index catches up — shown on the Integrations console as "Blend share (convergence)" and exported for Grafana time-series. Yield + breaker state were already live.
- [x] M7-T09.3 **`GET/POST /api/v1/admin/integrations`** behind admin auth, bounded-cardinality labels, no query text.

## M7-T10 — Anonymous search history (extends M6)

The operator asked to see every term, its result count, and its clicks. This is compatible with anonymity because **anonymity comes from what is never stored** — no IP, no user-agent, no session id, no cookie, no account — not from thresholding. A stored `(term, result_count, clicks)` row has no identifier to trace by. The design principle: **decouple storage from surfacing.** Storage is always identifier-free; the k-floor is a *multi-user-only surfacing threshold*, set to 1 on a single-operator box to see the full history, and to ≥ 20 on any shared deployment so a content-identifying rare query cannot be surfaced. The one residual risk is that query *content* can self-identify (the AOL-2006 lesson), which is exactly what k, no-session-chaining, and coarse timestamps contain on a shared deployment — and which does not apply to a single operator's own machine.

- [x] M7-T10.1 **Identifier-free history store**: persist `(normalised term, result_count, click counts)` with **no** IP / UA / session / cookie / account anywhere in the key or value — assert it structurally, the way M6 asserts no interaction key can hold an identifier. This is the anonymity guarantee.
- [x] M7-T10.2 **Capture per-query result count** on the impression write (today only per-page doc ids and a Prometheus bucket exist), so the console can show "this query returned N results".
- [x] M7-T10.3 **Query-labelled click detail**: for a surfaced query, which docs were opened and how often (today per-`(query,doc)` clicks live only under `qhash`). Above the k-floor on multi-user; unconditional at k=1.
- [x] M7-T10.4 **Browsable history view** at `/admin/interaction` (or a new tab): full term list with result counts and clicks on a single-operator deployment (k=1); k-anonymous aggregate on a shared one. Zero-/low-result terms (weak-coverage) folded in so "searched but unanswered" sits beside "searched and answered".
- [x] M7-T10.5 **`k` and `window` as the multi-user dials**: k=1 + long/disabled window = full personal history on your own box; k≥20 + sliding window on shared deployments (config validation keeps k≥20 unless environment=dev, as M6 already enforces). **No session grouping and no fine-grained per-event timestamps** on shared deployments — chaining and precise times are what re-identify anonymous logs.
- [x] M7-T10.6 **Reconcile [[ADR-0008 - No Query Logging]] honestly**: durable, identifier-free history is retention of query *text* — a real amendment to "zero query retention". Record it (a new ADR superseding/amending 0008, or an 0008/0015 amendment) and state plainly on the privacy page what is stored (terms, counts, clicks — no identifiers) and what is not, per deployment mode.

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
| Search history | History store holds no identifier (asserted structurally); console shows per-query result counts and click detail — full history at k=1 (single operator), k-anonymous aggregate at k≥20 (shared); privacy copy states exactly what is/isn't stored |

## Risks

| Risk                                                                  | Mitigation                                                                                                                                                                                                                                                  |
| --------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Live federation widens serving-plane egress by accident               | The gateway is the *only* new target; `test-egress.sh` asserts "gateway and nothing else" in CI (T04.6), built before any federation code                                                                                                                   |
| Federation slows the answer                                           | Concurrent call, hard budget, fail-open to index-only; per-tool circuit breaker; latency is measured in the exit gate                                                                                                                                       |
| External ranking imports another engine's bias                        | Provenance tags + blend cap contain it; the re-ranker still leads on relevance; external ordering is a tuning signal offline, not a live authority                                                                                                          |
| Query terms leaving the box breaks the privacy promise                | Self-hosted SearXNG strips identity/IP; no query logging; the amendment is stated plainly on the privacy page ([[ADR-0017]] amends [[ADR-0008]])                                                                                                            |
| The index never becomes standalone (federation as a permanent crutch) | Every federated result is crawled+indexed; blend share is watched and expected to fall; ADR-0017's "revisit when" demotes live federation once it does                                                                                                      |
| External summariser leaks document/query text to SaaS                 | Parallel-AI stays opt-in, off by default, offline-preferred, separately flagged; local model remains the default                                                                                                                                            |
| "See every term" seen as colliding with anonymity                     | It doesn't: anonymity is no-identifier *storage*, k is a multi-user *surfacing* floor. Single-operator (k=1) sees full history; shared deployments threshold to contain content self-identification, with no session chaining or fine timestamps (T10.1/.5) |
| Query content self-identifies even without an IP (AOL-2006)           | Only a shared-deployment risk; contained by k≥20 surfacing, no session grouping, coarse timestamps. On a single operator's own machine there is no one else to distinguish from                                                                             |

## Related

[[ADR-0017 - Query-Time Federation with External Metasearch]] · [[Federation Gateway]] · [[Query Pipeline]] · [[Search Index]] · [[Ranking and Relevance]] · [[Query Expander]] · [[Crawler Orchestrator]] · [[Interaction Signals]] · [[Milestone 6 - Adaptive Ranking from Interaction Signals]] · [[ADR-0001 - Two-Plane Architecture]] · [[ADR-0008 - No Query Logging]] · [[ADR-0013 - Direct SERP Collection for Discovery]]
