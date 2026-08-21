---
tags:
  - planning
  - milestone
milestone: 3
status: in-progress
updated: 2026-08-21
---

# Milestone 3 - Multimodal Input

> **Goal:** voice and image become real input methods, and crawled images become searchable text.
> **Exit gate:** WER within targets; screenshot OCR CER ≤ 15 %; `/search/image` p95 ≤ 500 ms; no
> regression to text search latency.
> Parent: [[TODO]] · Previous: [[Milestone 1 - Text Search MVP]] · Previous: [[Milestone 2 - Ingestion at Scale]]

---

## Status as of 2026-08-21 — the image OCR track is up, and image *input* is a real feature

The **image OCR pipeline, index-side enrichment, and the user-facing image tools are live.** Started
with the "crawled images become searchable text" half of the goal, since tesseract is installed on the
reference machine and it needs no model beyond the traineddata; then built the user-facing half on top.

Done (ingestion): the **`xustive-media` crate** with an in-memory OCR pipeline (decode + pixel-budget
bomb guard, EXIF auto-orient/strip with GPS never read, upscale/grayscale, tesseract `ara+fra+eng`,
confidence + usability scoring, `xustive-text` normalisation) — no file ever touches disk (P4 holds by
construction), verified reading a rendered screenshot back verbatim (M3-T04.1/.2/.5/.6, partials on
.3/.4/.7). And **index-side image enrichment** (`media_ocr` in `xustive-ingest`): the crawler fetches
a page's images (SSRF-guarded, size-capped, bounded per doc), OCRs them, fills `media[].ocr_text`
(searchable, weighted below body), backfills a thin body, and **a failed image never fails its
document** (M3-T07.1/.4/.5/.7). Opt-in via `[media] image_ocr_enabled`, default off.

Done (input, M3-T06): an **`OcrBackend` trait** ([[ADR-0016 - Two OCR Engines with an Optional
Unlimited-OCR Sidecar]]) with two engines — in-process tesseract (the CPU-only default and the whole
ingestion path) and an optional **Unlimited-OCR sidecar** (a 3B vision-language model, `services/
ocr-sidecar`) preferred for the tools when `media.ocr_backend = "unlimited"`, with automatic fallback
to tesseract. **`POST /api/v1/ocr`** takes a raw image body and returns the text. The **standalone
image-to-text tool** (`/[lang]/tools/ocr`) and a **search-by-image entry** in the search box are both
live: file picker, camera capture, paste and drag-drop; browser-side downscale to ≤ 2048 px with EXIF
(GPS) stripped before upload; and the recognised text lands **editable and is never auto-submitted**
(M3-T06.1/.2/.4/.7).

Done (visual similarity, M3-T05): the **`xustive-vector` crate** — a lean Qdrant REST client
(collection with int8 quantisation, cosine on L2-normalised vectors, payload indexes, ANN search
with NSFW filtering, `delete_by_document` for takedowns), **verified live end-to-end against dev
Qdrant** with synthetic vectors (exact match ranks first at score > 0.99). A **CLIP embedder** trait
+ sidecar client, and the **`clip-embed` service** (CLIP ViT-B/32, CPU-capable — *not* GPU-gated).
The **write path** (`media_embed`): the crawler embeds a page's images and upserts them, opt-in and
failure-isolated. The **read path**: **`POST /api/v1/search/image`** embeds an upload, ANN-searches,
collapses by document, and resolves the documents for display — and the image tool's **"Find similar
images"** renders them with qualitative match labels. All isolated: vector search being down never
touches text search. See [[ADR-0016 - Two OCR Engines with an Optional Unlimited-OCR Sidecar]] for
the shared sidecar pattern.

Blocked on model/data provisioning (like M2's social track), not code: **voice/STT** (M3-T02/T03)
needs a **whisper model** (none present) and the **audio fixture corpus** (B7); the formal **CER
target** (M3-T04.8) needs a **labelled screenshot ground-truth set**; the **Unlimited-OCR sidecar**
needs a **GPU box (≥ 8 GB) with the model** to run live — the Rust side and the service are done and
fall back to tesseract until then; **image similarity** needs the **CLIP model provisioned** into the
`clip-embed` service before it returns real results (the whole path is wired and tested, off by
default via `[vector] enabled`). Orphan reconciliation is done as `xustive-cli reconcile-vectors`
(a removed document's image vectors are deletable, closing the [[Security and Privacy]] §8 gap).
The phash reuse-skip (T05.3) is done — a re-posted image reuses its embedding via the Redis
`embed_cache`. Remaining vector follow-up: the recall/latency measurement (T05.6, needs the model +
a corpus). Remaining OCR follow-ups: the fetch-skip on a known phash (T07.2), NSFW scoring (T07.6),
PSM 6/11 retry and adaptive threshold (T04.3/.4).

Done (voice, M3-T02/T03): a **speech-to-text track** built on the same sidecar pattern. The
**`stt-sidecar`** wraps Whisper `small` on faster-whisper (CTranslate2, CPU-capable — voice is not
GPU-gated) with VAD trimming and in-memory decode (no disk); **`POST /api/v1/transcribe`** forwards
audio to it with an Arabic-preferred language hint, isolated so text search never depends on it; and
the **`VoiceButton`** UI records (MediaRecorder/Opus), shows a focus-trapped recording dialog with a
30-second cap, and drops the transcript **into the search box, editable and never auto-submitted**.
Off by default (`[stt] enabled`), visible on the admin Image-AI page, blocked only on a whisper model
being provisioned. The architecture differs from the tasks below (a Python sidecar rather than
in-process `whisper.cpp`/`symphonia`), so their codec/VAD/decode sub-tasks are handled inside the
sidecar; the **WER measurement (T02.10) still needs the audio corpus (B7)**.

---

## Why This Milestone Exists

Two user realities drive it. Typing Arabic on a phone is slow, so **voice is a primary input** for a
meaningful share of the audience. And a large volume of Algerian information circulates as
**screenshots** in group chats, which are invisible to a text index until OCR reaches them.

The second point matters more than it looks: OCR is not just a query feature here, it is an
*ingestion* feature. An Instagram post with no caption and a text-heavy image is currently an empty
document ([[Social Connector - Instagram]] §4.2).

---

## M3-T01 — `xustive-ml` service

- [ ] M3-T01.1 Service scaffold: internal HTTP, health, readiness gated on model load
- [ ] M3-T01.2 Model manifest with checksums; `model-init` job populating the shared read-only volume
      ([[Deployment Topology]] §5)
- [ ] M3-T01.3 Model licence audit → `models/LICENSES.md` ([[Legal and Compliance]] §7)
- [ ] M3-T01.4 Shared admission controller so voice/image bursts cannot starve [[Summarizer]]
- [ ] M3-T01.5 Per-model memory and latency metrics
- [ ] M3-T01.6 Decide B6: is a GPU in the budget? Config path either way, no redesign

## M3-T02 — [[Speech to Text]]

> **Architecture note:** built as a **Python sidecar** (`services/stt-sidecar`, faster-whisper) rather
> than in-process `whisper.cpp`/`symphonia` — the same pattern as the OCR and CLIP sidecars, chosen to
> keep the model out of the Rust build and because whisper `small` runs CPU-only. So the decode/VAD/
> resample sub-tasks below are handled *inside* the sidecar (PyAV + whisper), not as Rust code.

- [~] M3-T02.1 Codec handling — *the sidecar decodes via PyAV/ffmpeg; the API caps size and forwards, no strict Rust-side allowlist*
- [~] M3-T02.2 Decode with caps — *in-sidecar (PyAV), not `symphonia`; wall-clock bounded by the request timeout*
- [~] M3-T02.3 Resample to 16 kHz mono — *whisper resamples internally*
- [x] M3-T02.4 VAD trimming — *`vad_filter=True` in the sidecar*
- [~] M3-T02.5 Whisper `small`, Arabic-preferred language — *faster-whisper, with the UI language forwarded as `?lang=`; not `whisper.cpp` FFI*
- [ ] M3-T02.6 **Artefact filter** for hallucinated trailing text on near-silent input
- [~] M3-T02.7 Bounded queue and slot management — *the API rate-limits `/transcribe`; the sidecar is single-model*
- [x] M3-T02.8 **Zero-disk-write** — *the sidecar decodes from a `BytesIO`, no temp file; the API forwards a raw body (test still to add)*
- [ ] M3-T02.9 Robustness suite: silence, noise, truncated container, 1-sample file, 30 s clip
- [ ] M3-T02.10 **WER evaluation**: 100 Algerian recordings — ar ≤ 25 %, fr ≤ 20 %, ary ≤ 45 % ← *needs the audio corpus (B7)*

## M3-T03 — [[UI - Voice Search]]

- [x] M3-T03.1 Capability detection; hide the button when unsupported
- [x] M3-T03.2 Permission on tap only, never on load; guidance on denial
- [~] M3-T03.3 Recording overlay: timer, stop, cancel; focus-trapped `<dialog>` — *no waveform yet*
- [x] M3-T03.4 `MediaRecorder` Opus capture at 24 kbps
- [~] M3-T03.5 30 s hard cap — *silence auto-stop and the 25 s announce not done yet*
- [x] M3-T03.6 **Transcript lands in the search box, editable, not auto-submitted**
- [x] M3-T03.7 Track stop on completion/cancel so the browser mic indicator clears
- [ ] M3-T03.8 Retry keeps the recorded blob in memory (3G resilience)
- [~] M3-T03.9 `prefers-reduced-motion` — *the recording indicator is `motion-safe`; no waveform to vary*
- [~] M3-T03.10 Live-region announcements — *state and errors are announced; not yet every transition*

## M3-T04 — [[Image Pipeline]] — OCR path

- [x] M3-T04.1 Magic-byte typing; pixel budget; decode timeout in `spawn_blocking`
- [x] M3-T04.2 **EXIF auto-orient then strip**; GPS never read
- [~] M3-T04.3 OCR preprocessing: upscale, grayscale, adaptive threshold
- [~] M3-T04.4 `tesseract-rs` with `ara+fra+eng`, `--psm 6` with `--psm 11` retry
- [x] M3-T04.5 Confidence filtering and usability scoring
- [x] M3-T04.6 Normalisation via `xustive-text`
- [~] M3-T04.7 Adversarial suite: bombs, truncated, wrong extension, 1×1, CMYK
- [ ] M3-T04.8 **CER ≤ 15 % on the screenshot subset**

## M3-T05 — CLIP and [[Vector Index]]

- [x] M3-T05.1 Qdrant collection with int8 quantisation and payload indexes
- [x] M3-T05.2 CLIP ViT-B/32 image tower; L2 normalisation ← *via the `clip-embed` sidecar, not `rust-bert`; keeps the model out of the Rust build and runs CPU-only*
- [x] M3-T05.3 dHash `phash` + Redis `phash → vector` reuse map — *dHash computed (`xustive_media::phash`) and stamped on `media.phash`; the `embed_cache` reuses a known image's embedding instead of re-calling the model, verified live against Redis. Keyed on `phash → vector` rather than `→ embedding_id` (one Redis read, no Qdrant round trip)*
- [x] M3-T05.4 Upsert batching from the crawler (index-side, `media_embed`)
- [x] M3-T05.5 ANN search with payload filters; `ef` tuning
- [ ] M3-T05.6 **Measure recall vs latency on our own corpus** — the table in
      [[Vector Index]] §4 is a hypothesis until this runs ← *needs the CLIP model + a corpus*
- [x] M3-T05.7 Orphan reconciliation job (deleted document → deleted vectors) — *`xustive-cli reconcile-vectors` walks the collection, checks each document against the index, and deletes orphans (`--dry-run` to preview); verified live. A cron/timer around it is an ops concern, not code*
- [x] M3-T05.8 Round-trip test: re-uploaded image ranks 1 — *verified live against dev Qdrant with synthetic vectors (exact match score > 0.99); with a real CLIP model this becomes the > 0.95 image test*
- [ ] M3-T05.9 Transform robustness: crop / resize / recompress / watermark → top 3 ← *needs the CLIP model*

## M3-T06 — [[UI - Image Search]]

- [x] M3-T06.1 Input methods: camera, file picker, paste, drag & drop
- [x] M3-T06.2 Client-side downscale to 2048 px + EXIF strip before upload
- [ ] M3-T06.3 Optional crop step, skippable
- [x] M3-T06.4 OCR result panel — **text into the search box, not auto-submitted**
- [x] M3-T06.5 Similarity grid with qualitative labels, not raw scores — *"Find similar images" on the tool; results as pages with a qualitative match label, never a raw score*
- [~] M3-T06.6 Determinate upload progress; cancellable; blob retained for retry ← *cancellable; progress is indeterminate*
- [x] M3-T06.7 Privacy statement at the preview step
- [x] M3-T06.8 Keyboard equivalent for drag & drop; accessible names on tiles

## M3-T07 — Index-side media enrichment

- [x] M3-T07.1 Media fetch in [[Enrichment Pipeline]] via `SafeUrl`, size-capped, bounded concurrency
- [~] M3-T07.2 Skip fetch when `phash` is already known ([[Deduplication Service]] §4.4) ← *dHash now computed and stamped on every fetched image; the "skip fetch if phash seen" registry is not wired yet*
- [ ] M3-T07.3 Prioritise Instagram media (expiring CDN URLs)
- [x] M3-T07.4 OCR text backfills `body` when the caption is empty; `body_source = "ocr"`
- [x] M3-T07.5 A failed image never fails its document
- [ ] M3-T07.6 NSFW scoring and default filtering of image results
- [x] M3-T07.7 `media[].ocr_text` searchable, weighted below `body`

## M3-T08 — Quality suites

- [ ] M3-T08.1 Audio fixture corpus (100 recordings, 4 languages) + reference transcripts ← *B7*
- [ ] M3-T08.2 Image fixture corpus (200 images) with labelled OCR ground truth
- [ ] M3-T08.3 ANN recall probe set (500 queries), nightly, alert on > 3 % drop
- [ ] M3-T08.4 All three wired into `make eval` and CI

---

## Exit Gate

| Check | Threshold |
|:---|:---|
| Voice | WER ar ≤ 25 %, fr ≤ 20 %, ary ≤ 45 %; end-to-end ≤ 1 500 ms p95 |
| OCR | CER ≤ 15 % on screenshots |
| Image search | `/search/image` p95 ≤ 500 ms |
| Vector recall | recall@10 ≥ 0.95 at the chosen `ef` |
| Privacy | zero-disk-write test passing for both audio and images; EXIF test passing |
| Isolation | text search p95 unchanged while voice/image load runs |
| Accessibility | both flows fully keyboard- and screen-reader-operable |

## Risks

| Risk | Mitigation |
|:---|:---|
| Darija WER is bad enough to feel broken | the transcript is editable, not auto-submitted; be honest in the UI; fine-tuning is a v2 item |
| Arabic OCR on photographed signage is poor | we promise the **screenshot** case, and measure that specifically |
| `xustive-ml` starves [[Summarizer]] under image load | shared admission controller (M3-T01.4) + the isolation gate |
| Model licences preclude commercial use | audited in M3-T01.3, before the model is embedded in the design |
| Face recognition creeps in as a "useful feature" | permanently out of scope, stated in [[Image Pipeline]] §10 |

## Related

[[TODO]] · [[Speech to Text]] · [[Image Pipeline]] · [[Vector Index]] · [[UI - Voice Search]] ·
[[UI - Image Search]] · [[Security and Privacy]] ·
[[ADR-0016 - Two OCR Engines with an Optional Unlimited-OCR Sidecar]]
