---
tags: [adr]
adr-id: "0016"
status: accepted
date: 2026-08-21
---
# ADR-0016 - Two OCR Engines with an Optional Unlimited-OCR Sidecar

## Status

Accepted. Constrains [[Image Pipeline]] and [[Milestone 3 - Multimodal Input]] (the OCR track and [[UI - Image Search]]), extends [[ADR-0001 - Two-Plane Architecture]], and relates to the hardware target recorded for the reference machine (Quadro T1000 4 GB, CPU-capable).

## Context

OCR is used in two very different places. It is an **ingestion** feature — every crawled image is read so the text inside it becomes searchable, running over the whole corpus on the reference hardware — and it is a **user-facing** feature: a standalone image-to-text tool and a "photograph → search" (Lens-style) flow, one image at a time, latency-tolerant, quality-prioritised.

The always-on ingestion path already uses **tesseract** in-process (`xustive-media`, [[ADR-0001 - Two-Plane Architecture]]-clean, zero-disk-write). It fits the CPU-only hardware and needs no model beyond the `*.traineddata` files. But glyph OCR is weak on layout, tables, multi-page documents, and handwriting — exactly where a user pointing a camera at a document wants the most help.

A much stronger engine exists and was requested: Baidu's **Unlimited-OCR**, a 3B-parameter vision-language model (MIT licence). It parses whole-page layout in one shot. Its cost is hardware and runtime: 3B params in bf16 ≈ 6 GB of weights — it does **not** fit the 4 GB reference GPU, and on CPU a 3B VLM is minutes per image. It is also PyTorch/Transformers, not Rust.

Forcing every crawled image through that model would break the CPU-only target and make ingestion untenable at corpus scale. Refusing it entirely would leave the user-facing tools weaker than they need to be, and ignores an explicit preference for capable open Chinese models.

## Decision

**Keep tesseract as the always-available default; add Unlimited-OCR as an optional sidecar the user-facing tools can prefer. One trait, two engines, automatic fallback.**

- **`OcrBackend` trait** (`xustive-media::backend`) abstracts the engine. `Tesseract` runs in-process on the blocking pool. `Sidecar` is a thin HTTP client for the Unlimited-OCR service.
- **The crawl-time enrichment path always uses tesseract**, regardless of configuration. It runs over every image and must fit CPU-only hardware; the heavy model is never in that path.
- **The user-facing OCR endpoint (`POST /api/v1/ocr`) uses the selected backend.** `media.ocr_backend = "tesseract" | "unlimited"`; `"unlimited"` wraps the sidecar in a `Fallback` over tesseract, so a down, slow, or mis-configured sidecar **degrades to tesseract instead of failing**. Selecting the sidecar never turns OCR off.
- **The sidecar is an internal-service call, not internet egress.** It runs as a separate Python/GPU process on the private network (`services/ocr-sidecar`, FastAPI). The serving plane reaching it over HTTP is the same category as reaching Meilisearch or Redis — [[ADR-0001 - Two-Plane Architecture]]'s sealed-against-the-public-internet invariant holds. It is kept out of the base `deploy/docker-compose.yml` (which must come up on CPU-only hosts) and wired in behind a compose profile where a GPU exists.
- **Zero-persist on both engines.** Tesseract reads bytes in memory ([[Security and Privacy]] P4, by construction). The sidecar's model API takes a file path, so it writes each image to a private temp file and deletes it in a `finally` — the same posture, enforced by cleanup because the model leaves no path-free option.

The two user-facing surfaces ship on this: the **standalone image-to-text tool** (`/[lang]/tools/ocr`) and a **search-by-image entry** in the search box. Both downscale to ≤ 2048 px and strip EXIF (GPS included) in the browser before upload, and both put the recognised text in an **editable box that is never auto-submitted** to search ([[UI - Image Search]] M3-T06.4) — OCR guesses, and a wrong guess must not become a search on its own.

## Consequences

**Good**
- The CPU-only reference target is untouched: the default and the entire ingestion path stay on tesseract.
- The heavy model is available where it helps most (user-facing document reading) without being mandatory anywhere.
- Fallback makes the sidecar safe to enable — the worst case of a GPU outage is degraded quality, not a broken feature.
- The serving plane keeps its no-public-internet invariant; the sidecar is a private-network service.

**Bad**
- Two engines to reason about, and a wire contract to keep stable between the Rust client and the Python service.
- The sidecar needs a real GPU (≥ 8 GB VRAM) and a Python/PyTorch runtime — a heavier operational surface than the rest of the (Rust) system. It is opt-in precisely so this cost is only paid where wanted.
- "Confidence" is not comparable across engines: tesseract reports per-word confidence, the VLM does not, so the sidecar's output is treated as high-confidence and filtered by length. The UI shows confidence qualitatively, never as a cross-engine number.

## Alternatives considered

- **Replace tesseract entirely with Unlimited-OCR.** Rejected: breaks the CPU-only target for the ingestion path and makes corpus-scale OCR untenable without a GPU on every deployment.
- **Sidecar only, no fallback.** Rejected: a GPU service that is down would turn OCR off for the tools; fallback costs little and removes that failure mode.
- **Quantise the VLM to fit 4 GB.** Not pursued now: the model ships bf16 with no ready GGUF/int4 path, and quantising a VLM well is its own project. The trait leaves room to add a quantised backend later without touching callers.
