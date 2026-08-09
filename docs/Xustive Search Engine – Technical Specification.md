---
tags:
  - xustive
  - overview
type: overview
status: living
updated: 2026-08-06
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

---

## 1. Overview

**Xustive** is a self‑hosted search engine designed for the Algerian digital ecosystem. It indexes public web content and public social media posts (Facebook groups, Instagram profiles, TikTok videos) to provide fast, relevant results for queries in Arabic, French, English, and Darija (local Algerian dialect).

All components are **100% open-source and free** under permissive licenses (MIT/Apache-2.0/BSD).

---

## 2. Functional Requirements

### 2.1. Search Interface

> Detail: [[UI - Home Page]] · [[UI - Voice Search]] · [[UI - Image Search]] · [[Language Detector]]

| Requirement | Description |
|:---|:---|
| **Text Search** | Single search bar supporting Arabic, Darija, French, and English input. Auto‑complete suggestions appear as user types. |
| **Voice Search** | Microphone button activates local speech‑to‑text. Transcribes speech in Darija, Arabic, French, or English and populates the search bar. |
| **Image Search** | Upload or capture an image. System extracts text via OCR (if present) OR performs reverse image similarity search against indexed image data. |
| **Language Detection** | Automatically detects the query language to apply appropriate tokenization and stemming. |

### 2.2. Results Display

> Detail: [[UI - Results Page]] · [[Summarizer]] · [[UI - Filters and Facets]] · [[Ranking and Relevance]]

| Requirement | Description |
|:---|:---|
| **AI Summary** | At the top of results, a 2‑3 sentence summary synthesising the most relevant, recent information from the top results. |
| **Clickable Links** | Below the summary, a paginated list (20 per page) of source links. Each entry includes: title (hyperlinked), description excerpt, source platform badge (web/Facebook/Instagram/TikTok), post timestamp, and sentiment indicator (positive/neutral/negative). |
| **Faceted Filtering** | Users can filter results by: date range, source platform, and sentiment. |
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
| **Social Media Scraping** | Fetches public posts from Facebook public groups, Instagram public profiles, and TikTok public videos. |
| **Date Stamping** | Records both the original post date and the crawl/index date for every piece of content. |
| **Comment Extraction** | Extracts and indexes comments alongside main posts. |
| **Sentiment Analysis** | Assigns a sentiment score to each post and comment (positive/neutral/negative). |
| **Deduplication** | Prevents indexing the same content multiple times using content hashing. |

### 2.4. Data Sovereignty

> Detail: [[Security and Privacy]] · [[ADR-0008 - No Query Logging]] · [[Deployment Topology]]

| Requirement | Description |
|:---|:---|
| **Self‑Hosted** | All services run on servers physically located within Algeria. |
| **No External Dependencies** | No user queries or crawled data are sent to external third‑party services. |
| **Zero Query Logging** | User search history is not stored or logged. |

---

## 3. Technology Stack

### 3.1. Frontend

| Component | Technology | License | Purpose |
|:---|:---|:---|:---|
| **Web UI** | HTML5 + Tailwind CSS + Vanilla JS | MIT | Provides responsive search interface and results display. |
| **Speech‑to‑Text** | `whisper.cpp` (via Rust FFI) or `vosk-rs` | MIT / Apache-2.0 | Local, offline voice transcription. Supports multilingual input. |
| **Image Processing** | `tesseract-rs` (OCR) + `image` crate | Apache-2.0 | Extracts text from uploaded images for text‑based search. |

### 3.2. Backend API

| Component | Technology | License | Purpose |
|:---|:---|:---|:---|
| **HTTP Server** | `Axum` (Rust web framework) | MIT | Handles all API requests (search, upload, voice, etc.). |
| **Request Routing** | `Tower` middleware | MIT | Adds logging, CORS, and rate limiting. |
| **Language Detection** | `lingua-rs` | Apache-2.0 | Identifies query language for appropriate processing. |
| **Query Rewriting** | `DziriBERT` (on‑device) | MIT | Expands Darija/Arabizi variants to improve recall. |

### 3.3. Search Core

| Component | Technology | License | Purpose |
|:---|:---|:---|:---|
| **Primary Search Engine** | `Meilisearch` (community edition) | MIT | Provides sub‑50ms full‑text search with typo tolerance, faceted filtering, and native Arabic tokenization. |
| **Backup / Advanced Index** | `Tantivy` (Rust library) | MIT / Apache-2.0 | Used for experimental ranking and custom tokenization if needed. |
| **Vector Search (Images)** | `qdrant` or `tantivy` with vector support | Apache-2.0 | Stores and queries CLIP embeddings for reverse image search. |

### 3.4. Natural Language Processing (NLP)

| Component | Technology | License | Purpose |
|:---|:---|:---|:---|
| **Sentiment Analysis** | `vader-sentiment-rust` | MIT | Fast rule‑based sentiment scoring for Darija/Arabic/French text. |
| **Advanced Sentiment** | `rust-bert` (DistilBERT) | MIT / Apache-2.0 | Deep learning sentiment for nuanced Darija text. |
| **Summarisation** | `llama-cpp-rs` (quantised Mistral‑7B or Phi‑3‑mini) | MIT / Apache-2.0 | Generates the 2‑3 sentence AI summary at the top of search results. |
| **Image Embeddings** | `rust-bert` (CLIP model) | MIT / Apache-2.0 | Generates 512‑dimension vectors for image similarity search. |

### 3.5. Scraping & Ingestion

| Component | Technology | License | Purpose |
|:---|:---|:---|:---|
| **Task Queue** | `Redis` (via Rust client) | BSD | Decouples discovery from processing. Enables horizontal scaling of crawlers. |
| **Headless Browser** | `obscura` (Rust) | Apache-2.0 | Lightweight headless browser for JavaScript‑rendered content (Instagram, etc.). |
| **Stealth Scraping** | `stealthscraper-rs` | MIT | HTTP‑level fingerprint spoofing (TLS, JA4/JA3) to avoid bot detection. |
| **Proxy Management** | `stygian-proxy` | *(Check Cargo.toml)* | Manages proxy pools: rotation, health checks, circuit breakers. |
| **Platform API Client** | `scrapebadger-rust` (TikTok/Instagram endpoints) | *(Check Cargo.toml)* | Dedicated endpoints for TikTok video metadata, comments, user info. |
| **Facebook API Client** | `facebook_api_rs` | MIT | Official Graph API client for Facebook public pages/groups. |
| **Data Parsing** | `social_parser` | MIT | Parses exported social media data archives (if users upload their data). |
| **Web Scraping** | `reqwest` + `scrapling_fetch` | MIT / Apache-2.0 | HTTP client for static web pages with proxy support. |

### 3.6. Infrastructure & Storage

| Component | Technology | License | Purpose |
|:---|:---|:---|:---|
| **Containerisation** | `Docker` | Apache-2.0 | Containerises all services for consistent deployment. |
| **Orchestration** | `docker-compose` (or Kubernetes) | Apache-2.0 | Manages multi‑container deployments. |
| **Metrics** | `Prometheus` | Apache-2.0 | Collects system and application metrics. |
| **Monitoring** | `Grafana` | AGPL-3.0 | Visualises metrics dashboards. |
| **Logging** | `tracing` (Rust crate) | MIT | Structured JSON logging. |

---

## 4. Data Flow

> Detail: [[System Architecture]] §3–4 · [[Task Queue]] · [[Data Model]]

### 4.1. Ingestion Pipeline

1. **Discovery**: Crawler receives seed URLs or finds new content via sitemaps / social media APIs.
2. **Fetching**: 
   - For static web pages: `reqwest` + proxies → fetch HTML.
   - For social media: `obscura` or `scrapebadger-rust` → fetch structured JSON.
3. **Parsing**: Extract title, content, author, timestamp, comments, media URLs.
4. **Enrichment**:
   - Run sentiment analysis (`vader-sentiment-rust` / `rust-bert`).
   - Generate CLIP embeddings for images.
   - Hash content for deduplication.
5. **Queue**: Push normalised data to Redis queue.
6. **Indexing**: Workers pull from queue and POST to Meilisearch index.

### 4.2. Search Flow

1. **User Input**: Text, voice, or image submitted via frontend.
2. **Pre‑processing**:
   - Voice → transcribed to text via `whisper.cpp`.
   - Image → OCR extracted text OR CLIP embedding generated.
3. **Language Detection**: `lingua-rs` detects language (Arabic/Darija/French/English).
4. **Query Expansion**: `DziriBERT` (or custom dictionary) expands Darija/Arabizi variants.
5. **Search**: Query sent to Meilisearch (or vector index for images).
6. **Ranking**: Meilisearch applies ranking rules (relevance, freshness, sentiment).
7. **Summarisation**: Top 10 results sent to `llama-cpp-rs` → generates 2‑3 sentence summary.
8. **Response**: JSON payload with `{ summary, results }` returned to frontend.
9. **Rendering**: Frontend displays summary at top, followed by paginated links.

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

- TLS 1.3 for all network communication.
- API keys required for crawler authentication.
- No persistent query logs.
- Encryption at rest for indexes and vector stores.

### 5.4. Reliability

- Circuit breakers in proxy management (`stygian-proxy`).
- Retry logic with exponential backoff for failed fetches.
- Health checks and automatic restart for failed services (Docker).

---

## 6. Implementation Phases

> Superseded in detail by **[[TODO]]** and the milestone notes, which break these phases into tasks
> and subtasks with exit gates: [[Milestone 0 - Foundations]] · [[Milestone 1 - Text Search MVP]] ·
> [[Milestone 3 - Multimodal Input]] · [[Milestone 2 - Ingestion at Scale]] ·
> [[Milestone 4 - Quality and Operations]] · [[Milestone 5 - Beta Launch]].
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
| Tantivy, `obscura`, `stealthscraper-rs` | Apache-2.0 |
| `rust-bert`, `llama-cpp-rs`, `vader-sentiment-rust` | MIT / Apache-2.0 |
| Redis | BSD |
| Grafana | AGPL-3.0 (self‑hosted) |
| `docker-compose` | Apache-2.0 |

All components are freely available for modification, distribution, and commercial use.

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