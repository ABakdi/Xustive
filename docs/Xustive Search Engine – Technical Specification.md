---
tags:
  - xustive
  - overview
type: overview
status: living
updated: 2026-08-27
---

# Xustive Search Engine – Technical Specification

> [!info] This is the overview document
> It states *what* Xustive is and *which* technologies were chosen. The detailed design lives in
> linked notes: start at **[[Home]]**, then [[System Architecture]] for how it fits together,
> [[Component Map]] for the component inventory, [[UI Specification]] for the interface, and
> [[TODO]] for the implementation plan.
>
> Where this document and a detailed note disagree, **the detailed note is more current** — it
> reflects decisions made since, recorded in [[Decision Log]].
>
> §3 keeps the original technology choices of 2026-02 for the record and marks, in each row, what
> was actually built. §3.7 is the as-built inventory as of 2026-08-27 and is the part to trust.

---

## 1. Overview

**Xustive** is a self‑hosted search engine designed for the Algerian digital ecosystem. It indexes public web content and public social media posts (Facebook groups, Instagram profiles, TikTok videos) to provide fast, relevant results for queries in Arabic, French, English, and Darija (local Algerian dialect).

As of 2026-08-27 the open-web half is built and running (crawl, index, search, summaries,
instant answers, voice and image input, federation, entity panels, image/video verticals). The
social-platform connectors are specified ([[Social Connector - Facebook]] and siblings) but **not
built**; the collection machinery they will share (sessions, fingerprints, proxies) exists in
`xustive-ingest`.

All components are **100% open-source and free** under permissive licenses (MIT/Apache-2.0/BSD).

---

## 2. Functional Requirements

### 2.1. Search Interface

> Detail: [[UI - Home Page]] · [[UI - Voice Search]] · [[UI - Image Search]] · [[Language Detector]]

| Requirement | Description |
|:---|:---|
| **Text Search** | Single search bar supporting Arabic, Darija, French, and English input. Auto‑complete suggestions appear as user types. |
| **Voice Search** | Microphone button activates local speech‑to‑text. Transcribes speech in Darija, Arabic, French, or English and populates the search bar — editable, never auto-submitted. Built: `POST /api/v1/transcribe` → `stt-sidecar`, with live partials while speaking. |
| **Image Search** | Upload or capture an image. System extracts text via OCR (if present) OR performs reverse image similarity search against indexed image data. Built: `POST /api/v1/ocr` and `POST /api/v1/search/image` (CLIP → Qdrant). |
| **Instant answers** | Added by [[Milestone 1B - Frontend and Instant Answers]] and [[Milestone 8 - The Answer Layer]]: calculator, units, dates, prayer times, fuel, exam dates, wilaya, currency, weather, translate, transliterate, plus an entity panel and list answers ([[Instant Answers]], [[ADR-0019 - The Knowledge Layer]]). |
| **Verticals** | `?v=all\|news\|images\|videos` — saved filters over one index, with optional live federation per category ([[UI - Search Verticals]], [[Milestone 9 - Images and Videos]]). |
| **Language Detection** | Automatically detects the query language to apply appropriate tokenization and stemming. |

### 2.2. Results Display

> Detail: [[UI - Results Page]] · [[Summarizer]] · [[UI - Filters and Facets]] · [[Ranking and Relevance]]

| Requirement | Description |
|:---|:---|
| **AI Summary** | At the top of results, a 2‑3 sentence summary synthesising the most relevant, recent information from the top results. |
| **Clickable Links** | Below the summary, a paginated list (20 per page) of source links. Each entry includes: title (hyperlinked), description excerpt, source platform badge (web/Facebook/Instagram/TikTok), post timestamp, and sentiment indicator (positive/neutral/negative). |
| **Faceted Filtering** | Users can filter results by: date range, source platform, sentiment, language and media type. |
| **Highlighting** | Search keywords are highlighted in result descriptions. |

### 2.3. Data Ingestion

> Detail: [[Crawler Orchestrator]] · [[Web Fetcher]] · [[Content Parser]] · [[Enrichment Pipeline]] ·
> [[Deduplication Service]] · [[Sentiment Engine]] · social connectors
> ([[Social Connector - Facebook]], [[Social Connector - Instagram]], [[Social Connector - TikTok]])
>
> Social content is collected directly via the collection layer — [[Session Manager]],
> [[Fingerprint Engine]], [[Signature Service]] — per
> [[ADR-0009 - Direct Collection for Social Platforms]]. Obligations that survive that decision
> (deletion propagation, takedowns, no profiling, Law 18-07) are in [[Legal and Compliance]].

| Requirement | Description |
|:---|:---|
| **Web Crawling** | Automatically discovers and fetches public Algerian web pages. |
| **Social Media Scraping** | Fetches public posts from Facebook public groups, Instagram public profiles, and TikTok public videos. **Not built** as of 2026-08-27. |
| **Date Stamping** | Records both the original post date and the crawl/index date for every piece of content. |
| **Comment Extraction** | Extracts and indexes comments alongside main posts (the `comments` index exists; it fills only once social connectors land). |
| **Sentiment Analysis** | Assigns a sentiment score to each post and comment (positive/neutral/negative). Built as a lexicon scorer; it is a facet, never a ranking signal. |
| **Discovery** | Seeds, sitemaps, outlinks, Common Crawl, optional Brave API, SERP collection ([[ADR-0013 - Direct SERP Collection for Discovery]]) and weak-coverage terms. |
| **Deduplication** | Prevents indexing the same content multiple times using content hashing. |

### 2.4. Data Sovereignty

> Detail: [[Security and Privacy]] · [[ADR-0008 - No Query Logging]] · [[Deployment Topology]]

| Requirement | Description |
|:---|:---|
| **Self‑Hosted** | All services run on servers physically located within Algeria. |
| **No External Dependencies** | No user queries or crawled data are sent to external third‑party services. Enforced by network: the serving plane's `core` network is `internal: true` ([[ADR-0001 - Two-Plane Architecture]]). Two bounded exceptions, each recorded: query-time federation through the [[Federation Gateway]] when the operator enables it ([[ADR-0017 - Query-Time Federation with External Metasearch]]), and the web tier's entity/image fetches to Wikimedia ([[ADR-0014 - Knowledge Panel from Wikipedia via the Web Tier]]). |
| **Zero Query Logging** | User search history is not stored or logged ([[ADR-0008 - No Query Logging]]). Aggregate counters (interaction signals, weak coverage) are k-anonymous (k ≥ 20 outside dev) and off by default. |

---

## 3. Technology Stack

### 3.1. Frontend

| Component | Technology | License | Purpose |
|:---|:---|:---|:---|
| **Web UI** | HTML5 + Tailwind CSS + Vanilla JS | MIT | Provides responsive search interface and results display. **Superseded 2026-08** by Next.js 16 + React 19 + Tailwind 4 ([[ADR-0010 - Next.js for the Frontend]]). |
| **Speech‑to‑Text** | `whisper.cpp` (via Rust FFI) or `vosk-rs` | MIT / Apache-2.0 | Local, offline voice transcription. **Built as** `services/stt-sidecar`: Whisper `small` on faster-whisper (CTranslate2), CPU-capable, `base` as the live partial model. |
| **Image Processing** | `tesseract-rs` (OCR) + `image` crate | Apache-2.0 | Extracts text from uploaded images. **Built as** `leptess` (tesseract) + `image` in `xustive-media`, with an optional GPU `ocr-sidecar` (Unlimited-OCR) per [[ADR-0016 - Two OCR Engines with an Optional Unlimited-OCR Sidecar]]. |

### 3.2. Backend API

| Component | Technology | License | Purpose |
|:---|:---|:---|:---|
| **HTTP Server** | `Axum` (Rust web framework) | MIT | Handles all API requests (search, upload, voice, etc.). |
| **Request Routing** | `Tower` middleware | MIT | Adds logging, CORS, and rate limiting. |
| **Language Detection** | `lingua-rs` | Apache-2.0 | Identifies query language. **Built as** script rules first, `whatlang` only for genuine ambiguity, in `xustive-lang::detect`. |
| **Query Rewriting** | `DziriBERT` (on‑device) | MIT | Expands Darija/Arabizi variants. **Built as** a lexicon + transliteration + light morphology expander in `xustive-lang`; no model. DziriBERT not built. |

### 3.3. Search Core

| Component | Technology | License | Purpose |
|:---|:---|:---|:---|
| **Primary Search Engine** | `Meilisearch` (community edition) | MIT | Provides sub‑50ms full‑text search with typo tolerance, faceted filtering, and native Arabic tokenization. Built: v1.13, indexes `documents`, `comments`, `knowledge`, addressed through an alias convention (`documents` → `documents_vN`). |
| **Backup / Advanced Index** | `Tantivy` (Rust library) | MIT / Apache-2.0 | Used for experimental ranking and custom tokenization if needed. **Not used**; re-ranking is done in `xustive-search::rank` over Meilisearch candidates. |
| **Vector Search (Images)** | `qdrant` or `tantivy` with vector support | Apache-2.0 | Stores and queries CLIP embeddings. Built: Qdrant v1.12 over REST (`xustive-vector`), collections `image_clip` (512-d) and `text_bge` (1024-d, semantic text search, off by default). |

### 3.4. Natural Language Processing (NLP)

| Component | Technology | License | Purpose |
|:---|:---|:---|:---|
| **Sentiment Analysis** | `vader-sentiment-rust` | MIT | Fast rule‑based sentiment scoring. **Built as** our own VADER-style scorer over four lexicons in `xustive-lang::sentiment`. |
| **Advanced Sentiment** | `rust-bert` (DistilBERT) | MIT / Apache-2.0 | Deep learning sentiment for nuanced Darija text. **Not built**; a transformer would dominate the ingestion budget. |
| **Summarisation** | `llama-cpp-rs` (quantised Mistral‑7B or Phi‑3‑mini) | MIT / Apache-2.0 | Generates the AI summary. **Built as** `llama-cpp-2` in `xustive-ml`; default model Qwen2.5-3B-Instruct Q4_K_M (non-commercial licence — swap for an Apache-2.0 size before commercial launch), also used for the translate card ([[ADR-0005 - Local Quantised LLM for Summaries]]). |
| **Image Embeddings** | `rust-bert` (CLIP model) | MIT / Apache-2.0 | 512‑d vectors for image similarity. **Built as** `services/clip-embed` (Python, CLIP ViT-B/32, CPU-capable). |
| **Text Embeddings** | — | Apache-2.0 | Added M7: `services/text-embed` (bge-m3, 1024-d) for semantic retrieval. |

### 3.5. Scraping & Ingestion

| Component | Technology | License | Purpose |
|:---|:---|:---|:---|
| **Task Queue** | `Redis` (via Rust client) | BSD | Decouples discovery from processing. Built: Redis Streams with consumer groups, reclaim and DLQ in `xustive-queue` ([[ADR-0006 - Redis Streams for the Ingestion Pipeline]]); a second Redis holds only anonymous signals. |
| **Headless Browser** | `obscura` (Rust) | Apache-2.0 | Lightweight headless browser for JavaScript‑rendered content. **Not built**; all fetching is static. |
| **Stealth Scraping** | `stealthscraper-rs` | MIT | HTTP‑level fingerprint spoofing. **Built in-house** as `xustive-ingest::fingerprint` (coherence checks) — no third-party crate; used by no connector yet. |
| **Proxy Management** | `stygian-proxy` | *(Check Cargo.toml)* | Proxy pools. **Built in-house** as `xustive-ingest::proxy` (pool, health, breaker, attribution, bandwidth, ladder). |
| **Platform API Client** | `scrapebadger-rust` (TikTok/Instagram endpoints) | *(Check Cargo.toml)* | **Not built.** Direction changed to [[ADR-0009 - Direct Collection for Social Platforms]]. |
| **Facebook API Client** | `facebook_api_rs` | MIT | **Not built** (ADR-0007 superseded by ADR-0009). |
| **Data Parsing** | `social_parser` | MIT | **Not built.** |
| **Web Scraping** | `reqwest` + `scrapling_fetch` | MIT / Apache-2.0 | Built: `reqwest` behind the `SafeUrl` guard, `scraper` for HTML, `pdf-extract` for PDFs, `quick-xml` for sitemaps — all in `xustive-ingest`. |
| **External data** | `xustive-toold` | — | Scheduled fetch of rates (open.er-api), weather (Open-Meteo) and Wikidata entities into Redis / the `knowledge` index, from the ingest network ([[Tool Data Plane]]). |
| **Federation** | SearXNG + `xustive-federator` | AGPL-3.0 (self-hosted) / — | Optional query-time metasearch for web, images and videos through one gateway ([[Federation Gateway]]). |

### 3.6. Infrastructure & Storage

| Component | Technology | License | Purpose |
|:---|:---|:---|:---|
| **Containerisation** | `Docker` | Apache-2.0 | Containerises all services for consistent deployment. |
| **Orchestration** | `docker-compose` (or Kubernetes) | Apache-2.0 | Built: `deploy/docker-compose.yml` (prod topology, no published ports) + `docker-compose.dev.yml` overlay; the Rust binaries and Next.js run on the host in dev (`make dev`). Kubernetes not used. |
| **Metrics** | `Prometheus` | Apache-2.0 | v3.1, scraping `/metrics`; alert rules in `deploy/prometheus`. |
| **Monitoring** | `Grafana` | AGPL-3.0 | 11.4, provisioned dashboards in `deploy/grafana`, port 3001 in dev. |
| **Logging** | `tracing` (Rust crate) | MIT | Structured logging; JSON in prod (`[telemetry] log_json`). `scripts/scan-logs.sh` asserts no query text appears. |
| **Location** | DB-IP lite via `maxminddb` | CC-BY | Wilaya-level only, in-process, never stored ([[ADR-0020 - Approximate Location from a Local Database]]). |

### 3.7. As built — inventory (2026-08-27)

**Crates** (`crates/`): `xustive-api` (Axum surface), `xustive-core` (config, errors, `SafeUrl`,
circuit breaker, source registry), `xustive-search` (Meilisearch client, settings, rank,
authority, eval), `xustive-text` (normalisation), `xustive-lang` (detect, expand, sentiment,
translit), `xustive-ingest` (fetch, robots, parse, frontier, dedup, enrichment, discovery, proxy,
session, fingerprint), `xustive-queue` (Redis Streams, DLQ, breaker), `xustive-ml` (llama.cpp
summariser/translator, device), `xustive-media` (OCR, pHash), `xustive-vector` (Qdrant),
`xustive-tools` (instant answers), `xustive-toold` (external data fetcher), `xustive-knowledge`
(entities, resolver, Wikidata), `xustive-federation` (SearXNG client), `xustive-federator`
(egress gateway), `xustive-cli` (migrate, crawl, crawld, worker, eval, calibrate, ab, mine,
pagerank, registry, serp-eval, discover, commoncrawl), `xustive-loadgen`.

**Sidecars** (`services/`, Python, each opt-in): `stt-sidecar` :8093, `ocr-sidecar` :8091 (GPU),
`clip-embed` :8092, `text-embed` :8094, plus `searxng` (config only, ingest network).

**Web** (`web/`, Next.js): pages `/[lang]`, `/[lang]/search`, `/[lang]/settings`,
`/[lang]/privacy`, `/[lang]/tools/ocr`; operator pages `/admin/*` (compute, config, crawler,
discovery, documents, evaluation, integrations, interaction, live, maintenance, media, queue,
sources, sources/health, weak-coverage) and `/bot`; server routes `/api/knowledge` (Wikipedia),
`/api/knowledge-live` (Wikidata → API render), `/api/knowledge-list` (SPARQL list answers),
`/api/wiki-image` (Wikimedia-only image proxy), `/api/thumb` (HMAC-signed thumbnail proxy,
[[ADR-0021 - Proxied Thumbnails with Signed URLs]]).

**Rust API** (`:8080`, all under `/api/v1`): `GET search` (`q`, `v`, filters, pagination),
`GET suggest`, `GET tools`, `GET languages`, `POST translate`, `POST summary` (streamed),
`POST interaction`, `GET knowledge`, `POST knowledge/render`, `POST knowledge/resolve-live`,
`POST ocr`, `POST search/image`, `POST transcribe`; `admin/*` behind `X-Admin-Key` (crawler
status/events/documents/channels/sources/enqueue/pause/registry/weak-coverage, queue + DLQ
replay/drop, status, config, eval, media, interaction, integrations, takedown, device,
politeness, log-level); `/healthz`, `/readyz`, `/metrics` at the root. Full shapes in
[[API Contract]].

**Stores** (dev ports): Meilisearch :7700 (`documents`, `comments`, `knowledge`), Qdrant :6333
(`image_clip`, `text_bge`), Redis :6390 (queue, frontier, tool cache, raw store), Redis :6391
(signals only). Prometheus :9090, Grafana :3001. Production publishes none of these.

**Networks**: `core` (internal, no egress: API, stores, sidecars), `ingest` (egress: crawl
processes, toold, searxng), `obs`. `federator` sits on both — the one bridge.

---

## 4. Data Flow

> Detail: [[System Architecture]] §3–4 · [[Task Queue]] · [[Data Model]]

### 4.1. Ingestion Pipeline

1. **Discovery**: seeds and the source registry, sitemaps, outlinks, Common Crawl, optional
   Brave / SERP collection, weak-coverage terms. (Social media APIs: not built.)
2. **Fetching** (`xustive-cli crawld`): `reqwest` through `SafeUrl`, robots and per-host
   politeness, bounded frontier, adaptive revisit ([[ADR-0011 - Adaptive Recrawl over Static Crawling]]).
   HTML and PDF only; no headless rendering.
3. **Parsing**: title, content, author, timestamp, media URLs, per-domain rules.
4. **Enrichment**: language, lexicon sentiment, topics, quality/spam, SimHash + exact-hash
   dedup, optional image OCR / pHash / CLIP and text embeddings.
5. **Queue**: the document goes on the Redis `index` stream ([[Task Queue]]).
6. **Indexing** (`xustive-cli worker`): consumer group pulls, batches, writes Meilisearch (and
   Qdrant when vectors are on); poison messages go to the DLQ, replayable from `/admin/queue`.

### 4.2. Search Flow

1. **User Input**: Text, voice, or image submitted via the Next.js frontend.
2. **Pre‑processing**:
   - Voice → `POST /api/v1/transcribe` → `stt-sidecar` (faster-whisper); text lands in the box.
   - Image → `POST /api/v1/ocr` (text) or `POST /api/v1/search/image` (CLIP → Qdrant ANN).
3. **Language Detection**: `xustive-lang::detect` (script rules, `whatlang` for ambiguity).
4. **Query Expansion**: lexicon / transliteration expansion in `xustive-lang::expand`.
5. **Search**: `GET /api/v1/search` → Meilisearch candidate pool (plus optional semantic leg via
   `text_bge`, plus optional federated hits through the [[Federation Gateway]] within budget).
6. **Ranking**: Meilisearch ranking rules, then `xustive-search::rank` re-ranks with freshness,
   authority, trust tier and (when enabled) interaction CTR. Sentiment never affects order.
7. **Instant answers**: in parallel, `GET /api/v1/tools` (computed cards) and
   `GET /api/v1/knowledge` (entity panel from the `knowledge` index, falling back to the web
   tier's `/api/knowledge-live`).
8. **Summarisation**: after results are on screen the frontend calls `POST /api/v1/summary`, which
   streams from the local model ([[ADR-0004 - Stream Summary Separately from Results]]).
9. **Rendering**: cards and panel, then results (20 per page), then the summary as it arrives.

---

## 5. Non‑Functional Requirements

> Authoritative numbers: **[[Performance Budgets]]** (this table is a summary; where they differ,
> Performance Budgets wins). See also [[Observability]] and [[Error Handling and Resilience]].

### 5.1. Performance

| Metric | Target |
|:---|:---|
| Search latency (excluding summary) | ≤ 200 ms |
| Summary generation | ≤ 2.5 seconds |
| Crawling throughput | ≥ 100 pages/posts per minute per worker |
| Image search | ≤ 500 ms |

### 5.2. Scalability

- Horizontal scaling: Add more crawling workers as needed.
- Vertical scaling: Upgrade CPU/RAM for faster summarisation and indexing.
- Meilisearch: Supports sharding and replication for large indexes.

### 5.3. Security

- TLS 1.3 for all network communication (terminated in front of the stack; not part of the compose).
- API keys required for crawler authentication. Built as `X-Admin-Key` on `/api/v1/admin/*`
  and a Meilisearch master key; the crawler is a host process on the ingest network, not an API
  client.
- No persistent query logs (`scripts/scan-logs.sh` and `lint-telemetry.sh` enforce it).
- Encryption at rest for indexes and vector stores — **not implemented** in the compose; volumes
  are plain. See [[Security and Privacy]].

### 5.4. Reliability

- Circuit breakers per host (`xustive-core::circuit`), per proxy (`xustive-ingest::proxy::breaker`)
  and on the indexer (`xustive-queue::breaker`).
- Retry logic with exponential backoff for failed fetches; DLQ after retries are exhausted.
- Health checks (`/healthz`, `/readyz`, each sidecar's `/health`) and Docker restart policies.
- A degradation ladder under load: searches narrow (fewer legs, smaller pool) before they fail
  ([[Error Handling and Resilience]]).

---

## 6. Implementation Phases

> Superseded in detail by **[[TODO]]** and the milestone notes, which break these phases into tasks
> and subtasks with exit gates: [[Milestone 0 - Foundations]] (complete) ·
> [[Milestone 1 - Text Search MVP]] · [[Milestone 1B - Frontend and Instant Answers]] ·
> [[Milestone 2 - Ingestion at Scale]] · [[Milestone 3 - Multimodal Input]] ·
> [[Milestone 4 - Quality and Operations]] · [[Milestone 5 - Beta Launch]] ·
> [[Milestone 6 - Adaptive Ranking from Interaction Signals]] (done 2026-08-22, off by default) ·
> [[Milestone 7 - Federated Retrieval and External Tools]] (closed 2026-08-25) ·
> [[Milestone 8 - The Answer Layer]] (closed 2026-08-26) ·
> [[Milestone 9 - Images and Videos]] (closed 2026-08-26). Phase 3 below (social sources) has not
> started.
>
> Note the sequencing change: legal review and platform API applications now start during Milestone 1,
> because they gate Milestone 3 and take months ([[Legal and Compliance]] §8).

| Phase | Duration | Deliverables |
|:---|:---|:---|
| **Phase 0** | 2 weeks | Deploy Meilisearch, build basic frontend search bar, index 10k sample articles. |
| **Phase 1** | 2 weeks | Implement text search pipeline, integrate sentiment analysis and summarisation (LLM). |
| **Phase 2** | 2 weeks | Add voice and image input modalities. |
| **Phase 3** | 4 weeks | Deploy scraping workers with proxies, connect to Redis queue, index social media sources. |
| **Phase 4** | 4 weeks | Optimise summarisation, add faceted filtering, implement monitoring (Prometheus/Grafana). |
| **Phase 5** | Ongoing | Beta launch, community feedback, add "Submit a Source" feature. |

---

## 7. Open Source Licenses Summary

| Component | License |
|:---|:---|
| Axum, Tokio, Tower | MIT |
| Meilisearch (Community) | MIT |
| Qdrant | Apache-2.0 |
| `llama-cpp-2`, `fend-core`, `whatlang`, `scraper`, `reqwest` | MIT / Apache-2.0 / GPL-3.0 (`fend-core`) |
| faster-whisper, CLIP ViT-B/32, bge-m3 | MIT / MIT / Apache-2.0 |
| Qwen2.5-3B-Instruct (default summariser) | **Qwen-Research, non-commercial** |
| Unlimited-OCR (optional sidecar) | MIT |
| Redis | BSD / RSALv2 (7.x) |
| SearXNG, Grafana | AGPL-3.0 (self‑hosted) |
| DB-IP lite | CC-BY 4.0 |
| Tantivy, `obscura`, `stealthscraper-rs`, `rust-bert`, `vader-sentiment-rust` | not used (see §3) |

Everything is free to run self-hosted. Two licence caveats before a commercial launch: the
default 3B summariser must be swapped for an Apache-2.0 Qwen size (1.5B/7B), and `fend-core` is
GPL-3.0 — verify (⚖ VERIFY, [[Legal and Compliance]]).

---

This specification provides a complete technical blueprint for building **Xustive**. All functional requirements are mapped to specific open‑source technologies, with clear explanations of what each component does and why it was chosen.

---

## 8. Where to Go Next

| Question | Note |
|:---|:---|
| How does it all fit together? | [[System Architecture]] |
| What are the components? | [[Component Map]] |
| What does the data look like? | [[Data Model]] |
| What does the API return? | [[API Contract]] |
| What does it look like? | [[UI Specification]] |
| Why was X chosen? | [[Decision Log]] |
| What do we build first? | [[TODO]] |
| What does this word mean? | [[Glossary]] |
| Everything | [[Home]] |

> [!note] Changes since this document was written
> The detailed notes revise a few choices made here. The significant ones:
> - **Social content is collected directly**, per [[ADR-0009 - Direct Collection for Social Platforms]].
>   The stealth-scraping intent of §3.5 is retained and expanded into a proper design: a collection
>   layer of [[Session Manager]], [[Fingerprint Engine]], and [[Signature Service]], with
>   [[Proxy Manager]] rebuilt for residential/mobile pools. Open-web crawling remains fully polite
>   and is governed by a separate crawl profile ([[Politeness and Robots]] §4.0).
> - **The AI summary streams separately from results**, so search latency is never gated on the LLM
>   ([[ADR-0004 - Stream Summary Separately from Results]]).
> - **Comments live in their own index**, not nested in documents
>   ([[ADR-0003 - Comments in a Separate Index]]).
> - **There is no separate database** — Meilisearch is the system of record
>   ([[ADR-0002 - Meilisearch as System of Record]]).
> - **Qdrant is the vector store**, chosen over Tantivy vectors ([[Vector Index]]).
> - **The frontend is Next.js**, not vanilla JS ([[ADR-0010 - Next.js for the Frontend]]), and the
>   web tier is the one place allowed to fetch Wikimedia for the entity panel and thumbnails
>   ([[ADR-0014 - Knowledge Panel from Wikipedia via the Web Tier]],
>   [[ADR-0021 - Proxied Thumbnails with Signed URLs]]).
> - **Live data reaches a no-egress serving plane through `xustive-toold`** and, when enabled,
>   **live web results through one gateway** ([[Tool Data Plane]], [[Federation Gateway]]).
> - **An entity store (`knowledge` index) and instant answers** were added by M8
>   ([[ADR-0019 - The Knowledge Layer]]); **image and video verticals** by M9.
> - **Ranking may learn from anonymous clicks**, k-anonymous and off by default
>   ([[ADR-0015 - Anonymous Interaction Signals for Ranking]]).
> - **Social connectors remain unbuilt** as of 2026-08-27; the crawl-side collection machinery
>   exists.