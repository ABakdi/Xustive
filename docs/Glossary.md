---
tags:
  - reference
type: reference
status: living
updated: 2026-08-27
---

# Glossary

> Shared vocabulary. If a term means something specific in this project, it is defined here — and the
> definition here is the one that counts.
> Parent: [[Home]]

---

## Language and Region

| Term | Meaning |
|:---|:---|
| **Darija** (`ary`) | Algerian colloquial Arabic. Distinct enough from MSA that a query in one may not match content in the other. Written in Arabic script *or* Latin script. The reason [[Query Expander]] exists. |
| **MSA** (`ar`) | Modern Standard Arabic. The formal register of news, government, and official pages. |
| **Arabizi** | Darija written in Latin script with digits standing in for Arabic consonants: `3`=ع, `7`=ح, `9`=ق, `5`/`kh`=خ, `2`=ء. E.g. `wach rak`, `ch7al`, `3aslema`. |
| **Code-switching** | Mixing languages within one sentence — *rani f la gare* (Darija + French). Routine in Algerian text and a core detection challenge ([[Language Detector]]). |
| **Wilaya** | Algerian administrative province (58 of them). Used in `geo.wilaya` and as a gazetteer for entity extraction. |
| **Tatweel** | The Arabic kashida character `U+0640` used to stretch words visually. Stripped during normalisation because it changes bytes without changing meaning. |
| **Harakat** | Arabic short-vowel diacritics. Stripped at both index and query time. |
| **Bidi** | Bidirectional text — mixed RTL/LTR in one string. See [[UI - RTL and Localization]] §5. |

## Search

| Term | Meaning |
|:---|:---|
| **Retrieval** | Stage 1: getting candidate documents out of the index. Meilisearch's job. |
| **Re-ranking** | Stage 2: reordering candidates with our own signals in [[Query Pipeline]]. |
| **Facet** | A filterable, countable attribute (source type, sentiment, language). Counts reflect the current query plus all other active filters. |
| **Recall** | Fraction of relevant documents retrieved. What [[Query Expander]] improves. |
| **Precision** | Fraction of retrieved documents that are relevant. What expansion risks harming. |
| **nDCG@10** | Normalised discounted cumulative gain over the top 10 — the primary relevance metric ([[Ranking and Relevance]] §6). |
| **Golden set** | A fixed set of queries with human-judged results, used as the regression gate for ranking changes. |
| **Zero-result rate** | Share of searches returning nothing. The main *aggregate* health signal available to us, given [[ADR-0008 - No Query Logging]]. |
| **Query expansion** | Adding variant forms of query terms to improve recall. |
| **Freshness decay** | `exp(−age_days / τ)`, the time component of ranking. τ varies by inferred query intent. |
| **Trust tier** | A/B/C accountability rating of a source, contributing a ranking boost ([[Data Sources Registry]] §3). |
| **Authority** | A curated per-domain prior for how well-known a site is (`data/sources/authority.tsv`, `xustive-search::authority`), compiled in so a missing file cannot flatten the signal. Algeria-first: home-floor for `.dz`. A link-graph PageRank exists in the CLI but authority, not PageRank, is the serving-time signal ([[Ranking and Relevance]]). |
| **Vertical** | A saved filter over the one `documents` index, selected with `?v=`: `all` (default), `news` (dated web documents), `images` and `videos` (`media.type` facet). Not a separate index ([[UI - Search Verticals]]). |
| **Federation** | Asking a self-hosted SearXNG for live results at query time, merged under a strict budget, when the local corpus is thin ([[Federation Gateway]], [[ADR-0017 - Query-Time Federation with External Metasearch]]). Off by default (`[federation] enabled`). |
| **Federator** | `xustive-federator`, the one process on both the `core` and `ingest` networks — the single allowlisted egress hop the API is permitted to call. It talks to SearXNG; the API never does. |
| **SearXNG** | The open-source metasearch aggregator we self-host on the `ingest` network. Categories `web`, `images`, `videos`. |
| **Eager index** | Writing a thin document (title + snippet) from a federated hit into the index at once so it is a real result in seconds; the crawl overwrites it later. Off by default because it puts un-crawled text into the index. |
| **Weak coverage** | Query-driven discovery: k-anonymous, windowed counters of search terms the corpus could not answer, used to find sources worth adding. Off unless `[discovery] weak_coverage_enabled`; the reconciliation with [[ADR-0008 - No Query Logging]] is structural, not policy. |
| **Interaction signal** | An anonymous, k-anonymous, windowed click count on a (query-hash, document) pair kept in a *separate* Redis (`signals_url`) and folded into ranking as CTR ([[Interaction Signals]], [[ADR-0015 - Anonymous Interaction Signals for Ranking]]). Off by default. |
| **Hot click** | A (query, document) pair whose clicks pass `hot_click_floor` (defaults to *k*), which makes the document a re-crawl freshness candidate. |

## Knowledge and Answers

> Vocabulary from [[ADR-0019 - The Knowledge Layer]] and [[Milestone 8 - The Answer Layer]].

| Term | Meaning |
|:---|:---|
| **Entity** | A thing the product knows about — a person, place, film, team — stored as a typed record (`xustive-knowledge::Entity`) with labels per language, a kind, claims, and a Wikidata id. |
| **`knowledge` index** | The third Meilisearch index (after `documents` and `comments`), holding entities. Written by `xustive-toold`'s Wikidata harvest, read by `GET /api/v1/knowledge`. |
| **Resolver** | `xustive-knowledge::resolve`: the pure, index-free judgement of *which* entity a query means, if any. Built to say nothing rather than guess — a confident panel about the wrong thing is worse than no panel. |
| **Harvest** | `xustive-toold`'s scheduled fetch of entities from the Wikidata API into the `knowledge` index. Runs on the ingest network; the serving plane only reads. |
| **Live entity** | An entity not in the store, looked up on Wikidata by the **web tier** (`/api/knowledge-live`, the one place with sanctioned egress per [[ADR-0014 - Knowledge Panel from Wikipedia via the Web Tier]]) and rendered by the API's `/knowledge/render` so both paths share one parser and template set. |
| **List answer** | The cast of a film, the books of an author: members from one Wikidata SPARQL query via `/api/knowledge-list`, each card linking to authorities by identifier, none of which is scraped. No ratings. |
| **Instant answer / tool card** | A computed answer above results: calculator (fend), units, dates, prayer times, fuel, exam dates, wilaya facts, utilities, translate, transliterate ([[Instant Answers]]) plus the data-backed currency and weather cards ([[Tool Data Plane]]). |
| **Tool data plane** | `xustive-toold` on the ingest network fetching rates (open.er-api) and weather (Open-Meteo) into Redis on a schedule; the API reads the cache and never fetches. If toold dies, cards age out; search is unaffected. |
| **Wilaya coarsening** | Turning a connecting IP into *at most* a wilaya via a local DB-IP database (`maxminddb`), on the stack, never stored, never a cache key — so "weather" with no place can be answered ([[ADR-0020 - Approximate Location from a Local Database]]). |
| **Thumb proxy** | `/api/thumb?u=&s=`: the web tier fetches a result's image so the reader's address and referrer never reach the crawled host. `s` is an HMAC over `u`; unsigned requests are refused before any fetch. Wikimedia and Open Library hosts are allowed unsigned because they are public by construction ([[ADR-0021 - Proxied Thumbnails with Signed URLs]]). |
| **Sidecar** | A small Python HTTP service holding a model the Rust build should not carry: `stt-sidecar` (faster-whisper), `ocr-sidecar` (Unlimited-OCR, GPU), `clip-embed`, `text-embed` (bge-m3). Each is opt-in, on the `core` network, health-checked, and its absence degrades one feature only. |

## Ingestion

| Term | Meaning |
|:---|:---|
| **Frontier** | The prioritised set of URLs waiting to be fetched, owned by [[Crawler Orchestrator]]. |
| **Seed** | A starting URL or social object from [[Data Sources Registry]]. |
| **Revisit interval** | How long before we re-fetch a URL; adapts to whether content actually changed. |
| **Crawler trap** | A site structure generating infinite URLs (calendars, faceted navigation, repeating paths). Detected by depth, param count, and path-repetition rules. |
| **Politeness** | The set of self-imposed limits protecting crawled hosts: robots, crawl-delay, one request per host at a time ([[Politeness and Robots]]). |
| **Headless fetch** | Rendering a page in a real browser to get JS-generated content. ~30× the cost of a static fetch, capped at 10 % of fetches. |
| **Enrichment** | Adding sentiment, OCR, embeddings, quality, and spam scores after parsing. |
| **SimHash** | A 64-bit locality-sensitive hash; similar documents have small Hamming distances. Near-duplicate detection ([[Deduplication Service]] §4.3). |
| **pHash / dHash** | Perceptual image hash; survives resizing and re-encoding. Used for image dedup and to skip redundant CLIP embedding. |
| **DLQ** | Dead-letter queue. Where a message goes after exhausting retries or hitting a poison error. |
| **Backpressure** | Slowing producers when consumers fall behind — signalled by queue depth ([[Error Handling and Resilience]] §4). |
| **Poison message** | A payload that reliably crashes a stage. Goes to the DLQ and becomes a test fixture. |
| **Idempotent** | Safe to run more than once with the same effect. Required of every ingestion stage because delivery is at-least-once. |
| **Bounded frontier** | The frontier has a hard URL cap (`frontier_max_urls`), a per-host page cap, and an outlink cap per page, so a crawl cannot grow without limit ([[PROB-001 - Bounded Frontier and Queue]]). |
| **Discovery channel** | A way new sources arrive: seeds, sitemaps, outlinks, Common Crawl, Brave (optional API), SERP collection ([[ADR-0013 - Direct SERP Collection for Discovery]]) and weak coverage. Each is visible on `/admin/discovery`. |
| **Raw store** | A copy of fetched HTML kept in Redis for `raw_ttl_days` so a parser fix can re-parse without re-fetching (`xustive-ingest::raw_store`). Off by default. |

## Models and ML

| Term | Meaning |
|:---|:---|
| **CLIP** | A model producing comparable embeddings for images (and text). We use the image tower, 512-d, for reverse image search. |
| **Embedding** | A fixed-length float vector representing content; similarity is cosine distance. |
| **ANN** | Approximate nearest neighbour — fast, slightly lossy vector search. Qdrant's HNSW index. |
| **Quantisation** | Storing model weights (or vectors) at lower precision to cut memory. `Q4_K_M` for the LLM; int8 for stored vectors. |
| **TTFT** | Time to first token. The number that determines whether a streaming summary *feels* fast. |
| **WER / CER** | Word / character error rate — accuracy metrics for [[Speech to Text]] and OCR. |
| **Prompt injection** | Hostile instructions embedded in crawled content attempting to steer the model. Defended in [[Summarizer]] §4.5. |
| **Faithfulness** | Whether a summary's claims are actually supported by the passages. Our gate is ≥ 95 %. |
| **DziriBERT** | A BERT model trained on Algerian dialect text. Planned as an optional expansion/sentiment tier in 2026-02; **not built** — expansion is lexicon-driven (`xustive-lang`) and sentiment is a lexicon scorer. |
| **bge-m3** | The multilingual text-embedding model (1024-d, Apache-2.0) behind semantic search: `text-embed` sidecar → Qdrant `text_bge` collection. Off by default (`[vector] text_enabled`). |
| **Partial model** | The lighter Whisper (`base`) the STT sidecar runs every half-second for a live reading while the person is still speaking; the `small` model gives the final text. |
| **Device** | `[ml] device`: `auto` / `cpu` / `cuda`. The reference machine is a 4 GB Quadro T1000, and every model path must also run CPU-only; the device is switchable from `/admin/compute`. |

## Collection

> Vocabulary from [[ADR-0009 - Direct Collection for Social Platforms]].

| Term | Meaning |
|:---|:---|
| **Identity** | The unit of collection currency: an account plus its pinned proxy, fingerprint, device profile, and cookie jar. Not just "an account" ([[Session Manager]] §4.1). |
| **Pinning invariant** | `account ↔ proxy ↔ fingerprint ↔ device` stays stable for an identity's life. Rotating any element independently is the largest single cause of bans. |
| **Warm-up** | Human-shaped browsing a new identity performs for 10+ days before it is allowed to collect anything. Wall-clock; cannot be parallelised away. |
| **Burned** | An identity retired permanently after repeated challenges. Credentials revoked, never reused. |
| **Cloaking** | A platform serving HTTP 200 with plausible but empty content instead of an error. The dominant silent failure mode — a connector that trusts status codes reports success while collecting nothing. |
| **Canary** | A known-stable public object fetched on a schedule by a low-value identity. Ground truth for distinguishing "we are being cloaked" from "the platform changed". |
| **Soft ban** | Degraded or empty responses without an explicit error. Detected by canary disagreement plus `consecutive_empty`. |
| **Challenge** | A captcha, checkpoint, 2FA prompt, or suspicious-login interstitial. Quarantines the identity. |
| **JA3 / JA4** | Hashes of a TLS ClientHello (cipher order, extensions, curves). The default Rust TLS handshake is instantly identifiable as non-browser. |
| **Coherence** | Whether a fingerprint's layers agree — UA, TLS, HTTP/2, headers, JS surface, proxy geo, timezone, language. Detection targets *incoherence*, not any single value ([[Fingerprint Engine]] §4.2). |
| **Signer** | Obfuscated platform JavaScript computing request parameters (`X-Bogus`, `msToken`). Rotates without notice ([[Signature Service]]). |
| **Session constants** | Per-session values harvested once at bootstrap: `fb_dtsg`, `lsd`, `X-IG-WWW-Claim`, `doc_id`. |
| **Access-path ladder** | The ordered list of ways to reach a platform's content, cheapest and most stable first, with automatic demotion on failure. |
| **Embedded hydration** | A JSON blob the platform ships inside public page HTML (`__UNIVERSAL_DATA_FOR_REHYDRATION__`, `_sharedData`). Needs no signing or login — the most stable path available. |
| **Crawl profile** | `open_web` (robots-compliant, honest UA) vs `platform` (fingerprinted, identity-based). Config-driven and CI-asserted so the two cannot be confused ([[Politeness and Robots]] §4.0). |
| **Identity lifespan** | Median days from `mature` to `burned`. The metric that says whether pacing is sustainable. |

## Privacy and Security

| Term | Meaning |
|:---|:---|
| **k-anonymity** | A record is indistinguishable among at least *k* others. Our threshold for any aggregate query statistic is k ≥ 20. |
| **SSRF** | Server-side request forgery — tricking our crawler into fetching internal addresses. Defended by the `SafeUrl` type ([[Security and Privacy]] §4). |
| **`SafeUrl`** | A newtype no URL can bypass on its way to the HTTP client; validates scheme, resolved IP range, port, and every redirect hop. |
| **Egress segmentation** | The `core` Docker network is `internal: true`; only the `ingest` network (crawler processes, `toold`, `searxng`, `federator`) can reach the internet, and the web tier's three sanctioned fetchers (`/api/knowledge*`, `/api/wiki-image`, `/api/thumb`). This is what makes "queries never leave" enforceable rather than promised. `scripts/test-egress.sh` checks it. |
| **Admin key** | `[api] admin_key`: the bearer secret every `/api/v1/admin/*` call carries. Empty in dev. |
| **Takedown** | Permanent removal of content plus a URL blocklist entry, so re-crawling cannot resurrect it. |
| **Decompression bomb** | A small file that expands enormously when decoded. Guarded by pixel/sample budgets on every upload. |

## Operations

| Term | Meaning |
|:---|:---|
| **Plane** | One of the two halves of the system: serving (synchronous) or ingestion (asynchronous). [[ADR-0001 - Two-Plane Architecture]]. |
| **Degradation ladder** | The ordered list of things we drop under load before failing a request ([[Error Handling and Resilience]] §6). |
| **Circuit breaker** | Per-host or per-platform state that stops requests after repeated failures, with an exponential cooldown. |
| **Fail open / fail closed** | Whether a component allows or blocks work when its state store is unavailable. Dedup fails **open**; politeness fails **closed**. The asymmetry is deliberate. |
| **SLO** | Service level objective — a target with an error budget ([[Performance Budgets]] §8). |
| **Alias flip** | Atomically repointing `documents` from `documents_v1` to `documents_v2`; how we reindex without downtime. |
| **Consumer group** | A Redis Streams construct letting multiple workers share a stream with per-message acknowledgement. |
| **Signals Redis** | The second Redis (`[queue] signals_url`, port 6391 in dev) holding only interaction and weak-coverage counters, so a queue dump never contains a click and vice versa. |
| **Loadgen** | `xustive-loadgen`, the open-loop Rust load generator (`make load`) that measures the serving plane against [[Performance Budgets]]. |
| **Toold** | `xustive-toold`, the scheduled fetcher of external data (rates, weather, entity harvest). See *Tool data plane*. |

## Project

| Term | Meaning |
|:---|:---|
| **Component id** (`C01`…) | Stable identifier per component, used in commit scopes and [[Component Map]]. |
| **ADR** | Architecture Decision Record — a decision that was hard to make and expensive to reverse ([[Decision Log]]). |
| **⚖ VERIFY** | A marker in [[Legal and Compliance]] indicating something that must be confirmed by a lawyer before shipping. |
| **Quality gate** | A metric threshold that blocks a merge, distinct from a pass/fail test ([[Testing Strategy]] §7). |

## Related

[[Home]] · [[Component Map]] · [[Data Model]] · [[Decision Log]] · [[UI - RTL and Localization]]
