---
tags:
  - component
  - serving
  - ml
component-id: C09
binary: stt-sidecar
status: built
updated: 2026-08-27
---

# Speech to Text

> **ID** C09 · **Runs in** `services/stt-sidecar` (Python, faster-whisper) behind
> `crates/xustive-api/src/stt.rs` · **Upstream** [[API Gateway]] (`POST /api/v1/transcribe`) ·
> **Downstream** none (the transcript returns to the client)

## 1. Purpose

Transcribe a short spoken query into text, entirely on our own servers. Voice matters
disproportionately here: a large share of Algerian users are more fluent speaking Darija than typing
it, and typing Arabic on a phone keyboard is slow.

## 2. Responsibilities

**In scope**: Whisper inference; language identification from audio; silence trimming; dropping
hallucinated segments; live partial readings while the person is still speaking.

**Out of scope**: performing the search (the client does that with the transcript); wake words;
long-form audio; speaker identification (explicitly never).

## 3. Where it lives today

| Piece | Path |
|:---|:---|
| Sidecar (FastAPI + faster-whisper / CTranslate2) | `services/stt-sidecar/app.py`, `run.sh`, `README.md` |
| Rust client with circuit breaker, artefact filter | `crates/xustive-api/src/stt.rs` |
| Config | `[stt]` in `config/*.toml` (`SttConfig`) |
| Button, recording, partials | `web/components/search/VoiceButton.tsx` |

The Whisper-in-Rust design from the first draft (`whisper.cpp` FFI inside `xustive-ml`) was not
built; the sidecar follows the OCR and CLIP pattern instead ([[ADR-0016 - Two OCR Engines with an Optional Unlimited-OCR Sidecar]]) — a tiny HTTP contract in front of a model kept out of the Rust
build. **Voice is not GPU-gated**: Whisper `small` at int8 runs on CPU in a few seconds, and uses
a GPU when one is there.

## 4. Interface

```
POST /api/v1/transcribe?lang=ar[&partial=1]     body = raw audio bytes
  → { "text": "…", "language": "ar" }           text may be empty for silence — not an error
  503 model_unavailable  stt_unavailable          disabled, breaker open, or sidecar failed
  400 empty_audio | audio_too_large

sidecar:  POST /transcribe?lang=&partial=    →  {"text","language"}     GET /health → 200 | 503
```

`lang` is the UI language, whitelisted to `ar | ary | fr | en`, so a short Arabic clip is not
mis-detected and a stray query value cannot reach the model. `partial=1` asks for a reading of the
words so far — fast model, greedy decode, no timestamps — which the box shows while the person is
still talking; the final pass, without the flag, gets the careful model and a beam.

## 5. Internal Design

### 5.1 The browser (`VoiceButton.tsx`)

Renders only where `getUserMedia` and `MediaRecorder` exist; asks for the microphone on tap, never
on load; records Opus in WebM (or Ogg / MP4 where that is what the browser has) at 24 kbit/s.
Every `PARTIAL_EVERY_MS = 400` ms it posts the bytes so far with `partial=1` and shows the newest
reading **inline in the search box**, so the text grows while you talk. A failed partial is not
worth interrupting for — the next one may land. Recording is capped at `MAX_MS = 30 s`.

On **stop, the search is submitted** with the words that were on screen the whole time; editing
is one tap away on the results page. (The earlier rule was never to submit for the person —
auto-submit on stop was asked for after the live version shipped.) Cancel discards. Every track is
stopped either way. When the server has no transcriber at all the button stops early and says so
rather than recording thirty seconds for nothing.

### 5.2 The client (`stt.rs`)

`SttClient` exists only when `[stt] enabled`. A **circuit breaker** (`xustive_core::circuit`,
3 failures, 5 s cooldown backing off to 60 s) makes a down sidecar fail fast with
`stt_unavailable` instead of every request waiting out the timeout; the admin console shows its
state and probes `/health`. The audio is forwarded as an `application/octet-stream` body and
never stored or logged.

**Artefact filter** (M3-T02.6, defence in depth): a transcript that is *nothing but* a known
Whisper silence hallucination — "thank you", "thanks for watching", "merci d'avoir regardé",
"شكرا لكم", "اشتركوا في القناة", "ترجمة" and the rest of `ARTEFACTS` — is blanked, so a voice
search never runs for a word nobody said. Only whole-transcript matches are removed; an utterance
that merely contains "thank you" is untouched.

### 5.3 The sidecar (`app.py`)

1. **Two models.** `STT_MODEL` (`small`) for the final pass; `STT_PARTIAL_MODEL` (`base`) for
   partials. Whisper encodes a fixed thirty-second window whatever the clip's length, so a partial
   costs a whole encoder pass; `base` does that in a fraction of `small`'s time. Unset, the main
   model answers partials too — correct, just not live.
2. **Decode** straight from the in-memory bytes via PyAV — no temp file, nothing persists.
3. **VAD filter** on, so a near-silent clip returns little.
4. **Final**: `beam_size 5`, `best_of 5`, conditioned on previous text. **Partial**: greedy,
   `without_timestamps`, unconditioned — it is replaced 400 ms later, and what it gets wrong the
   final pass gets right.
5. **Segment filter**: a segment with `no_speech_prob > 0.6` *and* `avg_logprob < −1.0` was the
   model guessing on silence and is dropped.
6. **OOM fallback**: the GPU is shared with the API's own models and the desktop, and the careful
   model's beam search is the first thing to run out of room. A CUDA out-of-memory on a final pass
   is answered by the partial model — a worse answer than the careful one and a far better one
   than a 500.

### 5.4 Device and precision (`run.sh`)

| Where | Final (`small`) | Partial (`base`) |
|:---|:---|:---|
| GPU (CTranslate2 sees CUDA) | `int8_float16` — as fast as float32 at beam 5 on the T1000 (1.2 s vs 1.4 s) and a third of the memory on a shared 4 GB card | `float32` — on the T1000 float16 is *slower* for this encoder (435 ms vs ~130 ms) |
| CPU | `int8` | `int8` |

`STT_DEVICE`, `STT_COMPUTE`, `STT_PARTIAL_COMPUTE`, `STT_CPU_THREADS` override. The CUDA 12
runtime comes from the pip wheels in the venv (the host's CUDA 13 does not match CTranslate2's
build), which is what `run.sh`'s `LD_LIBRARY_PATH` line is for.

### 5.5 Language

`ar` when hinted, else Whisper's own detection (`STT_DEFAULT_LANG` can bias it). Darija is
transcribed by the Arabic model; there is no `ary` Whisper language. **Expect degraded accuracy
on Darija.** Whisper's Arabic training is MSA-heavy; code-switched Darija/French transcribes
imperfectly. The mitigations are that the transcript is editable ([[UI - Voice Search]]) and that
[[Query Expander]] absorbs some variance. Do not promise verbatim accuracy in the UI.

## 6. Configuration

| Key | Default | Meaning |
|:---|:---|:---|
| `stt.enabled` | `false` (dev: `true`) | no client, no route behaviour beyond `stt_unavailable` |
| `stt.endpoint` | `http://127.0.0.1:8093/transcribe` | internal-network call, not egress |
| `stt.timeout_ms` | 30 000 | per request |
| `stt.max_audio_bytes` | 8 MiB | a 30 s Opus clip is well under 1 MB |
| `STT_MODEL` / `STT_PARTIAL_MODEL` | `data/models/faster-whisper-small` / `-base` | a Whisper size name or a directory; the directory is preferred because the hub download ran at 60 KB/s on 2026-08-27 |
| `STT_MAX_BYTES` | 16 MiB | sidecar's own ceiling |
| `STT_HOST` / `STT_PORT` | `127.0.0.1` / `8093` | |

Dev has it **on** so the voice button answers honestly — "unavailable" while the sidecar is down,
words once it is up — instead of a 404 that reads as a broken button.

## 7. Data

**Stores nothing.** Audio is decoded from an in-memory buffer; no file is written; the sidecar
logs only that a request succeeded, its byte and character counts, language and latency — never
the audio or the transcript ([[Security and Privacy]] P4). Buffer zeroising and the "no new files
in the writable layer" test from the first draft are not built (2026-08-27).

## 8. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Voice disabled | config | `stt_unavailable`; button stops early and says so |
| Sidecar down | breaker | `stt_unavailable` immediately; probes recover it |
| Empty body / too large | size checks | 400 `empty_audio` / `audio_too_large` |
| Bad container | PyAV | sidecar 500 → `stt_unavailable`; traceback to the sidecar log only |
| Silence only | VAD + segment filter + artefact list | empty `text`; the client shows an empty state |
| CUDA OOM on the final pass | `RuntimeError` | answered by the partial model |
| Model not loaded | `/health` 503 | breaker opens |
| Inference over `timeout_ms` | client timeout | `stt_unavailable`, breaker counts a failure |

Text search never depends on any of this.

## 9. Performance

Measured on a Quadro T1000 with a 10 s Arabic clip: a partial in ~0.4 s, the final in ~1 s. On
CPU the same are ~1.2 s and ~3.5 s, because of the fixed thirty-second encoder window. The
original budgets (5 s utterance ≤ 1.5 s p95, RTF ≤ 0.3×) are met on GPU and not on CPU.

## 10. Observability

Latency and outcome go to the sidecar's log; the breaker state is on the admin console. The
`xustive_stt_*` metrics from the first draft are not built (2026-08-27). **The transcript is user
query content — never logged** ([[Observability]] §1).

## 11. Security

Untrusted binary input: hard size caps on both sides, decode inside the sidecar process where a
crash is a 500 and not an API outage, whitelisted language hint, no disk writes, no egress
(the sidecar is an internal-network service). Microphone permission is requested by the browser
on tap and scoped by `Permissions-Policy: microphone=(self)`.

## 12. Testing

- `stt.rs`: disabled config builds no client; enabled builds one; artefact phrases are blanked
  only as whole transcripts.
- Not built (2026-08-27): the 100-recording Darija/Arabic/French fixture corpus and per-language
  WER targets; the robustness suite (noise, clipped, 1-sample, truncated container). Improvement
  on Darija remains a v2 goal, possibly via fine-tuning.

## 13. Open Questions

- [ ] Fine-tune Whisper on Algerian Darija? Needs a licensed, consented speech corpus — a project
      in itself, and a data-provenance question for [[Legal and Compliance]].
- [x] Streaming transcription — built as 400 ms partials with a second, lighter model.
- [ ] Is `small` the right final model, or does `medium` on GPU become viable now the card is
      shared three ways (API models, STT, desktop)?
- [ ] Should auto-submit on stop stay, given imperfect Darija transcription? The words are on
      screen throughout, which is the argument for it.

## Related

[[UI - Voice Search]] · [[API Contract]] · [[Security and Privacy]] · [[Query Expander]] ·
[[Performance Budgets]] · [[Language Detector]] · [[Media Extraction]] ·
[[ADR-0016 - Two OCR Engines with an Optional Unlimited-OCR Sidecar]] ·
[[Milestone 3 - Multimodal Input]]
