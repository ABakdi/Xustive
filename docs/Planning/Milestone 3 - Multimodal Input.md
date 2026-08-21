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

## Status as of 2026-08-21 — the image OCR track is up

The **image OCR pipeline and index-side enrichment are live.** Started with the "crawled images become
searchable text" half of the goal, since tesseract is installed on the reference machine and it needs
no model beyond the traineddata.

Done: the **`xustive-media` crate** with an in-memory OCR pipeline (decode + pixel-budget bomb guard,
EXIF auto-orient/strip with GPS never read, upscale/grayscale, tesseract `ara+fra+eng`, confidence +
usability scoring, `xustive-text` normalisation) — no file ever touches disk (P4 holds by
construction), verified reading a rendered screenshot back verbatim (M3-T04.1/.2/.5/.6, partials on
.3/.4/.7). And **index-side image enrichment** (`media_ocr` in `xustive-ingest`): the crawler fetches
a page's images (SSRF-guarded, size-capped, bounded per doc), OCRs them, fills `media[].ocr_text`
(searchable, weighted below body), backfills a thin body, and **a failed image never fails its
document** (M3-T07.1/.4/.5/.7). Opt-in via `[media] image_ocr_enabled`, default off.

Blocked on model/data provisioning (like M2's social track), not code: **voice/STT** (M3-T02/T03)
needs a **whisper model** (none present) and the **audio fixture corpus** (B7); the formal **CER
target** (M3-T04.8) needs a **labelled screenshot ground-truth set**. **CLIP + Qdrant** (M3-T05) is
buildable next — Qdrant already runs in dev — and needs a CLIP model. Remaining OCR follow-ups:
per-image phash dedup (T07.2), NSFW scoring (T07.6), PSM 6/11 retry and adaptive threshold (T04.3/.4).

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

- [ ] M3-T02.1 Magic-byte codec sniffing; reject non-allowlisted formats
- [ ] M3-T02.2 `symphonia` decode in `spawn_blocking` with wall-clock and sample-count caps
- [ ] M3-T02.3 Resample to 16 kHz mono; loudness normalisation
- [ ] M3-T02.4 VAD trimming; reject below `min_speech_ms`
- [ ] M3-T02.5 `whisper.cpp` FFI integration, `small` q5_1, Arabic-preferred language selection
- [ ] M3-T02.6 **Artefact filter** for hallucinated trailing text on near-silent input
- [ ] M3-T02.7 Bounded queue and slot management
- [ ] M3-T02.8 **Zero-disk-write test**: no new files after a request ([[Security and Privacy]] P4)
- [ ] M3-T02.9 Robustness suite: silence, noise, truncated container, 1-sample file, 30 s clip
- [ ] M3-T02.10 **WER evaluation**: 100 Algerian recordings — ar ≤ 25 %, fr ≤ 20 %, ary ≤ 45 %

## M3-T03 — [[UI - Voice Search]]

- [ ] M3-T03.1 Capability detection; hide the button when unsupported
- [ ] M3-T03.2 Permission on tap only, never on load; per-browser re-enable guidance
- [ ] M3-T03.3 Recording overlay: waveform, timer, stop, cancel; focus-trapped `<dialog>`
- [ ] M3-T03.4 `MediaRecorder` Opus capture at 24 kbps mono
- [ ] M3-T03.5 Auto-stop on 2 s silence; 30 s hard cap announced at 25 s
- [ ] M3-T03.6 **Transcript lands in the search box, editable, not auto-submitted**
- [ ] M3-T03.7 Track stop on completion/cancel so the browser mic indicator clears
- [ ] M3-T03.8 Retry keeps the recorded blob in memory (3G resilience)
- [ ] M3-T03.9 `prefers-reduced-motion` variant of the waveform
- [ ] M3-T03.10 Live-region announcements for every state transition

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

- [ ] M3-T05.1 Qdrant collection with int8 quantisation and payload indexes
- [ ] M3-T05.2 `rust-bert` CLIP ViT-B/32 image tower; L2 normalisation
- [ ] M3-T05.3 dHash `phash` + Redis `phash → embedding_id` reuse map
- [ ] M3-T05.4 Upsert batching from [[Indexer Worker]]
- [ ] M3-T05.5 ANN search with payload filters; `ef` tuning
- [ ] M3-T05.6 **Measure recall vs latency on our own corpus** — the table in
      [[Vector Index]] §4 is a hypothesis until this runs
- [ ] M3-T05.7 Orphan reconciliation job (deleted document → deleted vectors)
- [ ] M3-T05.8 Round-trip test: re-uploaded image ranks 1 at > 0.95
- [ ] M3-T05.9 Transform robustness: crop / resize / recompress / watermark → top 3

## M3-T06 — [[UI - Image Search]]

- [ ] M3-T06.1 Input methods: camera, file picker, paste, drag & drop
- [ ] M3-T06.2 Client-side downscale to 2048 px + EXIF strip before upload
- [ ] M3-T06.3 Optional crop step, skippable
- [ ] M3-T06.4 OCR result panel — **text into the search box, not auto-submitted**
- [ ] M3-T06.5 Similarity grid with qualitative labels, not raw scores
- [ ] M3-T06.6 Determinate upload progress; cancellable; blob retained for retry
- [ ] M3-T06.7 Privacy statement at the preview step
- [ ] M3-T06.8 Keyboard equivalent for drag & drop; accessible names on tiles

## M3-T07 — Index-side media enrichment

- [x] M3-T07.1 Media fetch in [[Enrichment Pipeline]] via `SafeUrl`, size-capped, bounded concurrency
- [ ] M3-T07.2 Skip fetch when `phash` is already known ([[Deduplication Service]] §4.4)
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
[[UI - Image Search]] · [[Security and Privacy]]
