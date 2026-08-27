---
tags: [adr]
adr-id: "0024"
status: accepted
date: 2026-08-27
---
# ADR-0024 - Two-Model Voice Transcription that Submits on Stop

## Status

Accepted, implemented. Constrains [[Speech to Text]], [[UI - Voice Search]] and the `[stt]`
section of the API config. Extends [[ADR-0016 - Two OCR Engines with an Optional Unlimited-OCR Sidecar]]'s
sidecar pattern (a private-network Python service the serving plane calls like it calls Redis) to
audio. Operates inside [[ADR-0008 - No Query Logging]]: audio stays in memory on both sides and no
transcript is written anywhere.

Records two decisions made in code on 2026-08-27, one of which **reverses** an earlier UI rule.

## Context

Voice search wraps Whisper (`faster-whisper` on CTranslate2) in `services/stt-sidecar`. The first
version transcribed once, after the person stopped. On CPU a clip took a second or two, which
reads as dead air; and the transcript landed in the box unsubmitted, because the earlier rule —
written for OCR and copied to voice — was that a guess must never become a search on its own.

Making the words appear *while* the person speaks changed the economics. Whisper encodes a fixed
thirty-second window regardless of clip length, so every live partial costs a full encoder pass;
`small` on CPU is ~1.2 s per pass, which is not live. On the reference GPU (Quadro T1000, 4 GB,
shared with the API's own models and the desktop) two more facts were measured rather than
assumed: `float16` is *slower* than `float32` for the `base` encoder on that card (435 ms vs
128 ms), and the careful model's beam search at `float32` is what runs the card out of memory.

## Decision

1. **Two models, two jobs.** `base` answers live partials (`?partial=1`: greedy, no timestamps, no
   context), `small` answers the final pass with a beam of 5. `STT_PARTIAL_MODEL` unset means the
   main model answers partials too — correct, just not live.
2. **Compute type is chosen per model from measurement, not from the usual GPU defaults.** On CUDA
   the final model runs `int8_float16` (a third of the memory, no slower at beam 5); the partial
   model stays `float32`, where its encoder is fastest on this card. On CPU both run `int8`.
   `run.sh` picks the device by asking CTranslate2 whether it sees a GPU; `STT_DEVICE` /
   `STT_COMPUTE` / `STT_PARTIAL_COMPUTE` override.
3. **A final pass that hits CUDA out-of-memory is answered in-process by the partial model**, not
   with a 500 — a worse reading is a far better answer than an error under a box full of text.
4. **Stop means search.** The box shows the words as they are spoken (dimmed), so confirming them
   again is a second tap for nothing; editing is one tap away on the results page. This overrides
   the earlier "never auto-submit" rule, which was written for a box that showed nothing until the
   end. If the final pass fails, the box searches with the last live reading. Esc still cancels
   without searching. OCR keeps its never-auto-submit rule ([[ADR-0016 - Two OCR Engines with an Optional Unlimited-OCR Sidecar]]):
   a photographed document is not a spoken query.
5. **Voice is not GPU-gated.** The sidecar is behind the `voice` compose profile and `[stt]
   enabled` (default off); when on without a GPU it works on CPU int8, just not live.

## Consequences

**Good**
- Live words on the reference card: a partial in 0.35–0.5 s, the final in 1.0–1.5 s for a 10 s
  Arabic clip; both models together 756 MB of a shared 4 GB.
- One tap fewer per voice search, and the words are visible before the search happens, which is
  the review step the old rule was protecting.

**Bad**
- Two models to distribute (~486 MB + the `base` weights), fetched by hand because the hub client
  was measured at 60 KB/s unauthenticated (`services/stt-sidecar/README.md`).
- The compute-type choice is specific to one card. A card with real FP16 throughput wants
  `STT_COMPUTE=float16`; the launcher's defaults will be wrong there and must be overridden.
- Auto-submit on stop means a mis-heard word runs a search before it is corrected. Accepted
  because the reading was on screen the whole time and the results page has the same box.

## Alternatives

| Option | Why not |
|:---|:---|
| One model for both partials and final | `small` per partial is ~1.2 s on CPU and the words never feel live; `base` for the final is a worse transcript than the card can afford |
| `float16` everywhere on GPU | measured slower than `float32` for the `base` encoder on the T1000 |
| Keep never-auto-submit | written for a box that showed nothing until the end; with live words in the box it is a redundant tap |
| Browser `SpeechRecognition` API | sends audio to the browser vendor; the sidecar exists so audio never leaves the box |

## Revisit when

- A card with working FP16 throughput becomes the reference — re-measure and flip the defaults.
- Whisper-class models gain a streaming encoder, at which point the two-model split is
  unnecessary cost.
- Mis-heard auto-submits show up as a complaint pattern — the fallback is a confirm step only
  when the final differs materially from the last partial, not a return to never-submit.

## Where it stands (2026-08-27)

Built as described: `services/stt-sidecar/app.py` (`PARTIAL_MODEL_NAME`, `?partial=1`, OOM
fallback in `transcribe`), `services/stt-sidecar/run.sh` (device/compute selection), the Rust
client `crates/xustive-api/src/stt.rs` behind `POST /api/v1/transcribe`, and the recorder in
`web/components/search/VoiceButton.tsx` (commits `8b17711`, `274089f`). The narrative and
measurements are in [[UI - Voice Search]] §Revisions 2026-08-27.

## Related

[[Speech to Text]] · [[UI - Voice Search]] · [[ADR-0016 - Two OCR Engines with an Optional Unlimited-OCR Sidecar]] ·
[[ADR-0008 - No Query Logging]] · [[Milestone 3 - Multimodal Input]] · [[Decision Log]]
