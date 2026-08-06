---
tags:
  - component
  - serving
  - ml
component-id: C10
binary: xustive-ml
status: specified
updated: 2026-08-06
---

# Image Pipeline

> **ID** C10 · **Binary** `xustive-ml` · **Upstream** [[API Gateway]] (`POST /search/image`), [[Enrichment Pipeline]] · **Downstream** [[Vector Index]]

## 1. Purpose

Two jobs sharing one set of models:

1. **Query side** — a user uploads or photographs an image; we extract its text (OCR) or find visually
   similar indexed images.
2. **Index side** — every crawled image gets OCR'd and embedded so it is findable at all.

The Algerian use case that drives this: screenshots. Announcements, job postings, official notices,
and price lists circulate as images in Facebook groups, not as text. Without OCR, that entire body of
content is invisible to a text search engine.

## 2. Responsibilities

**In scope**: decode + EXIF strip + resize; OCR (Arabic/French/English); CLIP embedding; perceptual
hash; NSFW flagging; similarity search against [[Vector Index]]; mode selection (`auto`).

**Out of scope**: face recognition (**explicitly never** — see §10); object detection; video frame
extraction beyond a single thumbnail.

## 3. Interface

```rust
pub trait ImageProcessor: Send + Sync {
    async fn analyze(&self, bytes: &[u8], mode: Mode) -> Result<Analysis, ImageError>;
    async fn embed(&self, bytes: &[u8]) -> Result<[f32; 512], ImageError>;   // index side
    async fn ocr(&self, bytes: &[u8], lang_hint: Option<Lang>) -> Result<Ocr, ImageError>;
}
pub enum Mode { Auto, Ocr, Similar }
pub struct Analysis { pub mode_used: Mode, pub ocr: Option<Ocr>, pub similar: Vec<SimilarHit>, pub phash: u64 }
```

Public contract in [[API Contract]] §6.

## 4. Internal Design

### 4.1 Common preprocessing

1. Magic-byte type check (`jpeg`, `png`, `webp`); reject others with 415.
2. Decode with the `image` crate under a pixel budget (≤ 40 MP) and a 5 s timeout inside
   `spawn_blocking` — the standard decompression-bomb guard ([[Security and Privacy]] §6).
3. **Strip EXIF immediately** after decode. GPS coordinates in a user upload are never read, and
   never leave the decode function.
4. Auto-orient from the EXIF orientation flag *before* stripping.
5. Produce two derivatives: `ocr_input` (upscaled to ≥ 1000 px on the short edge, grayscale,
   adaptive threshold) and `clip_input` (224 × 224 centre crop, normalised).

### 4.2 OCR

`tesseract-rs` with `ara+fra+eng` traineddata, `--psm 6` (uniform block) with a `--psm 11` (sparse
text) retry when the first pass yields < 8 characters.

Post-processing:
- Drop tokens with per-character confidence < 60.
- Apply `xustive-text` normalisation (same function as [[Content Parser]] and [[Query Pipeline]]).
- Compute a **usability score**: `usable = chars ≥ 8 && mean_confidence ≥ 0.55`.

Arabic OCR quality is the weak point — screenshots (rendered fonts, high contrast) do well;
photographed signage does poorly. This is acceptable: the dominant use case *is* screenshots.

### 4.3 CLIP embedding

`rust-bert` CLIP ViT-B/32 image tower → 512 floats → L2-normalise. Deterministic given the same
input, which matters for the index/query symmetry that makes similarity search work at all.

### 4.4 Perceptual hash

64-bit dHash. Used twice: as an exact/near-duplicate shortcut before ANN search (a re-upload of an
already-indexed image is answered from a Redis `phash → document_id` map in ~1 ms), and by
[[Deduplication Service]] at index time.

### 4.5 `auto` mode decision

```
phash exact hit?          → return that document as similar[0], plus ANN for the rest
OCR usable?               → mode_used = "ocr"; return ocr.text (client searches it)
otherwise                 → mode_used = "similar"; embed + ANN search [[Vector Index]]
```

### 4.6 Similarity search

Query [[Vector Index]] with `limit 40`, `score_threshold 0.75`, `is_nsfw = false`. Collapse hits by
`document_id` keeping the best score. Return top 20 with a join back to [[Search Index]] for title,
URL, and date.

### 4.7 Index-side path

Called by [[Enrichment Pipeline]] per media item: fetch (through [[Proxy Manager]], size-capped) →
preprocess → OCR → embed → phash → NSFW score. Results are written into `Document.media[]` and the
embedding is queued for [[Vector Index]] upsert. Images that fail to fetch or decode leave the
document intact with `media[].ocr_text = null` — **a bad image never fails a document**.

## 5. Configuration

| Key | Default |
|:---|:---|
| `max_bytes` | 8 MiB |
| `max_pixels` | 40 MP |
| `max_dimension` | 4096 |
| `decode_timeout_ms` | 5000 |
| `ocr_langs` | `ara+fra+eng` |
| `ocr_min_chars` | 8 |
| `ocr_min_confidence` | 0.55 |
| `clip_model_path` | `/models/clip-vit-b32.ot` |
| `ann_limit` | 40 |
| `ann_score_threshold` | 0.75 |
| `nsfw_threshold` | 0.85 |
| `index_side_max_images_per_doc` | 4 |

## 6. Data

Query side: stores nothing ([[Security and Privacy]] P4). Index side: writes `media[].ocr_text`,
`ocr_lang`, `phash`, `embedding_id` into the `Document` ([[Data Model]]), and a point into
[[Vector Index]].

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Unsupported/corrupt file | magic bytes / decode error | 415 or 422 `image_unreadable` |
| Decompression bomb | pixel budget | 422, `WARN` metric |
| Decode timeout or panic | task boundary | 422; process survives |
| OCR yields nothing | usability score | fall through to similarity mode |
| Tesseract data missing | startup | **Fatal** |
| CLIP model missing | startup | **Fatal** |
| Qdrant down | call error | OCR still works; similarity returns 503 |
| Index-side image fetch fails | HTTP error | skip that media item, keep the document |
| NSFW model unavailable | flag | `is_nsfw = null`; filter conservatively (exclude from image results) |

## 8. Performance

| Path | Budget |
|:---|:---|
| Whole `/search/image` | ≤ 500 ms p95 ([[Performance Budgets]]) |
| Decode + preprocess | ≤ 80 ms |
| OCR (1000 px) | ≤ 250 ms |
| CLIP embed | ≤ 60 ms |
| ANN search | ≤ 40 ms |
| phash shortcut hit | ≤ 5 ms |
| Index-side per image | ≤ 400 ms (throughput, not latency, bound) |

## 9. Observability

`xustive_image_duration_seconds{stage,mode}`, `xustive_ocr_chars`, `xustive_ocr_confidence`,
`xustive_image_rejected_total{reason}`, `xustive_phash_hit_total`,
`xustive_image_mode_total{mode_used}`. `ocr_text` from a *user upload* is query content and must
never be logged; `ocr_text` from a *crawled* image is corpus content and may appear in debug logs.

## 10. Security

- Untrusted binary input — see the guards in §4.1 and [[Security and Privacy]] §6.
- **EXIF/GPS is stripped and never read.** No location is derived from user uploads.
- **No face recognition, no person clustering, no reverse-search-by-face.** This is a deliberate,
  permanent scope exclusion: a reverse image search that identifies people is a surveillance tool,
  and it is not what this product is. CLIP embeddings are whole-image and are not used for identity
  matching.
- NSFW filtering is default-on for image results.
- Index-side fetches go through `SafeUrl` validation ([[Security and Privacy]] §4).

## 11. Testing

- Fixtures: 200 images — Arabic screenshots, French documents, photos, memes, low-quality phone
  shots, adversarial (bomb, truncated, wrong extension, 1×1 px, CMYK JPEG).
- OCR accuracy: character error rate ≤ 15 % on the screenshot subset (the case we actually promise).
- Round-trip: index an image, re-upload it, assert rank 1 with score > 0.95.
- Transform robustness: crop 10 %, resize 50 %, JPEG q40, watermark → still top 3.
- EXIF: upload an image with GPS; assert no coordinate value appears in any output, log, or store.
- Security: every adversarial fixture returns a 4xx within the timeout with no panic.

## 12. Open Questions

- [ ] Add a text→image CLIP path ("find images of…") using the text tower we already ship?
- [ ] Video: one embedding per TikTok cover image, or keyframes? Cover-only for v1.
- [ ] Which NSFW model, given licence constraints? (`open_nsfw` variants — check licence terms)
- [ ] Should index-side OCR text be a separate searchable attribute weighted below `body`?

## Related

[[Vector Index]] · [[Enrichment Pipeline]] · [[UI - Image Search]] · [[API Contract]] ·
[[Security and Privacy]] · [[Deduplication Service]]
