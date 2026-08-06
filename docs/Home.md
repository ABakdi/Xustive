---
tags:
  - moc
  - xustive
type: map-of-content
status: living
updated: 2026-08-06
---

# 🏠 Xustive — Documentation Home

**Xustive** is a self-hosted, Algeria-first search engine indexing public web content and public
social media posts, serving results in **Arabic, Darija, French, and English**.

> [!info] How to use this vault
> This is the entry point. Every note links to its neighbours — open the **graph view** to see the
> system topology, and use **backlinks** on any component note to find who depends on it.

---

## 🚦 Start Here

| If you are… | Read this |
|:---|:---|
| New to the project | [[Xustive Search Engine – Technical Specification]] → [[System Architecture]] |
| About to write code | [[TODO]] → [[Local Development]] → the component note you own |
| Designing screens | [[UI Specification]] |
| Reviewing decisions | [[Decision Log]] |
| Confused by a term | [[Glossary]] |

---

## 🧭 Architecture

- [[System Architecture]] — layers, boundaries, request/ingest paths
- [[Component Map]] — the full component inventory and dependency edges
- [[Data Model]] — canonical `Document`, `Comment`, `Media`, `Source` schemas
- [[API Contract]] — every HTTP endpoint, request/response shapes, error codes
- [[Ranking and Relevance]] — scoring, freshness decay, tie-breakers
- [[Deployment Topology]] — containers, networks, volumes, sizing
- [[Observability]] — metrics, logs, traces, dashboards, alerts
- [[Error Handling and Resilience]] — retries, circuit breakers, backpressure, DLQ
- [[Security and Privacy]] — threat model, zero query logging, encryption

## 🧩 Components

### Serving path
- [[API Gateway]] — HTTP surface, middleware, rate limiting
- [[Query Pipeline]] — orchestrates a single search request end-to-end
- [[Language Detector]] — ar / ary / fr / en / mixed classification
- [[Query Expander]] — Darija ↔ Arabizi ↔ MSA expansion
- [[Search Index]] — Meilisearch: schema, settings, ranking rules
- [[Vector Index]] — Qdrant: CLIP embeddings, ANN search
- [[Summarizer]] — local LLM producing the 2–3 sentence answer
- [[Autocomplete Service]] — as-you-type suggestions
- [[Speech to Text]] — offline voice transcription
- [[Image Pipeline]] — OCR + CLIP embedding + reverse image search

### Ingestion path
- [[Crawler Orchestrator]] — frontier, scheduling, budgets
- [[Web Fetcher]] — static + headless HTML retrieval
- [[Social Connector - Facebook]]
- [[Social Connector - Instagram]]
- [[Social Connector - TikTok]]
- [[Content Parser]] — HTML/JSON → canonical `Document`
- [[Enrichment Pipeline]] — sentiment, entities, embeddings, quality
- [[Sentiment Engine]] — lexicon + transformer sentiment scoring
- [[Deduplication Service]] — exact + near-duplicate suppression
- [[Indexer Worker]] — batched writes into [[Search Index]] / [[Vector Index]]

### Collection layer
> Added by [[ADR-0009 - Direct Collection for Social Platforms]] — direct collection for social
> platforms. Open-web crawling is unaffected and stays fully polite.

- [[Session Manager]] — identity pool, cookies, budgets, challenge handling
- [[Fingerprint Engine]] — coherent TLS / HTTP2 / header / browser fingerprints
- [[Signature Service]] — platform request signing (X-Bogus, msToken, fb_dtsg)

### Platform
- [[Task Queue]] — Redis streams, queue topology, DLQ
- [[Proxy Manager]] — residential/mobile pools, pinning, ban attribution, cost
- [[Politeness and Robots]] — two crawl profiles: polite open web, platform collection
- [[Admin and Source Submission]] — source registry + moderation

## 🎨 User Interface

- [[UI Specification]] — the UI hub note
- [[UI - Design System]] · [[UI - Component Library]]
- [[UI - Home Page]] · [[UI - Results Page]]
- [[UI - Voice Search]] · [[UI - Image Search]] · [[UI - Filters and Facets]]
- [[UI - RTL and Localization]] · [[UI - Accessibility]] · [[UI - States and Errors]]

## 🛠️ Engineering Practice

- [[Local Development]] — repo layout, toolchain, `make` targets
- [[Testing Strategy]] — unit → integration → relevance evaluation
- [[Performance Budgets]] — the numbers every PR is measured against
- [[Legal and Compliance]] — robots, ToS risk, Law 18-07, takedowns
- [[Data Sources Registry]] — what we crawl and under which policy

## 📋 Planning

- [[TODO]] — **the master implementation checklist**
- [[Milestone 0 - Foundations]]
- [[Milestone 1 - Text Search MVP]]
- [[Milestone 2 - Multimodal Input]]
- [[Milestone 3 - Ingestion at Scale]]
- [[Milestone 4 - Quality and Operations]]
- [[Milestone 5 - Beta Launch]]

---

## Note conventions

- Frontmatter `tags` drive the tag pane: `component`, `ui`, `architecture`, `planning`, `adr`.
- Frontmatter `status`: `draft` → `specified` → `implemented` → `verified`.
- Every component note follows the same 12-section template — see [[Component Map]].
- Unresolved questions live in a `## Open Questions` section and are mirrored into [[Decision Log]].
