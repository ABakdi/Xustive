---
tags:
  - component
  - serving
  - ml
component-id: C10
binary: xustive-api · xustive-cli crawld (crates/xustive-media)
status: built (index side opt-in)
updated: 2026-08-27
---

# Image Pipeline

> **ID** C10 · **Runs in** `xustive-api` (`POST /api/v1/ocr`, `POST /api/v1/search/image`) and
> the `crawld` parse path (`media_ocr.rs`, `media_embed.rs`) · **Library** `crates/xustive-media`
> · **Upstream** [[API Gateway]], [[Enrichment Pipeline]] · **Downstream** [[Vector Index]],
> [[Search Index]]

## 1. Purpose

Two jobs sharing one set of tools:

1. **Query side** — a person uploads or photographs an image; we read its text (OCR) or find
   visually similar indexed images.
2. **Index side** — a crawled page's images get OCR'd, hashed and embedded so they are findable at
   all.

The Algerian use case that drives this: screenshots. Announcements, job postings, official notices
and price lists circulate as images in Facebook groups, not as text. Without OCR that entire body
of content is invisible to a text search engine.

Finding and listing the images themselves (the Images tab, thumbnails) is [[Media Extraction]] and
[[Thumbnail Proxy]]; this note is about reading and embedding pixels.

## 2. What exists today

| Piece | Path | Built? |
|:---|:---|:---|
| Decode, auto-orient, preprocess, tesseract, score | `crates/xustive-media/src/ocr.rs` | yes |
| OCR backends: `Tesseract`, `Sidecar` (Unlimited-OCR), `Fallback` | `crates/xustive-media/src/backend.rs` | yes |
| dHash | `crates/xustive-media/src/phash.rs` | yes |
| `POST /api/v1/ocr` | `crates/xustive-api/src/ocr.rs` | yes |
| `POST /api/v1/search/image` | `crates/xustive-api/src/image_search.rs` | yes (off unless `[vector] enabled`) |
| Index-side OCR + body backfill | `xustive-ingest/src/media_ocr.rs` | yes (opt-in) |
| Index-side CLIP embed + pHash cache | `xustive-ingest/src/media_embed.rs`, `embed_cache.rs` | yes (opt-in) |
| Unlimited-OCR sidecar | `services/ocr-sidecar` | yes, GPU-only, opt-in |
| `auto` mode (OCR-or-similar in one call) | — | **not built** (2026-08-27) — the client picks the endpoint |
| pHash exact-hit shortcut before ANN | — | **not built** (2026-08-27) |
| NSFW classifier | — | **not built** (2026-08-27); `is_nsfw` is never set, the filter is a placeholder |
| EXIF/GPS handling | orientation read, nothing else read; bytes never written | yes by construction |

## 3. Interface

```
POST /api/v1/ocr            body: raw image bytes (Content-Type: image/*)
  200 { "text", "usable", "confidence" (0–100), "backend": "tesseract" | "unlimited" }
  400 empty_image | image_too_large | undecodable_image
  503 ocr_unavailable

POST /api/v1/search/image   body: raw image bytes
  200 { "results": [ResultCard…], "matched_images": n }
  400 empty_image | image_too_large
  503 image_search_unavailable      (feature off, sidecar or Qdrant down, undecodable)
```

A raw body, not multipart: exactly one image is sent, and a form wrapper would only add a parser
and a failure mode; it also keeps the image out of any URL, referrer or access log. Both routes
carry their own body limit (`[media] max_image_bytes`, 5 MiB) so the global 8 KB default does not
apply, and both sit under the `MEDIA` rate-limit class. OCR text is **never** auto-submitted to
search — the person sees and edits it first ([[UI - Image Search]]).

```rust
// crates/xustive-media
pub trait OcrBackend { fn name(&self) -> &'static str; async fn recognise(&self, bytes: Vec<u8>) -> Result<Ocr, OcrError>; }
pub struct Ocr { pub text: String, pub confidence: f32, pub usable: bool }
pub fn ocr::recognise(bytes, tessdata, langs, max_pixels) -> Result<Ocr, OcrError>  // blocking
pub fn phash::dhash(bytes, max_pixels) -> Option<String>;  phash::hamming(a, b) -> Option<u32>
```

## 4. Internal Design

### 4.1 Decode and preprocess (`ocr.rs`)

Dimensions are read from the header first, so a decompression bomb is refused before it is ever
expanded (`MAX_PIXELS = 40 M`). Decode with the `image` crate, apply the EXIF orientation flag,
upscale so the short edge is ≥ 1000 px, grayscale. Everything stays in memory — Leptonica reads
the encoded bytes via `set_image_from_mem` and there is no path that opens a file, which is how the
zero-disk-write rule ([[Security and Privacy]] P4) holds by construction. `recognise` is blocking
and CPU-bound; every caller runs it on `spawn_blocking`.

### 4.2 OCR

`leptess` (tesseract) with `ara+fra+eng`. Two passes at most: the first reads the grayscale image
(tesseract binarises internally, right for clean anti-aliased screenshot text). When that is
unusable or below `GOOD_ENOUGH_CONFIDENCE = 75`, a second pass reads an **Otsu-binarised**
version — the threshold comes from the image's own histogram, so it adapts per source — and the
better result wins, so the retry can only help. `usable = chars ≥ 8 && mean confidence ≥ 55`.
There is no `--psm` retry and no per-character token dropping; the confidence gate does that job.

Arabic OCR is the weak point — screenshots do well, photographed signage poorly. Acceptable,
because the dominant use case *is* screenshots.

### 4.3 Two engines ([[ADR-0016 - Two OCR Engines with an Optional Unlimited-OCR Sidecar]])

`Tesseract` runs in-process on the CPU and needs only the traineddata, so it is always available
and fits the reference hardware (a 4 GB GPU, or none). `Sidecar` is an HTTP client for
`services/ocr-sidecar` — `baidu/Unlimited-OCR`, a 3 B vision-language model that parses layout and
tables far better but needs a real GPU (`OCR_MODEL`, `OCR_PROMPT="<image>document parsing."`,
`OCR_MAX_BYTES` 8 MiB; `POST /ocr`). It reports an assumed confidence of 90 because the model does
not score itself. `Fallback` prefers the sidecar and drops to tesseract when it errors, so turning
the sidecar on never turns a feature off. `[media] ocr_backend = "unlimited"` selects it for the
**user-facing** routes only; crawl-time enrichment always uses tesseract — it must fit CPU-only.

### 4.4 CLIP embedding and similarity

`SidecarEmbedder` → `services/clip-embed` (CLIP ViT-B/32, 512-d, CPU-capable) → L2-normalised →
Qdrant. The search collapses hits by `document_id`, keeps the best score, resolves cards from the
lexical index and re-orders by similarity. Limits, `ef`, threshold and the pHash reuse cache are
in [[Vector Index]].

### 4.5 Perceptual hash

64-bit dHash: grayscale, resize to 9×8, one bit per adjacent-pixel brightness comparison —
gradients rather than absolute brightness, which is what survives an exposure or contrast shift.
Stamped into `media[].phash` by whichever index-side pass fetched the bytes first, and used as the
key of the CLIP reuse cache. The Redis `phash → document_id` shortcut before ANN is not built.

### 4.6 Index side

Runs in `Orchestrator::step` after parse ([[Enrichment Pipeline]] §4.2): one `ImageFetcher`
(own client, `XustiveBot/1.0 (…; media OCR)`, 15 s, `SafeUrl`, `image/*` only, size-capped on the
declared length and again on the bytes), at most `max_images_per_doc` per pass. OCR fills
`media[].ocr_text` / `ocr_lang` and backfills a body under 20 words. **A bad image never fails a
document.** There is no separate media fetch step and no [[Proxy Manager]] in this path.

## 5. Configuration (`[media]`)

| Key | Dev default | Meaning |
|:---|:---|:---|
| `image_ocr_enabled` | `false` | index-side OCR |
| `tessdata_dir` | `data/tessdata` | needs `ara`, `fra`, `eng` traineddata |
| `ocr_langs` | `ara+fra+eng` | |
| `max_images_per_doc` | 3 | OCR and embed passes each |
| `max_image_bytes` | 5 MiB | uploads and index-side fetches |
| `ocr_backend` | `tesseract` | `unlimited` = sidecar with fallback, user routes only |
| `[media.sidecar] endpoint`, `timeout_ms` | `http://127.0.0.1:8091/ocr`, 30 000 | |

Constants: `MAX_PIXELS` 40 M, `MIN_OCR_DIM` 1000, `MIN_CONFIDENCE` 55, `MIN_USABLE_CHARS` 8,
`GOOD_ENOUGH_CONFIDENCE` 75, `THIN_WORDS` 20.

## 6. Failure Modes

| Failure | Response |
|:---|:---|
| Unknown format / corrupt | `OcrError::Format` / `Decode` → 400 `undecodable_image` |
| Over the pixel budget | `TooLarge` → 400 `image_too_large`, refused before expansion |
| Tesseract init (missing traineddata) or sidecar 500 | `Engine` → 503 `ocr_unavailable`; cause logged, never the image |
| Sidecar down with `ocr_backend = unlimited` | `Fallback` → tesseract, `backend` says which |
| Sidecar or Qdrant down for similarity | 503 `image_search_unavailable`; text search untouched |
| Index-side fetch/decode/OCR/embed error | that image skipped |

## 7. Security

Untrusted binary input: header-first pixel budget, size caps at the route and again in the
handler, in-memory only. EXIF is consulted for orientation and nothing else; GPS is never read and
no coordinate can leave `decode`. The uploaded image and its text are the payload and are never
logged. **No face recognition, no person clustering, ever** — a reverse image search that
identifies people is a surveillance tool, and CLIP is used whole-image only. Index-side fetches
go through `SafeUrl` ([[Security and Privacy]]). The sidecar is an internal-network call, not
egress ([[Deployment Topology]]).

## 8. Observability

None specific yet. The `backend` field in the OCR reply lets an operator confirm the sidecar is
actually in the path; `matched_images` in the image-search reply shows the pre-collapse count.

## 9. Testing

`crates/xustive-media/tests/ocr_fixture.rs` with fixtures under `tests/fixtures`; dHash and Otsu
unit tests; `xustive-ingest/tests/ssrf.rs` for the fetcher; `state.rs` tests that the backend
defaults to tesseract. The 200-image fixture set, CER target and transform-robustness suite from
the original plan are not built.

## 10. Open Questions

- [ ] An NSFW model with an acceptable licence; until then the `is_nsfw` filter filters nothing.
- [ ] Text→image CLIP search.
- [ ] Should index-side `ocr_text` be a searchable attribute weighted below `body`, rather than
      only backfilling thin bodies? ([[Search Index]] settings)
- [ ] Video: cover image only, if ever — video is metadata-only today ([[Media Extraction]]).

## Related

[[Vector Index]] · [[Enrichment Pipeline]] · [[Media Extraction]] · [[Thumbnail Proxy]] ·
[[Deduplication Service]] · [[UI - Image Search]] · [[API Contract]] · [[Security and Privacy]] ·
[[ADR-0016 - Two OCR Engines with an Optional Unlimited-OCR Sidecar]] ·
[[Milestone 3 - Multimodal Input]]
