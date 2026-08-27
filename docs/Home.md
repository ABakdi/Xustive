---
tags:
  - moc
  - xustive
type: map-of-content
status: living
updated: 2026-08-27
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
| Just want to run it | **[[Running Xustive]]** |
| About to write code | [[TODO]] → [[Running Xustive]] → the component note you own |
| On call / it broke | [[Runbooks]] → [[Problems]] |
| Designing screens | [[UI Specification]] |
| Reviewing decisions | [[Decision Log]] |
| Confused by a term | [[Glossary]] |

---

## 📍 Where the code is (2026-08-27)

Seventeen Rust crates under `crates/`, one Next.js app under `web/`, five sidecars under
`services/`. The mapping from crate to note, so a reader can go either way:

| Crate | What it is | Note |
|:---|:---|:---|
| `xustive-api` | Axum HTTP surface, `/api/v1/*`, rate limits, admin | [[API Gateway]] · [[API Contract]] |
| `xustive-core` | config, error taxonomy, `SafeUrl`, circuit breaker, registry | [[Security and Privacy]] |
| `xustive-search` | Meilisearch client, index settings, operators, authority | [[Search Index]] |
| `xustive-text` · `xustive-lang` | normalisation; detection, expansion, sentiment | [[Language Detector]] · [[Query Expander]] · [[Sentiment Engine]] |
| `xustive-ingest` | fetch, robots, parse, frontier, dedup, enrichment, SERP, proxy/session/fingerprint | [[Crawler Orchestrator]] · [[Web Fetcher]] · [[Content Parser]] |
| `xustive-queue` | Redis Streams: produce, consume-group, ack, reclaim, DLQ | [[Task Queue]] |
| `xustive-ml` | llama.cpp summariser/translator, device selection | [[Summarizer]] |
| `xustive-media` · `xustive-vector` | OCR + pHash; Qdrant CLIP/text ANN | [[Image Pipeline]] · [[Vector Index]] |
| `xustive-tools` · `xustive-toold` | instant answers; scheduled fetch of rates/weather/entities | [[Instant Answers]] · [[Tool Data Plane]] |
| `xustive-knowledge` | entity model, `knowledge` index, resolver, Wikidata parser | [[ADR-0019 - The Knowledge Layer]] |
| `xustive-federation` · `xustive-federator` | SearXNG client; the one egress gateway | [[Federation Gateway]] |
| `xustive-cli` · `xustive-loadgen` | operator commands (migrate, crawl, eval…); load generator | [[Running Xustive]] · [[Performance Budgets]] |

Sidecars: `stt-sidecar` (faster-whisper), `ocr-sidecar` (Unlimited-OCR, GPU), `clip-embed`,
`text-embed` (bge-m3), `searxng`. See [[Deployment Topology]].

## 🧭 Architecture

- [[System Architecture]] — layers, boundaries, request/ingest paths
- [[Component Map]] — the full component inventory and dependency edges
- [[Data Model]] — canonical `Document`, `Comment`, `Media`, `Source`, `Entity` schemas
- [[API Contract]] — every HTTP endpoint, request/response shapes, error codes
- [[Ranking and Relevance]] — scoring, freshness decay, authority, interaction signals
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
- [[Search Index]] — Meilisearch: `documents`, `comments`, `knowledge`; settings, ranking rules
- [[Vector Index]] — Qdrant: CLIP image embeddings and bge-m3 text embeddings, ANN search
- [[Summarizer]] — local LLM producing the 2–3 sentence answer
- [[Autocomplete Service]] — as-you-type suggestions
- [[Speech to Text]] — offline voice transcription (faster-whisper sidecar)
- [[Image Pipeline]] — OCR + CLIP embedding + reverse image search
- [[Instant Answers]] — calculator, units, dates, prayer times, fuel, exams, wilaya, currency,
  weather, translate, transliterate
- [[Tool Data Plane]] — how live data (rates, weather, entities) reaches a no-egress serving plane
- [[Federation Gateway]] — one allowlisted egress hop for live web/images/videos federation
  ([[ADR-0017 - Query-Time Federation with External Metasearch]])
- [[Interaction Signals]] — anonymous click signals feeding ranking
  ([[ADR-0015 - Anonymous Interaction Signals for Ranking]]); built, off by default

### Ingestion path
- [[Crawler Orchestrator]] — frontier, scheduling, budgets, adaptive revisit
- [[Web Fetcher]] — static HTML/PDF retrieval (headless fetch is not built)
- [[Content Parser]] — HTML/JSON → canonical `Document`
- [[Enrichment Pipeline]] — sentiment, entities, embeddings, quality
- [[Sentiment Engine]] — lexicon sentiment scoring (transformer tier not built)
- [[Deduplication Service]] — exact + near-duplicate suppression
- [[Indexer Worker]] — batched writes into [[Search Index]] / [[Vector Index]]
- [[Crawler Console]] — the operator's view of the crawler (`/admin/*` pages)
- [[Social Connector - Facebook]] · [[Social Connector - Instagram]] ·
  [[Social Connector - TikTok]] — specified, **not built** as of 2026-08-27

### Collection layer
> Added by [[ADR-0009 - Direct Collection for Social Platforms]] — direct collection for social
> platforms. Open-web crawling is unaffected and stays fully polite. The shared machinery
> (`xustive-ingest::{session, fingerprint, proxy}`) exists; no platform connector uses it yet.

- [[Session Manager]] — identity pool, cookies, budgets, challenge handling
- [[Fingerprint Engine]] — coherent TLS / HTTP2 / header / browser fingerprints
- [[Signature Service]] — platform request signing (X-Bogus, msToken, fb_dtsg) — not built

### Platform
- [[Task Queue]] — Redis streams, queue topology, DLQ
- [[Proxy Manager]] — residential/mobile pools, pinning, ban attribution, cost
- [[Politeness and Robots]] — two crawl profiles: polite open web, platform collection
- [[Admin and Source Submission]] — source registry + moderation

## 🎨 User Interface

- [[UI Specification]] — the UI hub note
- [[UI - Frontend Architecture]] — Next.js, rendering, i18n
- [[UI - Design Language]] · [[UI - Component Library]]
- [[UI - Home Page]] · [[UI - Results Page]] · [[UI - Search Verticals]]
- [[UI - Tool Cards]] — instant-answer rendering
- [[UI - Voice Search]] · [[UI - Image Search]] · [[UI - Filters and Facets]]
- [[UI - RTL and Localization]] · [[UI - Accessibility]] · [[UI - States and Errors]]
- [[UI - Admin Console]] — the operator pages under `/admin`

## 🛠️ Engineering Practice

- **[[Running Xustive]]** — one command to run everything, the ports, and what to do when it breaks
- [[Runbooks]] — operational procedures and alert responses
- [[Testing Strategy]] — unit → integration → relevance evaluation
- [[Performance Budgets]] — the numbers every PR is measured against
- [[Legal and Compliance]] — robots, ToS risk, Law 18-07, takedowns
- [[Data Sources Registry]] — what we crawl and under which policy

## 🐛 Bugs and Problems

- [[Problems]] — the problems register (PROB-001…003, all solved by 2026-08-26)
- [[2026-08-25 - Code Audit Findings]] — the post-M7 audit and its 40 fixes
- Solutions: [[PROB-001 - Bounded Frontier and Queue]] ·
  [[PROB-002 - Crawl and Index Throughput]] · [[PROB-003 - Admin Console Coverage]]

## ⚖️ Decisions

[[Decision Log]] is the index. The records:
[[ADR-0001 - Two-Plane Architecture]] · [[ADR-0002 - Meilisearch as System of Record]] ·
[[ADR-0003 - Comments in a Separate Index]] ·
[[ADR-0004 - Stream Summary Separately from Results]] ·
[[ADR-0005 - Local Quantised LLM for Summaries]] ·
[[ADR-0006 - Redis Streams for the Ingestion Pipeline]] · [[ADR-0007 - API-First Social Access]] ·
[[ADR-0008 - No Query Logging]] · [[ADR-0009 - Direct Collection for Social Platforms]] ·
[[ADR-0010 - Next.js for the Frontend]] · [[ADR-0011 - Adaptive Recrawl over Static Crawling]] ·
[[ADR-0012 - Discovery-Only Aggregation]] · [[ADR-0013 - Direct SERP Collection for Discovery]] ·
[[ADR-0014 - Knowledge Panel from Wikipedia via the Web Tier]] ·
[[ADR-0015 - Anonymous Interaction Signals for Ranking]] ·
[[ADR-0016 - Two OCR Engines with an Optional Unlimited-OCR Sidecar]] ·
[[ADR-0017 - Query-Time Federation with External Metasearch]] ·
[[ADR-0018 - Anonymous Search History]] · [[ADR-0019 - The Knowledge Layer]] ·
[[ADR-0020 - Approximate Location from a Local Database]] ·
[[ADR-0021 - Proxied Thumbnails with Signed URLs]]

## 📋 Planning

- [[TODO]] — **the master implementation checklist**
- [[Milestone 0 - Foundations]] — complete
- [[Milestone 1 - Text Search MVP]] · [[Milestone 1B - Frontend and Instant Answers]]
- [[Milestone 2 - Ingestion at Scale]] — in progress
- [[Milestone 3 - Multimodal Input]] — in progress (OCR, image similarity, voice all wired)
- [[Milestone 4 - Quality and Operations]] — in progress
- [[Milestone 5 - Beta Launch]]
- [[Milestone 6 - Adaptive Ranking from Interaction Signals]] — done 2026-08-22; ships **off**
  by default (`[interaction] enabled`)
- [[Milestone 7 - Federated Retrieval and External Tools]] — closed 2026-08-25
- [[Milestone 8 - The Answer Layer]] — closed 2026-08-26 (entity panel, weather, currency,
  calculator, list answers)
- [[Milestone 9 - Images and Videos]] — closed 2026-08-26 (Images/Videos verticals, thumb proxy)
- [[Milestone 10 - Reverse Image Search]] — built 2026-08-27 (picture in, pictures out; ADR-0028); two gate items open

---

## Note conventions

- Frontmatter `tags` drive the tag pane: `component`, `ui`, `architecture`, `planning`, `adr`.
- Frontmatter `status`: `draft` → `specified` → `implemented` → `verified`.
- Every component note follows the same 12-section template — see [[Component Map]].
- Unresolved questions live in a `## Open Questions` section and are mirrored into [[Decision Log]].
