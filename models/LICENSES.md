# Model licence audit

**Milestone 3 · T01.3 · relates to [[Legal and Compliance]] §7.**

Every model the system loads, its source, its licence, and what that licence permits. Models are
**not** committed to the repo — they are provisioned into the mounted volumes at deploy time
(`models/`, the `*_models` compose volumes). This file is the audit that must be true *before* a
model is provisioned for anything beyond local evaluation.

> **How to read the "Verified" column.** ✅ = confirmed against the model's own page on the date
> shown. ⚠️ = the licence is known in general but the *specific artefact's* terms must be confirmed
> at provisioning, because a model card's licence field is the authority and it can change or differ
> per repo/quantisation. **Do not ship a ⚠️ row commercially without confirming it.**

Audited: 2026-08-21.

## Summariser / translator ([[Summarizer]], `xustive-ml`)

| Model | Purpose | Source | Licence | Commercial use | Verified |
|:---|:---|:---|:---|:---|:---|
| **Qwen2.5-3B-Instruct** (GGUF) | summary/translate (default present file) | Qwen (Alibaba) | **`qwen-research`** | **NO — research/non-commercial** | ✅ 2026-08-21 |
| **Qwen2.5-1.5B-Instruct** (GGUF) | summary/translate (lighter option) | Qwen (Alibaba) | Apache-2.0 (Qwen2.5 series) | yes | ⚠️ confirm on the repo |
| Qwen2.5-7B-Instruct (GGUF) | summary/translate (if provisioned) | Qwen (Alibaba) | Apache-2.0 (Qwen2.5 series) | yes | ⚠️ confirm on the repo |

> **⚠️ FINDING — the default summariser model is not commercially licensed.** The `models/` volume
> currently carries `qwen2.5-3b-instruct-q4_k_m.gguf`, and Qwen2.5-**3B** is released under the
> **qwen-research** licence (non-commercial), *unlike* the 0.5B/1.5B/7B/14B/32B sizes, which are
> Apache-2.0. For any commercial deployment, pin an Apache-2.0 size via `[ml] summariser_model`
> (1.5B is present; 7B is the quality option) and remove the 3B file. Local evaluation under the
> research licence is fine. This preserves the "prefer Chinese open models" choice — the whole Qwen
> family qualifies — while staying on a commercial-safe size.

## Image OCR ([[Image Pipeline]], M3-T04/T07)

| Model / data | Purpose | Source | Licence | Commercial use | Verified |
|:---|:---|:---|:---|:---|:---|
| **tesseract** engine | in-process OCR (default) | `leptess` → tesseract-ocr | Apache-2.0 | yes | ⚠️ standard, confirm |
| **tessdata** (`ara`, `fra`, `eng`) | OCR language data | tesseract-ocr/tessdata | Apache-2.0 | yes | ⚠️ confirm the tessdata repo |
| **baidu/Unlimited-OCR** (3B VLM) | optional high-quality OCR sidecar | Baidu (HuggingFace) | **MIT** | yes | ✅ 2026-08-21 |

## Image similarity ([[Vector Index]], M3-T05)

| Model | Purpose | Source | Licence | Commercial use | Verified |
|:---|:---|:---|:---|:---|:---|
| **openai/clip-vit-base-patch32** | 512-d image embeddings (CLIP embed sidecar) | OpenAI (HuggingFace) | MIT (OpenAI CLIP code is MIT) | yes | ⚠️ **the HF weights repo did not show a licence field on audit — confirm before production** |

## Voice / speech-to-text ([[Speech to Text]], M3-T02)

| Component | Purpose | Source | Licence | Commercial use | Verified |
|:---|:---|:---|:---|:---|:---|
| **faster-whisper** (library) | CTranslate2 whisper runtime | SYSTRAN | MIT | yes | ⚠️ standard, confirm |
| **Whisper `small`** (weights) | transcription | OpenAI, via faster-whisper conversion | MIT (OpenAI Whisper) | yes | ⚠️ confirm the specific converted-weights repo |

## Rust FFI dependencies (not models, noted for completeness)

| Crate | Wraps | Licence |
|:---|:---|:---|
| `leptess` / `tesseract-sys` / `leptonica-sys` | tesseract + leptonica | Apache-2.0 / MIT (leptonica: BSD-style) |
| `llama.cpp` (via `xustive-ml`) | GGUF inference | MIT |

## What still owes work here (T01.3 follow-ups)

- Replace every ⚠️ with the confirmed licence text captured at provisioning time (the `model-init`
  job — T01.2 — is the natural place to record the resolved licence per artefact).
- Resolve the **3B finding** before any commercial launch: swap the default to an Apache-2.0 size.
- Add each model's SHA-256 to a manifest (T01.2) so "which artefact was audited" is unambiguous.
