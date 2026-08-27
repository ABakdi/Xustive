---
tags:
  - component
  - serving
  - storage
component-id: C07
binary: qdrant (crates/xustive-vector)
status: built (off by default)
updated: 2026-08-27
---

# Vector Index

> **ID** C07 · **Service** `qdrant` · **Client** `crates/xustive-vector` · **Upstream** the crawler's
> `media_embed` / `text_embed` passes ([[Enrichment Pipeline]]) · **Downstream**
> `POST /api/v1/search/image` ([[Image Pipeline]]), semantic candidates in
> [[Query Pipeline]] (`text_search.rs`)

## 1. Purpose

Approximate nearest-neighbour search, kept apart from [[Search Index]] because vector and lexical
search have different memory profiles, different failure characteristics and — critically —
vector search being down must never affect text search. Two collections now share the client:

- `image_clip` — 512-d CLIP image embeddings, for "find posts containing this image".
- `text_bge` — 1 024-d bge-m3 sentence embeddings, one per document, for the durable fix to "the
  index has the page but the words don't match" (M7-T02).

Both are **off by default** (`[vector] enabled`, `[vector] text_enabled`) until their sidecar and
model are provisioned.

## 2. Where it lives today

| Piece | Path |
|:---|:---|
| REST client, collection setup, search, delete, scroll | `crates/xustive-vector/src/lib.rs` |
| CLIP sidecar client (`Embedder`, `SidecarEmbedder`, `l2_normalise`) | `crates/xustive-vector/src/embed.rs` |
| Text sidecar client (`TextEmbedder`, batch contract, breaker) | `crates/xustive-vector/src/text.rs` |
| Write paths | `xustive-ingest/src/media_embed.rs`, `text_embed.rs`, `embed_cache.rs` |
| Read paths | `xustive-api/src/image_search.rs`, `text_search.rs` |
| Sidecars | `services/clip-embed` (CLIP ViT-B/32), `services/text-embed` (bge-m3) |
| Orphan cleanup | `xustive-cli reconcile-vectors [--dry-run]` |

## 3. Interface

```rust
Store::new(url, api_key, collection, timeout)             // 512-d
Store::with_dim(url, api_key, collection, dim, timeout)   // the text collection
store.ensure_collection()        // PUT /collections/{c} + payload indexes; idempotent
store.upsert(&[Point])           // PUT /collections/{c}/points?wait=true
store.search(&vec, limit, ef, threshold, &SearchFilter)  // POST …/points/search, params.hnsw_ef
store.delete_by_document(id)     // POST …/points/delete, filter document_id
store.all_document_ids(batch) / count()                  // scroll, for reconciliation
pub fn point_id(url_or_id: &str) -> u64                   // stable id: re-crawl overwrites
```

`Point { id, vector, payload: Payload { document_id, media_url, source_type, published_at,
is_nsfw, phash } }`. A text point uses the same payload with the image fields left empty.

Why a hand-written REST client: `qdrant-client` pulls gRPC/`tonic` and a large tree; the
operations here are a handful of JSON POSTs and the rest of the system already speaks `reqwest`.
Every method returns a `Result`; the caller treats an error as "no similar images" / "no dense
candidates", never as a failed request.

## 4. Internal Design

### 4.1 Collection settings (`ensure_collection`)

```jsonc
{ "vectors": { "size": dim, "distance": "Cosine", "on_disk": true },
  "hnsw_config": { "m": 16, "ef_construct": 128, "full_scan_threshold": 10000, "on_disk": true },
  "optimizers_config": { "default_segment_number": 4, "indexing_threshold": 20000 },
  "quantization_config": { "scalar": { "type": "int8", "quantile": 0.99, "always_ram": true } } }
```

Payload indexes on `source_type` (keyword), `published_at` (integer), `is_nsfw` (bool),
`document_id` (keyword), `phash` (keyword), created on every start (idempotent) so an older
collection gains them. int8 with `always_ram` keeps 5M image vectors ≈ 2.5 GB resident; float32
stays on disk for rescoring. The API calls `ensure_collection` at startup; the crawler only writes.

### 4.2 Normalisation

`SidecarEmbedder` L2-normalises on the way out, so both write and query paths store unit vectors
and cosine reduces to a dot product. Whether the sidecar already normalised is irrelevant —
normalising a unit vector is a no-op. The text sidecar returns normalised vectors by contract.

### 4.3 Search

`search(vector, limit, ef, threshold, filter)` with `params.hnsw_ef` set per request from config
(`ef_search = 64`), `score_threshold` from `score_threshold_milli` (750 = cosine 0.75), and
`SearchFilter::safe()` (`is_nsfw = false`). Image hits are collapsed by `document_id` in the API,
keeping each document's best score, then resolved from the lexical index with one `id IN [...]`
query; the response also reports `matched_images` so the UI can say "12 similar images across 5
pages" honestly. Text hits feed reciprocal-rank fusion with the lexical candidates
([[Query Pipeline]]), `text_search_limit = 50`.

The `ef` bump to 128 for `limit > 40` from the original design is not implemented; `ef` is one
config value. The recall/latency table below is **unmeasured** on our corpus (2026-08-27).

### 4.4 pHash reuse cache

`embed_cache.rs`: `frontier:vecphash:{phash}` → little-endian f32 bytes, TTL
`embed_cache_ttl_days` (30). The same image reposted at another URL costs a Redis read, not a
model call; the reused vector is still upserted as its own point. Absence of the cache (Redis
down, TTL 0) means every image is embedded — nothing breaks.

### 4.5 Sidecars

`services/clip-embed` (`CLIP_MODEL=openai/clip-vit-base-patch32`, `CLIP_MAX_BYTES` 8 MiB;
`POST /embed` raw image bytes → `{vector}`; `GET /health`) — ~150M parameters, comfortably CPU-only,
so image similarity is not GPU-gated. `services/text-embed` (`TEXT_EMBED_MODEL=BAAI/bge-m3`,
`TEXT_EMBED_MAX_TEXTS` 128, `TEXT_EMBED_MAX_CHARS` 8192; `POST /embed {texts:[…]}` → vectors) —
truncates rather than rejects, because the head of a page carries its topic. Both run on the
internal `core` network; calling them is not internet egress. `text_dim` must match the sidecar's
model (bge-m3 = 1024; multilingual-e5-small = 384) — change both together.

## 5. Configuration (`[vector]`)

| Key | Dev default | Meaning |
|:---|:---|:---|
| `enabled` | `false` | image similarity (write + read) |
| `qdrant_url`, `qdrant_key` | `http://127.0.0.1:6333`, `""` | sent as `api-key` when set |
| `collection` | `image_clip` | |
| `embedder_endpoint` | `http://127.0.0.1:8092/embed` | CLIP sidecar |
| `timeout_ms` | 10 000 | per HTTP call, both sidecars and Qdrant |
| `search_limit`, `ef_search`, `score_threshold_milli` | 40, 64, 750 | |
| `embed_cache_ttl_days` | 30 | 0 disables the pHash cache |
| `text_enabled` | `false` | semantic text search (write + read) |
| `text_embedder_endpoint` | `http://127.0.0.1:8094/embed` | |
| `text_collection`, `text_dim` | `text_bge`, 1024 | |
| `text_search_limit` | 50 | dense candidates per query before fusion |

## 6. Data

Image: one point per media item keyed by `point_id(media_url)` — a post with four images is four
points sharing a `document_id`. Text: one point per document keyed by `point_id(document.id)`; the
federated eager-index id is reused so a full crawl overwrites the placeholder's vector.

## 7. Failure Modes

| Failure | Response |
|:---|:---|
| Qdrant or a sidecar down at query time | `/search/image` → 503 `image_search_unavailable`; dense candidates empty, lexical search proceeds |
| Qdrant or a sidecar down at index time | vector skipped, logged; document indexed for text; re-derivable on the next crawl |
| Collection missing | created by the API at startup; `reconcile-vectors` reports "nothing to reconcile" on 404 |
| Orphan points (document removed) | `takedown` deletes by `document_id` inline; `reconcile-vectors` walks the collection and deletes vectors whose document is gone from the lexical index |
| Dimension mismatch | Qdrant 400 on upsert, logged and skipped — means `text_dim` and the sidecar model disagree |

## 8. Security

Internal network only. Query vectors from uploads are transient and never stored; the upload
arrives as a raw POST body so it never reaches a URL or access log ([[Security and Privacy]]).
NSFW filtering is on for every image search — though nothing currently *sets* `is_nsfw` at index
time, so the filter is a placeholder until a classifier exists ([[Image Pipeline]]).

## 9. Testing

`crates/xustive-vector/tests/qdrant_roundtrip.rs` (collection, upsert, search, delete against a
live Qdrant; skipped without one), unit tests for `l2_normalise`, `point_id`, hit parsing;
`xustive-ingest/tests/embed_cache_redis.rs`. Verified live on dev Qdrant with synthetic vectors on
2026-08-21. Recall probes and the fixture-image round-trip remain to be built.

## 10. Open Questions

- [ ] Measure recall@10 vs `ef` on our own images; the 0.90/0.95/0.98 table is a hypothesis.
- [ ] Is 0.75 the right CLIP threshold here? Needs data.
- [ ] Text→image search using CLIP's text tower — the model is loaded, the endpoint is not.
- [ ] Cross-language dedup on the text collection ([[Deduplication Service]] §9).
- [ ] One embedding per video cover, or per keyframe? Video is metadata-only today
      ([[Media Extraction]]).

## Related

[[Image Pipeline]] · [[Enrichment Pipeline]] · [[Query Pipeline]] · [[Search Index]] ·
[[Deduplication Service]] · [[Data Model]] · [[Deployment Topology]] · [[UI - Image Search]] ·
[[Milestone 3 - Multimodal Input]] · [[Milestone 7 - Federated Retrieval and External Tools]]
