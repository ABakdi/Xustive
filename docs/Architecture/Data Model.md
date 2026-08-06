---
tags:
  - architecture
  - data
type: architecture
status: specified
updated: 2026-08-06
---

# Data Model

> The canonical shapes every component agrees on. If a struct is not defined here, it is local to a
> component and must not cross a queue or an API boundary.
> Parent: [[System Architecture]] · Consumers: [[Content Parser]], [[Indexer Worker]], [[Search Index]], [[API Contract]]

---

## 1. Entities

| Entity | Store | Primary key | Cardinality |
|:---|:---|:---|:---|
| `Document` | Meilisearch index `documents` | `id` (ULID) | 10M target |
| `Comment` | Meilisearch index `comments` | `id` (ULID) | 50M target |
| `MediaEmbedding` | Qdrant collection `image_clip` | `point_id` (UUID) | 5M target |
| `Source` | Meilisearch index `sources` + Redis | `id` (slug) | ~10k |
| `CrawlState` | Redis hash `crawl:{host}` | host | ~50k |
| `DedupKey` | Redis set / Bloom `dedup:*` | content hash | 10M |

Nothing above is a relational store. There is **no** SQL database in v1 — the index *is* the
database, and everything else is derivable by re-crawling. See
[[ADR-0002 - Meilisearch as System of Record]].

---

## 2. `Document`

```jsonc
{
  "id": "01J8ZK4Q...",              // ULID, time-sortable
  "content_hash": "b3:9f2c...",     // BLAKE3 of normalised body — exact dedup
  "simhash": "0xA31F...",           // 64-bit SimHash — near-dup, see [[Deduplication Service]]

  "url": "https://example.dz/article/123",
  "canonical_url": "https://example.dz/article/123",
  "domain": "example.dz",
  "source_type": "web",             // web | facebook | instagram | tiktok
  "source_id": "example.dz",        // FK → Source.id
  "platform_post_id": null,         // native id on the platform, if any

  "title": "…",
  "excerpt": "…",                   // ≤ 320 chars, used in result cards
  "body": "…",                      // normalised plain text, ≤ 200 KB
  "body_len": 4821,

  "language": "ary",                // ar | ary | fr | en | mixed | und
  "language_confidence": 0.86,
  "script": "arabic",               // arabic | latin | mixed
  "translit_body": "…",             // Arabizi→Arabic folded form, see [[Query Expander]]

  "author": {
    "id": "…", "name": "…", "handle": "…", "url": "…", "verified": false
  },

  "published_at": 1754438400,       // unix seconds — ORIGINAL post date
  "crawled_at":   1754524800,       // unix seconds — when we fetched it
  "indexed_at":   1754524830,
  "published_at_precision": "day",  // second | day | month | unknown

  "sentiment": {
    "label": "neutral",             // positive | neutral | negative
    "score": 0.12,                  // −1.0 … +1.0
    "confidence": 0.71,
    "model": "vader-dz@1"           // provenance, see [[Sentiment Engine]]
  },

  "engagement": {
    "likes": 0, "comments": 0, "shares": 0, "views": 0, "captured_at": 1754524800
  },
  "comments_count": 0,

  "media": [
    {
      "type": "image",              // image | video | audio
      "url": "https://…/x.jpg",
      "thumb_url": "https://…/x_t.jpg",
      "width": 1080, "height": 1350,
      "ocr_text": "…",              // from [[Image Pipeline]]
      "ocr_lang": "ar",
      "embedding_id": "6f1e…",      // FK → Qdrant point_id
      "phash": "0x8812…"            // perceptual hash, image dedup
    }
  ],

  "entities": ["الجزائر", "Oran", "Sonelgaz"],
  "topics": ["news", "economy"],
  "geo": { "wilaya": "31", "wilaya_name": "Oran", "lat": null, "lon": null },

  "quality_score": 0.64,            // 0…1, see [[Ranking and Relevance]]
  "spam_score": 0.03,
  "is_nsfw": false,
  "robots_indexable": true,
  "http_status": 200,
  "fetch_method": "static",         // static | headless | api
  "schema_version": 1
}
```

### Field rules

- `published_at` **must never** be back-filled from `crawled_at`. If unknown, set
  `published_at = crawled_at` **and** `published_at_precision = "unknown"` so ranking can discount it.
- `body` is normalised text only: no HTML, NFKC-normalised, tatweel stripped, Arabic-Indic digits
  folded to ASCII. Normalisation rules live in [[Content Parser]].
- `excerpt` is generated at parse time, not at query time — result rendering must stay cheap.
- Unknown numeric fields are `0`, unknown strings are `null`. Never `""` — it defeats facet counts.

---

## 3. `Comment`

```jsonc
{
  "id": "01J8ZK…",
  "document_id": "01J8ZK4Q…",       // parent Document
  "parent_comment_id": null,        // threading, 1 level in v1
  "source_type": "facebook",
  "platform_comment_id": "…",
  "body": "…",
  "language": "ary",
  "author": { "id": "…", "name": "…", "handle": "…" },
  "published_at": 1754438400,
  "crawled_at": 1754524800,
  "sentiment": { "label": "negative", "score": -0.42, "confidence": 0.68, "model": "vader-dz@1" },
  "likes": 12,
  "content_hash": "b3:…",
  "schema_version": 1
}
```

Comments are a **separate index**, not nested objects, because (a) Meilisearch cannot facet on nested
array elements efficiently, and (b) comments outnumber documents ~5:1. The [[Query Pipeline]] runs a
federated multi-search and folds matching comments into their parent card — see
[[ADR-0003 - Comments in a Separate Index]].

---

## 4. `MediaEmbedding` (Qdrant)

| Field | Type | Notes |
|:---|:---|:---|
| `point_id` | UUID | matches `Document.media[].embedding_id` |
| `vector` | `float32[512]` | CLIP ViT-B/32, L2-normalised |
| `payload.document_id` | string | for join-back |
| `payload.media_url` | string | |
| `payload.source_type` | string | filterable |
| `payload.published_at` | int | filterable, range |
| `payload.is_nsfw` | bool | filterable |

Distance metric: **cosine**. Collection config in [[Vector Index]].

---

## 5. `Source`

```jsonc
{
  "id": "elkhabar-dz",
  "kind": "web",                     // web | facebook_group | facebook_page | instagram | tiktok
  "display_name": "El Khabar",
  "entry_points": ["https://www.elkhabar.com/sitemap.xml"],
  "languages": ["ar", "fr"],
  "crawl_policy": {
    "enabled": true,
    "frequency": "hourly",           // realtime | hourly | daily | weekly
    "max_docs_per_run": 500,
    "respect_robots": true,
    "crawl_delay_ms": 1500,
    "depth_limit": 3
  },
  "trust_tier": "A",                 // A | B | C → boosts [[Ranking and Relevance]]
  "added_by": "operator|submission",
  "approved": true,
  "notes": "…",
  "last_run_at": 1754524800,
  "last_status": "ok"
}
```

Registry contents and policy live in [[Data Sources Registry]]; submission flow in
[[Admin and Source Submission]].

---

## 6. Queue Envelope

Every message on every stream shares this outer shape ([[Task Queue]]):

```jsonc
{
  "trace_id": "01J8ZK…",     // constant across the whole ingestion chain
  "stage": "parse",
  "attempt": 1,
  "enqueued_at": 1754524800,
  "source_id": "elkhabar-dz",
  "priority": 5,             // 0 = highest
  "payload": { /* stage-specific */ }
}
```

Stage payloads:

| Stream | `payload` |
|:---|:---|
| `q:fetch` | `{ url, kind, headers?, cursor?, depth }` |
| `q:parse` | `{ url, kind, raw_ref, content_type, http_status, fetched_at }` |
| `q:enrich` | `{ document: Document }` (pre-enrichment) |
| `q:index` | `{ document: Document, comments: Comment[] }` |
| `q:dlq:<stage>` | `{ original, error, attempts, failed_at }` |

`raw_ref` points at a short-TTL Redis key (or object-store path) holding the raw bytes — envelopes
stay small so Redis memory stays predictable.

---

## 7. Versioning and Migration

- `schema_version` is on every stored entity. Readers must tolerate `version <= current`.
- Adding a field → bump minor, no reindex. Changing/removing a field → new index alias + backfill.
- Index aliases: `documents_v1` behind alias `documents`. Reindex writes `documents_v2`, then the
  alias flips atomically. Rollback is an alias flip back.
- Backfill jobs replay from `q:enrich` using stored raw blobs where available, otherwise re-crawl.

---

## 8. Retention

| Data | Retention | Rationale |
|:---|:---|:---|
| `Document`, `Comment` | indefinite until takedown | it's the index |
| Raw fetched HTML/JSON | 7 days | debugging + reparse without re-crawl |
| Redis dedup keys | 180 days sliding | bound memory |
| Query strings | **0 seconds** | [[Security and Privacy]] |
| Aggregate query counters | 90 days, k-anonymous (k ≥ 20) | autocomplete quality |
| Metrics | 15 days raw / 1 year downsampled | [[Observability]] |

---

## 9. Open Questions

- [ ] Do we store `translit_body` for every doc (index size ~+35%) or compute at query time?
- [ ] Is one level of comment threading enough for Facebook group discussions?
- [ ] Should `engagement` be a separate mutable index to avoid rewriting whole documents on refresh?

## Related

[[Search Index]] · [[Vector Index]] · [[Content Parser]] · [[Indexer Worker]] ·
[[Ranking and Relevance]] · [[API Contract]]
