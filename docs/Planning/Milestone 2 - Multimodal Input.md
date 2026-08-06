---
tags:
  - planning
  - milestone
milestone: 2
status: not-started
updated: 2026-08-06
---

# Milestone 2 - Multimodal Input

> **Goal:** voice and image become real input methods, and crawled images become searchable text.
> **Exit gate:** WER within targets; screenshot OCR CER ≤ 15 %; `/search/image` p95 ≤ 500 ms; no
> regression to text search latency.
> Parent: [[TODO]] · Previous: [[Milestone 1 - Text Search MVP]] · Runs parallel to [[Milestone 3 - Ingestion at Scale]]

---

## Why This Milestone Exists

Two user realities drive it. Typing Arabic on a phone is slow, so **voice is a primary input** for a
meaningful share of the audience. And a large volume of Algerian information circulates as
**screenshots** in group chats, which are invisible to a text index until OCR reaches them.

The second point matters more than it looks: OCR is not just a query feature here, it is an
*ingestion* feature. An Instagram post with no caption and a text-heavy image is currently an empty
document ([[Social Connector - Instagram]] §4.2).

---

## M2-T01 — `xustive-ml` service

- [ ] M2-T01.1 Service scaffold: internal HTTP, health, readiness gated on model load
- [ ] M2-T01.2 Model manifest with checksums; `model-init` job populating the shared read-only volume
      ([[Deployment Topology]] §5)
- [ ] M2-T01.3 Model licence audit → `models/LICENSES.md` ([[Legal and Compliance]] §7)
- [ ] M2-T01.4 Shared admission controller so voice/image bursts cannot starve [[Summarizer]]
- [ ] M2-T01.5 Per-model memory and latency metrics
- [ ] M2-T01.6 Decide B6: is a GPU in the budget? Config path either way, no redesign

## M2-T02 — [[Speech to Text]]

- [ ] M2-T02.1 Magic-byte codec sniffing; reject non-allowlisted formats
- [ ] M2-T02.2 `symphonia` decode in `spawn_blocking` with wall-clock and sample-count caps
- [ ] M2-T02.3 Resample to 16 kHz mono; loudness normalisation
- [ ] M2-T02.4 VAD trimming; reject below `min_speech_ms`
- [ ] M2-T02.5 `whisper.cpp` FFI integration, `small` q5_1, Arabic-preferred language selection
- [ ] M2-T02.6 **Artefact filter** for hallucinated trailing text on near-silent input
- [ ] M2-T02.7 Bounded queue and slot management
- [ ] M2-T02.8 **Zero-disk-write test**: no new files after a request ([[Security and Privacy]] P4)
- [ ] M2-T02.9 Robustness suite: silence, noise, truncated container, 1-sample file, 30 s clip
- [ ] M2-T02.10 **WER evaluation**: 100 Algerian recordings — ar ≤ 25 %, fr ≤ 20 %, ary ≤ 45 %

## M2-T03 — [[UI - Voice Search]]

- [ ] M2-T03.1 Capability detection; hide the button when unsupported
- [ ] M2-T03.2 Permission on tap only, never on load; per-browser re-enable guidance
- [ ] M2-T03.3 Recording overlay: waveform, timer, stop, cancel; focus-trapped `<dialog>`
- [ ] M2-T03.4 `MediaRecorder` Opus capture at 24 kbps mono
- [ ] M2-T03.5 Auto-stop on 2 s silence; 30 s hard cap announced at 25 s
- [ ] M2-T03.6 **Transcript lands in the search box, editable, not auto-submitted**
- [ ] M2-T03.7 Track stop on completion/cancel so the browser mic indicator clears
- [ ] M2-T03.8 Retry keeps the recorded blob in memory (3G resilience)
- [ ] M2-T03.9 `prefers-reduced-motion` variant of the waveform
- [ ] M2-T03.10 Live-region announcements for every state transition

## M2-T04 — [[Image Pipeline]] — OCR path

- [ ] M2-T04.1 Magic-byte typing; pixel budget; decode timeout in `spawn_blocking`
- [ ] M2-T04.2 **EXIF auto-orient then strip**; GPS never read
- [ ] M2-T04.3 OCR preprocessing: upscale, grayscale, adaptive threshold
- [ ] M2-T04.4 `tesseract-rs` with `ara+fra+eng`, `--psm 6` with `--psm 11` retry
- [ ] M2-T04.5 Confidence filtering and usability scoring
- [ ] M2-T04.6 Normalisation via `xustive-text`
- [ ] M2-T04.7 Adversarial suite: bombs, truncated, wrong extension, 1×1, CMYK
- [ ] M2-T04.8 **CER ≤ 15 % on the screenshot subset**

## M2-T05 — CLIP and [[Vector Index]]

- [ ] M2-T05.1 Qdrant collection with int8 quantisation and payload indexes
- [ ] M2-T05.2 `rust-bert` CLIP ViT-B/32 image tower; L2 normalisation
- [ ] M2-T05.3 dHash `phash` + Redis `phash → embedding_id` reuse map
- [ ] M2-T05.4 Upsert batching from [[Indexer Worker]]
- [ ] M2-T05.5 ANN search with payload filters; `ef` tuning
- [ ] M2-T05.6 **Measure recall vs latency on our own corpus** — the table in
      [[Vector Index]] §4 is a hypothesis until this runs
- [ ] M2-T05.7 Orphan reconciliation job (deleted document → deleted vectors)
- [ ] M2-T05.8 Round-trip test: re-uploaded image ranks 1 at > 0.95
- [ ] M2-T05.9 Transform robustness: crop / resize / recompress / watermark → top 3

## M2-T06 — [[UI - Image Search]]

- [ ] M2-T06.1 Input methods: camera, file picker, paste, drag & drop
- [ ] M2-T06.2 Client-side downscale to 2048 px + EXIF strip before upload
- [ ] M2-T06.3 Optional crop step, skippable
- [ ] M2-T06.4 OCR result panel — **text into the search box, not auto-submitted**
- [ ] M2-T06.5 Similarity grid with qualitative labels, not raw scores
- [ ] M2-T06.6 Determinate upload progress; cancellable; blob retained for retry
- [ ] M2-T06.7 Privacy statement at the preview step
- [ ] M2-T06.8 Keyboard equivalent for drag & drop; accessible names on tiles

## M2-T07 — Index-side media enrichment

- [ ] M2-T07.1 Media fetch in [[Enrichment Pipeline]] via `SafeUrl`, size-capped, bounded concurrency
- [ ] M2-T07.2 Skip fetch when `phash` is already known ([[Deduplication Service]] §4.4)
- [ ] M2-T07.3 Prioritise Instagram media (expiring CDN URLs)
- [ ] M2-T07.4 OCR text backfills `body` when the caption is empty; `body_source = "ocr"`
- [ ] M2-T07.5 A failed image never fails its document
- [ ] M2-T07.6 NSFW scoring and default filtering of image results
- [ ] M2-T07.7 `media[].ocr_text` searchable, weighted below `body`

## M2-T08 — Quality suites

- [ ] M2-T08.1 Audio fixture corpus (100 recordings, 4 languages) + reference transcripts ← *B7*
- [ ] M2-T08.2 Image fixture corpus (200 images) with labelled OCR ground truth
- [ ] M2-T08.3 ANN recall probe set (500 queries), nightly, alert on > 3 % drop
- [ ] M2-T08.4 All three wired into `make eval` and CI

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
| `xustive-ml` starves [[Summarizer]] under image load | shared admission controller (M2-T01.4) + the isolation gate |
| Model licences preclude commercial use | audited in M2-T01.3, before the model is embedded in the design |
| Face recognition creeps in as a "useful feature" | permanently out of scope, stated in [[Image Pipeline]] §10 |

## Related

[[TODO]] · [[Speech to Text]] · [[Image Pipeline]] · [[Vector Index]] · [[UI - Voice Search]] ·
[[UI - Image Search]] · [[Security and Privacy]]
