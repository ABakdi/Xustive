---
tags:
  - adr
adr-id: "0017"
status: accepted
date: 2026-08-23
---
# ADR-0017 - Query-Time Federation with External Metasearch

## Status

Accepted. **Amends [[ADR-0001 - Two-Plane Architecture]]** (the serving plane's no-egress property and the "planes couple only through the index" rule) and **extends [[ADR-0013 - Direct SERP Collection for Discovery]]** (query-term egress, until now confined to the ingestion plane and offline, is allowed onto the serving path under strict controls). Interacts with [[ADR-0008 - No Query Logging]] (query terms now leave the box at request time) and [[ADR-0005 - Local Quantised LLM for Summaries]] (an external summariser becomes an *optional* alternative, never the default). Introduces the [[Federation Gateway]] component and governs [[Milestone 7 - Federated Retrieval and External Tools]].

## Context

The corpus already holds the pages people want. The failure is **retrieval**: a term or a full-sentence query often does not surface documents that are demonstrably in the index — because text search is purely lexical (Meilisearch, no stemming, no dense recall), the synonym lexicon is curated and sparse, and the expansion leg only fires below five hits. The engine looks empty when it is merely *word-mismatched*.

Two things follow. First, the index needs to get materially better at answering from what it holds — lexical tuning, semantic recall, term↔document linking. Second, while that matures, we want to **borrow recall from the open web**: a metasearch aggregator (SearXNG — open source, self-hostable) returns, for free, a ranked URL list drawn from many engines, and those results are exactly the pages we should be surfacing *and* crawling. Feeding them back into our own index is what lets the index eventually stand on its own.

[[ADR-0013 - Direct SERP Collection for Discovery]] already accepted that query terms may leave the box to a general engine — but only in the **ingestion plane, offline**, to discover URLs for the frontier. This ADR asks the harder question: may the **serving path**, answering a live user query, consult an external aggregator and blend its results with ours in real time?

[[ADR-0001 - Two-Plane Architecture]] says no serving component may reach the internet, and enforces it — `scripts/test-egress.sh` fails the build if `xustive-api` can reach outside, and the `core` network is `internal: true`. A naive "let the API call SearXNG" would demolish that property and the "no user data leaves the country" guarantee ([[Security and Privacy]] P2). So the question is not *whether* to federate but *through what structure*, such that the serving plane keeps its no-egress property and the privacy promise is amended honestly rather than broken silently.

## Decision

**Federate at query time through a single, narrow, allowlisted [[Federation Gateway]] — never by giving the serving plane general internet access — blend within a hard latency budget that fails open to index-only, and queue every federated result for crawl so the index converges toward answering alone. Default off; operator-controlled per tool.**

The structure, and how each old invariant is kept or knowingly amended:

| Rule | How it holds |
|---|---|
| **The serving plane still has no route to the open internet** | `xustive-api` gains exactly one new outbound target: the internal Federation Gateway, over the `core` network. It cannot reach anything else. `test-egress.sh` is updated to assert the API can reach *only* the gateway, not the wider internet — the property becomes "one allowlisted internal hop", not "none". |
| **The gateway is the only egress-capable serving-side component** | `xustive-federator` sits on a bridged tier: one interface on `core` (faces the API), one on an egress network (faces a **self-hosted** SearXNG and a fixed endpoint allowlist). It is stateless, read-only, holds no index, and writes nothing durable except the crawl hints below. |
| **Metasearch is self-hosted** | SearXNG runs as our own sidecar. User query terms reach third-party engines *through our SearXNG*, which strips client identity, carries no IP, no cookie, no session, and coalesces requests. We own the aggregator; no third party sees our users, only anonymised query terms — the same exposure [[ADR-0013 - Direct SERP Collection for Discovery]] already accepted, now at request time. |
| **Federation never blocks the local answer** | The gateway call runs **concurrently** with local retrieval under a hard budget (default ~250 ms). Miss the budget and results ship index-only. Federation is strictly additive; a slow or dead external tool degrades to today's behaviour, never to a slow page. |
| **The index converges to standalone** | Every federated URL is deposited as a capped crawl hint in Redis (the sanctioned cross-plane channel), so the ingestion plane fetches and indexes it. A result borrowed once is owned thereafter — repeat queries answer locally, and federation's contribution visibly shrinks over time. That shrinkage is the success metric, not a regression. |
| **No query logging** | The gateway logs no query text, mints no per-user record, and honours the telemetry lint (`query`/`token` remain forbidden field names). Query terms leave the box **in flight** to engines; they are never *retained* here. [[ADR-0008 - No Query Logging]] is amended from "nothing derived from a query leaves" to "anonymised query terms may transit to a self-hosted aggregator at request time; nothing is logged or profiled." |
| **The AI summariser stays local by default** | Parallel-AI (or any external MCP summariser) is an **opt-in** enrichment path, flagged distinctly from self-hosted SearXNG because it is genuinely third-party SaaS. The default summariser remains the local quantised model ([[ADR-0005 - Local Quantised LLM for Summaries]]); external summarisation is preferred **offline** (enrich stored documents), and if ever used live it obeys the same budget-and-fail-open rule. |
| **Off by default, operator-controlled** | Each tool (SearXNG federation, federated crawl-feed, external summariser) has its own `enabled` flag and its own controls — latency budget, endpoint, credentials, blend cap — surfaced on the admin **Integrations** page. Nothing federates until an operator turns it on for a deployment. |

**Blending.** Federated hits are merged into the candidate pool *before* the re-ranker, tagged with provenance, and subject to a bounded cap so external results can never dominate a page — the same containment the `authority` and `interaction` signals already respect. A federated URL already in our index reinforces the local document rather than appearing twice.

## Consequences

**Good**
- Recall stops being hostage to our lexicon and analyzers: a word-mismatched query still finds the right pages, borrowed from the open web while our own retrieval matures.
- The corpus bootstraps itself from real queries — the pages users actually ask for get crawled and indexed, so coverage grows where attention is.
- One narrow, auditable egress path instead of a general one; the serving plane's no-egress property survives as "one allowlisted internal hop", provable in CI.
- Federation is default-off, per-tool, and fail-open — a deployment that wants a pure local index simply never enables it, and one that does is never slowed by it.

**Bad**
- It amends the strongest architectural promise. "The serving plane cannot reach the internet" becomes "cannot reach it except through one allowlisted gateway." The privacy page must state that, with federation on, anonymised query terms transit to external engines via our own aggregator.
- Self-hosting SearXNG adds an operational surface (a sidecar to run, update, and watch) and a dependency on engines that rate-limit and change.
- Blending external ranking risks importing another engine's biases; the cap and provenance tagging contain it but do not erase it.
- An external summariser (Parallel-AI) is real third-party egress of document/query text; it is why that path is opt-in, offline-preferred, and separately flagged rather than folded in with SearXNG.

**Commits us to**
- A CI egress assertion that the serving plane reaches the gateway **and nothing else**, so "no-egress" cannot silently widen into "some-egress".
- Federation as strictly additive and budgeted: it may never gate, slow, or replace the local answer.
- Feeding every federated result back to the crawler, so the index trends toward standalone and federation's share is measured and expected to fall.
- Keeping the local summariser the default; any external AI stays optional, flagged, and off by default.

## Alternatives

| Option | Why not |
|---|---|
| Offline enrichment only (federate on the ingestion plane, never at query time) | The conservative reading of ADR-0001, and genuinely safer — but it cannot help the query *in front of the user right now*; recall gains wait a full crawl cycle. Chosen against deliberately: the point is to answer better immediately while the corpus catches up. It remains the fallback if the live path proves too costly. |
| Let `xustive-api` call SearXNG/engines directly | Demolishes the no-egress property outright and gives the serving plane a general internet route — exactly what ADR-0001 forbids and what the gateway exists to avoid. |
| Call Google/Bing directly, no SearXNG | Reinvents metasearch, multiplies rate-limit and fingerprinting problems, and forgoes the free aggregation an open-source project already solved. SearXNG *is* the connector. |
| Make an external LLM (Parallel-AI) the default summariser | Sends document and query text to third-party SaaS on every summary and contradicts [[ADR-0005 - Local Quantised LLM for Summaries]]. Kept as an opt-in, offline-preferred alternative only. |
| Stay local-only, just fix retrieval | The honest long game, and Milestone 7 does pursue it (lexical, semantic, term-linking). But it leaves today's queries under-answered while the corpus and analyzers mature, and forgoes free recall and a self-feeding discovery channel. |

## Revisit when

- Federation's measured contribution to result pages falls below a floor (the index now answers on its own) — demote live federation to offline enrichment and save the latency budget.
- An external tool's rate-limiting or terms make live use unreliable or non-compliant — fall back to the offline-only path.
- A multi-user or hosted deployment appears — re-audit that query terms transiting the gateway carry no identifier, and that the egress allowlist cannot widen.
- Semantic recall (M7-T05) closes the word-mismatch gap on its own — reassess whether live federation still earns its cost.

## Related

[[ADR-0001 - Two-Plane Architecture]] · [[ADR-0013 - Direct SERP Collection for Discovery]] · [[ADR-0008 - No Query Logging]] · [[ADR-0005 - Local Quantised LLM for Summaries]] · [[Federation Gateway]] · [[Query Pipeline]] · [[Crawler Orchestrator]] · [[Ranking and Relevance]] · [[Security and Privacy]] · [[Milestone 7 - Federated Retrieval and External Tools]]
