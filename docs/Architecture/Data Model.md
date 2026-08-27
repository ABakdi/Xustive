---
tags:
  - architecture
  - data
type: architecture
status: specified
updated: 2026-08-27
---

# Data Model

> The canonical shapes every component agrees on. If a struct is not defined here, it is local to a
> component and must not cross a queue or an API boundary.
> Parent: [[System Architecture]] · Consumers: [[Content Parser]], [[Indexer Worker]], [[Search Index]], [[API Contract]]
>
> **Verified against the code, 2026-08-27.** The Rust structs in `crates/xustive-core/src/model.rs`
> and the index settings in `crates/xustive-search/src/settings.rs` are the authority; where this
> note and they disagreed, this note was corrected and the difference is called out inline. Fields
> and stores that are specified but have no producer today are marked as such.

---

## 1. Entities

| Entity | Store | Primary key | Cardinality |
|:---|:---|:---|:---|
| `Document` | Meilisearch index `documents` | `id` (ULID) | 10M target |
| `Comment` | Meilisearch index `comments` — settings declared, **no producer yet** (2026-08-27; the social connectors that would write it are not built) | `id` (ULID) | 50M target |
| `Entity` | Meilisearch index `knowledge` (§4a, M8) | `id` (Wikidata Q-id) | seeds + live harvest |
| `MediaEmbedding` | Qdrant collection `image_clip` (512-d CLIP); a second collection `text_bge` (1024-d) backs the `semantic` profile | `point_id` (UUID) | 5M target |
| `Source` | Meilisearch index `sources`, mirrored in git at `data/sources/registry.jsonl` | `id` (slug) | ~10k |
| `CrawlState` | Redis keys `crawl:*` (`counters`, `hosts`, `state`, `recent`, `skips`, `paused`, `source`, `channel`) plus the frontier | key per concern, not per host | — |
| `DedupKey` | the frontier's `seen` set, rotated every 45 days so large sites resurface | URL | 10M |

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
  "content_type": "text/html",      // fetched MIME — the Files vertical selects PDFs on it (M2)
  "discovery": "sitemap",           // seed | link | sitemap | common_crawl | query_driven | brave | serp | federation | unknown (M7)
  "access_path": null,              // which collection path fetched it, social only

  "title": "…",
  "excerpt": "…",                   // ≤ 320 chars, used in result cards
  "body": "…",                      // normalised plain text, ≤ 200 KB
  "body_len": 4821,
  "body_source": "native",          // native | ocr | caption_asr — where the text came from (M3)

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
    "model": "none"                 // provenance, see [[Sentiment Engine]]; "none" until a model runs
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
      "phash": "0x8812…",           // perceptual hash, image dedup
      "provider": null              // youtube | … for embedded video (M9); filterable as media.provider
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
  "enrichment_level": "full",       // full | partial — partial when enriched under load; the repass job finds them
  "schema_version": 1
}
```

### Index shape, as declared (2026-08-27)

`crates/xustive-search/src/settings.rs` is the single place the `documents` settings live; the API
applies them at start and `make migrate-check` reports drift. A unit test there asserts every
facet the API exposes is declared.

| Setting | Attributes |
|:---|:---|
| `searchableAttributes` (ordered — `attribute` ranking uses it) | `title`, `excerpt`, `entities`, `body`, `media.ocr_text`, `translit_body`, `author.name` |
| `filterableAttributes` | `source_type`, `source_id`, `domain`, `language`, `script`, `sentiment.label`, `published_at`, `crawled_at`, `is_nsfw`, `published_at_precision`, `quality_score`, `spam_score`, `geo.wilaya`, `topics`, `robots_indexable`, `discovery`, `enrichment_level`, `content_type`, `id`, `media.type`, `media.provider` |
| `sortableAttributes` | `published_at`, `crawled_at`, `quality_score`, `engagement.likes` |
| `displayedAttributes` | `id`, `title`, `url`, `canonical_url`, `excerpt`, `source_type`, `source_id`, `domain`, `author`, `published_at`, `published_at_precision`, `sentiment`, `engagement`, `language`, `media`, `simhash`, `quality_score`, `comments_count`, `discovery`, `entities`, `topics`, `body_len` — **`body` is not displayed**, which is what [[Legal and Compliance]] §3 relies on |

`media.type` / `media.provider` are filterable because Meilisearch flattens arrays of objects: the
Images and Videos verticals (M9) select "any document with at least one media entry of that type"
— a settings change, not a reindex. Typo tolerance is disabled on `entities`.

### Field rules

- `media[].type` is the wire name; the Rust field is `kind` with `#[serde(rename = "type")]`.
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

> 2026-08-27: the `comments` index is created and its settings applied (`searchableAttributes`
> `body`, `author.name`; filterable `document_id`, `source_type`, `sentiment.label`,
> `published_at`, `language`), but nothing writes to it yet — comments arrive with the social
> connectors, which are not built.

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

## 4a. `Entity` (Meilisearch index `knowledge`, M8)

The answer layer's store ([[Knowledge Store]], [[ADR-0019 - The Knowledge Layer]]). The index
document is a flattening of the harvested entity, defined in
`crates/xustive-knowledge/src/index.rs`; the field names are constants shared with the settings so
the two cannot drift:

```jsonc
{
  "id": "Q83495",                  // Wikidata id — the primary key
  "kind": "film",                  // filterable — the only facet
  "names": ["The Matrix", "المصفوفة", "Matrix"],   // every label + alias, all languages, searchable
  "descriptions": ["1999 film", "…"],              // searchable
  "prominence": 0.91,              // sortable — tie-breaker when two entities share a name
  "updated_at": 1700000000,        // sortable — the harvester's re-harvest cadence reads it
  "entity": { /* the full Entity: facts, provenance, image credits, licence strings */ }
}
```

`entity` is **never searchable**: it holds image credits, licence strings and authority names that
would otherwise match queries. It is serialised defensively — an entity that will not serialise
writes as `null` and reads back as *absent* (no panel), never as a failed search or a failed
harvest pass. Ranking rules put `exactness` second so the exact name wins over a typo neighbour.

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

| Stream | `payload` | Built? |
|:---|:---|:---|
| `q:fetch` | `{ url, kind, headers?, cursor?, depth }` | ❌ fetch runs inside `crawld` against the Redis frontier, not a stream |
| `q:parse` | `{ url, kind, raw_ref, content_type, http_status, fetched_at }` | ❌ parse and enrich happen in-process in `crawld` |
| `q:enrich` | `{ document: Document }` (pre-enrichment) | ❌ as above |
| `q:index` | `{ document: Document, comments: Comment[] }` | ✅ the one stream that exists (`queue.index_stream`), capped with `XADD MAXLEN ~ 20 000` entries — a byte budget, not a count, after PROB-001's OOM |
| `q:dlq:<stage>` | `{ original, error, attempts, failed_at }` | ✅ for the index stage (`make dlq A=stats\|peek\|replay`) |

(2026-08-27) Raw bodies are kept in Redis for `queue.raw_ttl_days` so extraction can be re-run
without a re-fetch — **0 by default, which disables it**, because blanket storage would overwhelm
the development Redis; the real home is object storage.

---

## 7. Versioning and Migration

- `schema_version` is on every stored entity. Readers must tolerate `version <= current`.
- Adding a field → bump minor, no reindex. Changing/removing a field → new index alias + backfill.
- Reindex (`xustive-cli reindex`, M4-T04.8) rebuilds into a staging index, verifies the count,
  then swaps it into place in one atomic Meilisearch operation; `--rollback` swaps the previous
  contents back. It is an index **swap**, not an alias — Meilisearch has no aliases.
- Backfill: with `raw_ttl_days > 0` the stored raw body is reparsed; otherwise re-crawl.

---

## 8. Retention

| Data | Retention | Rationale |
|:---|:---|:---|
| `Document`, `Comment` | indefinite until takedown | it's the index |
| Raw fetched HTML/JSON | `queue.raw_ttl_days`, **0 (off) by default** | reparse without re-crawl; object storage is the intended home |
| Frontier `seen` set | rotated every 45 days, kept over two rotations | bound memory; big sites resurface |
| Query strings, tied to anyone | **never** | [[Security and Privacy]] P1/P5 |
| Normalised query terms as identifier-free counters | sliding window, decays; k ≥ 20 on a shared instance; **off by default** | [[ADR-0018 - Anonymous Search History]] amended ADR-0008 for exactly this: the interaction store keeps `(term → count, clicks)` with no IP, session or timestamp finer than the window — see [[Interaction Signals]] |
| Metrics | 15 days raw / 1 year downsampled | [[Observability]] |

---

## 9. Open Questions

- [x] `translit_body` is stored and searchable (weighted below `body`).
- [ ] Is one level of comment threading enough for Facebook group discussions?
- [ ] Should `engagement` be a separate mutable index to avoid rewriting whole documents on refresh?

## Related

[[Search Index]] · [[Vector Index]] · [[Knowledge Store]] · [[Content Parser]] · [[Indexer Worker]] ·
[[Ranking and Relevance]] · [[API Contract]] · [[Interaction Signals]]
