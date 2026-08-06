---
tags:
  - reference
type: reference
status: living
updated: 2026-08-06
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
| **DziriBERT** | A BERT model trained on Algerian dialect text; used optionally for expansion and sentiment. |

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
| **Egress segmentation** | Only `xustive-crawler` can reach the internet. This is what makes "queries never leave" enforceable rather than promised. |
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

## Project

| Term | Meaning |
|:---|:---|
| **Component id** (`C01`…) | Stable identifier per component, used in commit scopes and [[Component Map]]. |
| **ADR** | Architecture Decision Record — a decision that was hard to make and expensive to reverse ([[Decision Log]]). |
| **⚖ VERIFY** | A marker in [[Legal and Compliance]] indicating something that must be confirmed by a lawyer before shipping. |
| **Quality gate** | A metric threshold that blocks a merge, distinct from a pass/fail test ([[Testing Strategy]] §7). |

## Related

[[Home]] · [[Component Map]] · [[Data Model]] · [[Decision Log]] · [[UI - RTL and Localization]]
