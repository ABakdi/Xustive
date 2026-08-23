---
tags:
  - adr
  - moc
type: index
status: living
updated: 2026-08-06
---

# Decision Log

> Architecture Decision Records. One note per decision that was **hard to make and expensive to
> reverse**. Easy or reversible choices belong in the component note, not here.
> Parent: [[Home]]

---

## 1. Index

| ID | Decision | Status | Affects |
|:---|:---|:---|:---|
| [[ADR-0010 - Next.js for the Frontend]] | React Server Components; `xustive-api` stops rendering HTML | accepted | [[UI - Frontend Architecture]], [[UI - Design Language]] |
| [[ADR-0011 - Adaptive Recrawl over Static Crawling]] | recrawl on content longevity, not change rate; abandon volatile pages | accepted | [[Crawler Orchestrator]], [[Web Fetcher]], [[Deduplication Service]] |
| [[ADR-0012 - Discovery-Only Aggregation]] | external engines discover URLs; never called on the serving path | **superseded by 0013** | [[Crawler Orchestrator]], [[Query Pipeline]], [[API Gateway]] |
| [[ADR-0013 - Direct SERP Collection for Discovery]] | query Google directly for discovery, in the ingestion plane only, as the narrowest channel | accepted | [[Crawler Orchestrator]], [[Proxy Manager]], [[Session Manager]], [[Fingerprint Engine]] |
| [[ADR-0001 - Two-Plane Architecture]] | Split serving and ingestion, coupled only through the index | accepted | [[System Architecture]] |
| [[ADR-0002 - Meilisearch as System of Record]] | No separate database; the index is the store | accepted | [[Search Index]], [[Data Model]] |
| [[ADR-0003 - Comments in a Separate Index]] | Comments are their own index, folded in at query time | accepted | [[Data Model]], [[Query Pipeline]] |
| [[ADR-0004 - Stream Summary Separately from Results]] | Two requests: results first, summary streamed after | accepted | [[API Contract]], [[UI - Results Page]] |
| [[ADR-0005 - Local Quantised LLM for Summaries]] | 3B quantised model on CPU, no external API | accepted | [[Summarizer]] |
| [[ADR-0006 - Redis Streams for the Ingestion Pipeline]] | Streams + consumer groups, not lists or a broker | accepted | [[Task Queue]] |
| [[ADR-0007 - API-First Social Access]] | No scraping fallback exists in the code | **superseded by 0009** | social connectors, [[Legal and Compliance]] |
| [[ADR-0008 - No Query Logging]] | Zero query retention, enforced structurally | accepted | [[Security and Privacy]], [[Observability]] |
| [[ADR-0009 - Direct Collection for Social Platforms]] | Direct collection is a first-class path; adds the collection layer | accepted | social connectors, [[Session Manager]], [[Fingerprint Engine]], [[Signature Service]], [[Proxy Manager]] |
| [[ADR-0015 - Anonymous Interaction Signals for Ranking]] | k-anonymous interaction counters feed ranking and re-crawl; default off | accepted | [[Ranking and Relevance]], [[Query Pipeline]], [[Interaction Signals]], [[Observability]] |
| [[ADR-0016 - Two OCR Engines with an Optional Unlimited-OCR Sidecar]] | dual OCR backends, optional Unlimited-OCR sidecar | accepted | OCR / multimodal ingest |
| [[ADR-0017 - Query-Time Federation with External Metasearch]] | live web federation through one allowlisted gateway; serving-plane no-egress kept as one hop; converge to standalone | accepted | [[Federation Gateway]], [[Query Pipeline]], [[Crawler Orchestrator]], [[Ranking and Relevance]] |

---

## 2. Open Decisions

Decisions we know we must make, with the milestone that forces them.

| Question | Forced by | Notes |
|:---|:---|:---|
| Which residential/mobile proxy provider? DZ coverage, ≥ 4 ASNs, exit-node consent | [[Milestone 2 - Ingestion at Scale]] | [[Proxy Manager]] §12 |
| Which JS runtime for signer execution — `deno_core`, `rusty_v8`, `boa`? | [[Milestone 2 - Ingestion at Scale]] | [[Signature Service]] §12 — prototype against a real bundle first |
| Which impersonation library — `rquest` or alternatives? | [[Milestone 2 - Ingestion at Scale]] | [[Fingerprint Engine]] §12 — validate JA4 accuracy before committing |
| Do we join closed groups with our identities? | [[Milestone 2 - Ingestion at Scale]] | default no; per-source, operator-performed ([[Social Connector - Facebook]] §4.3) |
| Is the embedded-JSON path alone enough for IG/TikTok monitoring? | [[Milestone 2 - Ingestion at Scale]] | would make collection near signature-independent — measure early |
| Store `translit_body` (index +35 %) or transliterate at query time? | [[Milestone 1 - Text Search MVP]] | [[Data Model]] §9 |
| One Meilisearch node at 10M docs, or a read replica? | [[Milestone 4 - Quality and Operations]] | [[Performance Budgets]] §10 — needs a real load test |
| Is a 3B model good enough for Arabic synthesis? | [[Milestone 1 - Text Search MVP]] | [[Summarizer]] §12 — decided by the faithfulness gate |
| Proxy remote thumbnails through our own server? | [[Milestone 5 - Beta Launch]] | [[Security and Privacy]] §9 — leaning yes |
| Enable aggregate popularity for autocomplete? | [[Milestone 3 - Multimodal Input]] | [[Autocomplete Service]] §12 — leaning no |
| Sentiment transformer mode: where does labelled data come from? | [[Milestone 4 - Quality and Operations]] | [[Sentiment Engine]] §12 |
| What happens if Facebook group access is unobtainable? | [[Milestone 2 - Ingestion at Scale]] | [[Legal and Compliance]] §9 — a product-strategy question |
| Service worker for static assets only? | [[Milestone 5 - Beta Launch]] | [[UI - States and Errors]] §8 |

---

## 3. Template

```markdown
---
tags: [adr]
adr-id: NNNN
status: proposed | accepted | superseded | rejected
date: YYYY-MM-DD
---
# ADR-NNNN - <Title>

## Status
## Context          — the forces, constraints, and what we knew at the time
## Decision         — what we chose, stated plainly
## Consequences     — good, bad, and what this now commits us to
## Alternatives     — what else we considered, and why not
## Revisit when     — the concrete signal that should reopen this
```

The **Revisit when** section is the one that keeps a decision log useful. A decision without a
stated trip-wire quietly becomes an assumption nobody remembers making.

---

## 4. Conventions

- ADRs are **immutable once accepted**. Changing your mind means writing a new ADR that supersedes
  the old one, and updating the old one's `status` to `superseded` with a link.
- Number sequentially, never reuse.
- Link the ADR from the components it constrains, and link back from the ADR.
- A decision that a component note describes as "deliberate", "on purpose", or "we chose not to"
  should either have an ADR or be demoted to a plain implementation detail.

## Related

[[Home]] · [[System Architecture]] · [[Component Map]] · [[TODO]]
