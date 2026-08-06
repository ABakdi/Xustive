---
tags:
  - component
  - serving
  - ml
component-id: C09
binary: xustive-ml
status: specified
updated: 2026-08-06
---

# Speech to Text

> **ID** C09 · **Binary** `xustive-ml` · **Upstream** [[API Gateway]] (`POST /search/voice`) · **Downstream** none (transcript returns to the client)

## 1. Purpose

Transcribe a short spoken query into text, entirely on our own servers. Voice matters
disproportionately here: a large share of Algerian users are more fluent speaking Darija than typing
it, and typing Arabic on a phone keyboard is slow.

## 2. Responsibilities

**In scope**: audio decode and resample; VAD trimming; Whisper inference; language identification
from audio; returning a transcript with confidence.

**Out of scope**: performing the search (the client does that with the transcript); wake words;
long-form audio; speaker identification (explicitly never).

## 3. Interface

```
POST /internal/transcribe   (xustive-api → xustive-ml)
multipart: audio bytes + { hint_lang?: "ar"|"ary"|"fr"|"en" }
→ { transcript, language, confidence, duration_ms, took_ms }
```

Public contract in [[API Contract]] §5.

## 4. Internal Design

### Pipeline

1. **Sniff** container/codec by magic bytes (`webm/opus`, `ogg/opus`, `wav`). Reject anything else
   with 415.
2. **Decode** to PCM via `symphonia`, inside `spawn_blocking` with a 5 s wall-clock cap and a decoded
   sample-count cap (60 s of audio) to stop decompression bombs
   ([[Security and Privacy]] §6).
3. **Resample** to 16 kHz mono f32.
4. **VAD trim** (`webrtc-vad` or energy-based): strip leading/trailing silence. A 12 s recording that
   is 3 s of speech should cost 3 s of inference. Reject with 422 `no_speech_detected` if speech
   < 300 ms.
5. **Normalise** loudness to −20 dBFS RMS — phone mics vary wildly.
6. **Inference**: `whisper.cpp` via Rust FFI, `ggml-small` quantised `q5_1`.
7. **Post-process**: strip Whisper's hallucinated trailing artefacts (`شكرا لكم`, "Thanks for
   watching", subtitle credits) via a known-artefact list — these appear on near-silent input and are
   a well-known failure mode. Apply the shared `xustive-text` normalisation.

### Model and language

| Setting | Value | Rationale |
|:---|:---|:---|
| Model | `small` multilingual, q5_1 (~500 MB) | `base` is noticeably worse on Arabic; `medium` is too slow on CPU |
| `language` | `ar` when detected/hinted, else auto | Darija is transcribed by the Arabic model; there is no `ary` Whisper language |
| `translate` | **off** | we want the original words, not English |
| `temperature` | 0.0, with fallback ladder 0.2/0.4 on decode failure | determinism |
| `no_context` | true | each request is independent |
| threads | 4 | |

**Expect degraded accuracy on Darija.** Whisper's Arabic training is MSA-heavy; code-switched
Darija/French transcribes imperfectly. Two mitigations: (a) the transcript is *editable* in the
search box before submitting ([[UI - Voice Search]]) — the user is the error correction; (b)
[[Query Expander]] absorbs some transcription variance. Do not promise verbatim accuracy in the UI.

### Concurrency

`n_slots = 2` per replica, bounded queue of 4. Voice shares `xustive-ml` with [[Summarizer]] and
[[Image Pipeline]]; a shared admission controller prevents voice bursts from starving summaries.

## 5. Configuration

| Key | Default |
|:---|:---|
| `model_path` | `/models/ggml-small-q5_1.bin` |
| `max_audio_bytes` | 10 MiB |
| `max_duration_s` | 30 |
| `min_speech_ms` | 300 |
| `sample_rate` | 16000 |
| `n_slots` | 2 |
| `queue_capacity` | 4 |
| `decode_timeout_ms` | 5000 |
| `inference_timeout_ms` | 8000 |
| `artefact_filter_path` | `data/stt/artefacts.txt` |

## 6. Data

**Stores nothing.** Audio is decoded from an in-memory buffer, the buffer is zeroised after
inference, and no file is ever written ([[Security and Privacy]] P4). A test asserts no new files
appear in the container's writable layer after a request.

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Unsupported codec | magic-byte sniff | 415 `unsupported_media_type` |
| File too large | gateway body limit | 413 |
| Decode timeout / panic | task boundary | 422 `image_unreadable`-analogue: `audio_unreadable` |
| Silence only | VAD | 422 `no_speech_detected` |
| Model missing at boot | checksum | **Fatal**, `readyz` red |
| Queue full | bounded channel | 503, UI says "try again" |
| Inference timeout | 8 s cap | 504 |
| Hallucinated artefact output | artefact filter | return empty transcript → 422 `no_speech_detected` |

## 8. Performance

| Metric | Budget |
|:---|:---|
| 5 s utterance, end-to-end | ≤ 1 500 ms p95 |
| Real-time factor (CPU, 4 threads) | ≤ 0.3× |
| Decode + resample | ≤ 150 ms |
| Resident memory | ≤ 1.5 GB per replica |

## 9. Observability

`xustive_stt_duration_seconds`, `xustive_stt_audio_duration_seconds`,
`xustive_stt_rtf` (real-time factor), `xustive_stt_rejected_total{reason}`,
`xustive_stt_language_total{lang}`. **The transcript is user query content — never logged**
([[Observability]] §1).

## 10. Security

Untrusted binary input: magic-byte typing, hard size and duration caps, memory-limited decode in an
isolated blocking task, panic caught at the task boundary. No disk writes, no egress. Microphone
permission is requested by the browser and is scoped by `Permissions-Policy: microphone=(self)`.

## 11. Testing

- Fixture corpus: 100 recordings (Darija 40, Arabic 25, French 25, English 10) from Algerian
  speakers, phone-quality, with reference transcripts.
- Metric: **WER** per language. Targets for v1: Arabic ≤ 25 %, French ≤ 20 %, Darija ≤ 45 %
  (honest baseline — improvement is a v2 goal, possibly via fine-tuning).
- Robustness: silence, pure noise, 30 s clipped, 1-sample file, truncated container — all must return
  a clean 4xx, never a panic.
- Artefact filter: near-silent input must not yield "شكرا لكم".
- Security: assert zero disk writes; assert oversized/bomb inputs are rejected within the timeout.

## 12. Open Questions

- [ ] Fine-tune Whisper on Algerian Darija? Needs a licensed, consented speech corpus — a project in
      itself, and a data-provenance question for [[Legal and Compliance]].
- [ ] Offer streaming transcription (partial results while speaking) — better UX, much more complex.
- [ ] Is `small` the right size, or does `medium` on GPU become viable if we add one?

## Related

[[UI - Voice Search]] · [[API Contract]] · [[Security and Privacy]] · [[Query Expander]] ·
[[Performance Budgets]] · [[Language Detector]]
