---
tags:
  - component
  - serving
  - storage
component-id: C07
binary: qdrant
status: in-progress
updated: 2026-08-21
---

# Vector Index

> **ID** C07 · **Service** `qdrant` · **Upstream** [[Indexer Worker]], [[Image Pipeline]] · **Downstream** [[Image Pipeline]], [[Query Pipeline]]

> **Implemented (2026-08-21):** the `xustive-vector` crate is a lean Qdrant REST client matching §4:
> collection with int8 quantisation and payload indexes, cosine ANN search with `ef` tuning and
> NSFW filtering, and `delete_by_document` for takedowns/orphans. The write path is the crawler's
> `media_embed`; the read path is `POST /api/v1/search/image`. Embeddings come from the `clip-embed`
> sidecar (CLIP ViT-B/32, CPU-capable). Verified live against dev Qdrant with synthetic vectors.
> Off by default (`[vector] enabled`) until a CLIP model is provisioned. Not yet wired: the phash
> reuse-skip (§5), the scheduled orphan-reconciliation job (§7), and the recall/latency measurement
> (§4 — those numbers stay a hypothesis until measured on our corpus).

## 1. Purpose

Approximate nearest-neighbour search over CLIP image embeddings, powering reverse image search
("find posts containing this image"). Kept separate from [[Search Index]] because vector search and
lexical search have different memory profiles, different failure characteristics, and — critically —
vector search being down must not affect text search.

## 2. Responsibilities

**In scope**: storing 512-d image embeddings with filterable payload; cosine ANN search;
payload-filtered search (date range, source type, NSFW); snapshots.

**Out of scope**: generating embeddings (→ [[Image Pipeline]]); OCR; text embeddings (v2).

## 3. Interface

| Op | API | Caller |
|:---|:---|:---|
| Upsert points | `PUT /collections/image_clip/points` | [[Indexer Worker]] |
| Search | `POST /collections/image_clip/points/search` | [[Image Pipeline]] |
| Delete by filter | `POST /collections/image_clip/points/delete` | takedown flow |
| Snapshot | `POST /collections/image_clip/snapshots` | backup job |

Search request:

```jsonc
{ "vector": [0.013, -0.221, …],        // 512 floats, L2-normalised
  "limit": 40, "with_payload": true, "score_threshold": 0.75,
  "filter": { "must": [ { "key": "is_nsfw", "match": { "value": false } } ],
              "should": [], 
              "must_not": [] } }
```

## 4. Internal Design (configuration)

### Collection `image_clip`

```jsonc
{
  "vectors": { "size": 512, "distance": "Cosine", "on_disk": true },
  "hnsw_config": { "m": 16, "ef_construct": 128, "full_scan_threshold": 10000, "on_disk": true },
  "optimizers_config": { "default_segment_number": 4, "indexing_threshold": 20000 },
  "quantization_config": { "scalar": { "type": "int8", "quantile": 0.99, "always_ram": true } },
  "payload_schema": {
    "document_id": "keyword", "media_url": "keyword", "source_type": "keyword",
    "published_at": "integer", "is_nsfw": "bool", "phash": "keyword"
  }
}
```

Design choices:
- **int8 scalar quantisation with `always_ram`** — 512 × int8 = 512 B/vector, so 5M vectors ≈ 2.5 GB
  resident. Full float32 vectors stay `on_disk` and are used only for rescoring the top candidates.
  Without quantisation, 5M × 512 × 4 B = 10 GB resident, which does not fit the budget in
  [[Deployment Topology]].
- **Cosine on L2-normalised vectors** — normalisation happens in [[Image Pipeline]] once, at write
  time, so query-time cosine is a dot product.
- **Payload indexes** on `source_type`, `published_at`, `is_nsfw` — unindexed payload filters force a
  full scan and blow the latency budget.
- `ef` at search time is tuned per request: 64 default, 128 when `limit > 40`.

### Recall vs latency

| `ef` | recall@10 (measured target) | p95 latency |
|:---|:---|:---|
| 32 | ~0.90 | ~12 ms |
| 64 | ~0.95 | ~20 ms |
| 128 | ~0.98 | ~40 ms |

Default `ef = 64`. These numbers must be **measured** on our own corpus during
[[Milestone 3 - Multimodal Input]], not assumed.

## 5. Configuration

| Key | Default | Notes |
|:---|:---|:---|
| `collection` | `image_clip` | |
| `dim` | 512 | CLIP ViT-B/32 |
| `search_limit` | 40 | before dedup by `document_id` |
| `score_threshold` | 0.75 | below → "no similar images found" |
| `ef_search` | 64 | |
| `upsert_batch` | 256 | |
| `phash_prefilter` | `true` | exact-duplicate shortcut before ANN |

## 6. Data

`MediaEmbedding` shape in [[Data Model]] §4. One point per media item, not per document — a post with
4 images produces 4 points sharing a `document_id`. Results are collapsed by `document_id` in
[[Image Pipeline]] before display.

Sizing at 5M images: ~2.5 GB quantised RAM + ~11 GB on disk (float32 + payload + HNSW graph).

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Qdrant down | health check | image *similarity* returns 503; OCR path still works; **text search unaffected** |
| Collection missing | startup check | migration job recreates it; embeddings are re-derivable from stored media |
| Dimension mismatch on upsert | API 400 | DLQ + alert — means a model version changed without a migration |
| Recall degraded after bulk upsert | scheduled recall probe | trigger optimiser / raise `ef` |
| Disk pressure | disk metric | prune points whose parent document was deleted |
| Orphan points (document deleted) | nightly reconciliation job | delete by `document_id` filter |

The orphan reconciliation job matters for takedowns: deleting a document from [[Search Index]] must
also delete its vectors, or a removed image remains findable by similarity
([[Security and Privacy]] §8).

## 8. Performance

| Operation | Budget |
|:---|:---|
| ANN search, 5M points, `ef=64` | ≤ 40 ms p95 |
| Filtered search (indexed payload) | ≤ 60 ms p95 |
| Upsert batch of 256 | ≤ 200 ms |
| Whole `/search/image` request | ≤ 500 ms ([[Performance Budgets]]) |

## 9. Observability

Qdrant `/metrics` scraped: points count, segment count, search latency, RPS. Ours:
`xustive_vector_search_duration_seconds`, `xustive_vector_zero_results_total`,
`xustive_vector_orphans_deleted_total`.

## 10. Security

Internal network only, never a published port ([[Security and Privacy]] T5). API key required.
Query vectors are derived from uploaded images and are never persisted — an uploaded image produces
a transient vector that exists only for the duration of the request ([[Security and Privacy]] P4).

## 11. Testing

- Integration: index 10k fixture images; assert a re-uploaded image returns itself as rank 1 with
  score > 0.95.
- Near-duplicate: crop/resize/recompress an image; assert it still ranks in the top 3.
- Negative: an unrelated image must return nothing above `score_threshold`.
- Reconciliation: delete a document, run the job, assert the vectors are gone.
- Recall probe: a fixed 500-query probe set run nightly; alert if recall@10 drops > 3 %.

## 12. Open Questions

- [ ] Add a `text_clip` collection for cross-modal search ("find images of couscous" by text)?
      CLIP supports it and the model is already loaded.
- [ ] Is `score_threshold = 0.75` right for CLIP cosine on our corpus? Needs measurement.
- [ ] Should video thumbnails from TikTok get one embedding per keyframe, or one per video?

## Related

[[Image Pipeline]] · [[Indexer Worker]] · [[Data Model]] · [[Search Index]] ·
[[Deployment Topology]] · [[UI - Image Search]]
