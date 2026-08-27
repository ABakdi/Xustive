---
tags:
  - ui
type: ui
status: specified
updated: 2026-08-06
---

# UI - Voice Search

> The microphone flow. Backend: [[Speech to Text]] · API: [[API Contract]] §5
> Parent: [[UI Specification]]

---

## 1. Why This Matters Here

Voice is not a novelty in this product. Typing Arabic on a phone keyboard is slow, and many users are
far more fluent speaking Darija than writing it in any script. For a meaningful share of the audience
this is the *primary* input, not the fallback.

Which also means: it has to be forgiving, because transcription of Darija will be imperfect
([[Speech to Text]] §4).

---

## 2. Flow

```
[🎤 tap]
   → permission (first time only)
   → recording UI (waveform + timer + stop)
   → auto-stop on 2s silence, or manual stop, or 30s cap
   → uploading (spinner)
   → transcript lands IN THE SEARCH BOX, editable, not submitted
   → user edits if needed → Enter → normal search
```

**The transcript is never auto-submitted.** It goes into the search box and waits. This single
decision absorbs most of the accuracy problem: the user reads it, fixes a word, and searches. Auto-
submitting a wrong transcription wastes a round trip and feels broken.

---

## 3. States

| State | UI |
|:---|:---|
| `idle` | mic icon in the search box |
| `requesting-permission` | icon dimmed; browser's own prompt is showing |
| `permission-denied` | inline message + how to re-enable, per browser; mic icon becomes muted and stays visible |
| `recording` | overlay: live waveform, elapsed timer, "Speak now", large Stop button |
| `processing` | "Transcribing…" with an indeterminate bar |
| `done` | overlay closes, transcript in the box, cursor at the end, box focused |
| `no-speech` | "We didn't hear anything — try again" + Retry |
| `error` | "Voice search unavailable right now" + Retry; mic still usable |
| `unsupported` | mic button not rendered at all |

Support detection: `navigator.mediaDevices?.getUserMedia` and `MediaRecorder` with an Opus mime type.
If either is missing, the button never appears — a button that fails on tap is worse than no button.

---

## 4. Recording UI

| Property | Spec |
|:---|:---|
| Presentation | bottom sheet on `sm`, centred dialog on `lg` (`<dialog>`, focus-trapped) |
| Waveform | 24 bars from `AnalyserNode` RMS, 60 fps, `--color-accent` |
| Reduced motion | waveform replaced by a static level meter that updates 4×/s |
| Timer | `0:03 / 0:30` |
| Auto-stop | 2 s of silence below the VAD threshold |
| Hard cap | 30 s, with the last 5 s counting down visibly |
| Cancel | `Esc`, backdrop tap, or the Cancel button — **discards audio, no upload** |
| Stop | uploads what was captured |

The waveform is not decoration: it is the only feedback that the microphone is actually picking up
sound. A user speaking into a muted mic needs to see flat bars, not a spinner.

---

## 5. Capture Settings

| Setting | Value |
|:---|:---|
| Codec | `audio/webm;codecs=opus`, fallback `audio/ogg;codecs=opus` |
| Sample rate | 48 kHz (server downsamples to 16 kHz) |
| Channels | mono |
| Bitrate | 24 kbps — plenty for speech, small on 3G |
| Constraints | `echoCancellation: true`, `noiseSuppression: true`, `autoGainControl: true` |
| Typical upload | ~15 KB for 5 s |

Upload uses `fetch` with an `AbortController` wired to the cancel action, so cancelling mid-upload
actually stops it.

---

## 6. Permissions

- Requested **only on tap**, never on page load. A page that asks for the microphone unprompted
  destroys trust in a product whose main claim is privacy.
- Denial is remembered by the browser; we detect it via `navigator.permissions.query({name:'microphone'})`
  where supported and show the muted state without re-prompting.
- The re-enable instructions are browser-specific (Chrome Android, Firefox, Safari iOS) because the
  generic "check your settings" is useless.
- `Permissions-Policy: microphone=(self)` is set at the edge ([[Security and Privacy]] §3).

---

## 7. Privacy Surface

Stated plainly in the recording overlay, in small text:

> Audio is processed on our servers in Algeria and is never stored.

That is a claim [[Speech to Text]] §6 must keep true — audio is decoded from an in-memory buffer,
zeroised after inference, and never written to disk, with a test asserting it.

The recording indicator is always visible while capturing. The stream's tracks are stopped
(`track.stop()`) immediately on stop or cancel so the browser's own mic indicator clears — leaving a
live track open after recording looks, correctly, like eavesdropping.

---

## 8. Error Handling

| Error | Message | Recovery |
|:---|:---|:---|
| `NotAllowedError` | "Microphone access is blocked" + per-browser instructions | user action |
| `NotFoundError` | "No microphone found" | none; hide the flow for the session |
| `NotReadableError` | "Your microphone is in use by another app" | Retry |
| 413 payload too large | "Recording too long" | Retry (should be impossible — the 30 s cap prevents it) |
| 415 unsupported | "This browser's audio format isn't supported" | hide the button for the session |
| 422 `no_speech_detected` | "We didn't hear anything" | Retry |
| 503 / queue full | "Voice search is busy — try again" | Retry |
| Network failure | "Connection lost" | Retry with the audio still buffered client-side |

The last row matters on 3G: keep the recorded blob in memory until the upload succeeds so a retry
does not require re-recording.

---

## 9. Accessibility

- The mic button has `aria-label="Search by voice"` and `aria-pressed` while recording.
- The overlay is a modal `<dialog>`: focus trapped, `Esc` closes, focus returns to the mic button.
- State changes announce via `aria-live="assertive"` ("Recording", "Transcribing", "Transcript
  ready") — assertive rather than polite because the user is mid-interaction and timing matters.
- Every stage is keyboard-operable; the waveform is `aria-hidden`.
- The 30 s limit is announced at 25 s, not only shown.
- **Voice search is not the only way to do anything** — it is an alternative input, so no
  functionality is lost if a user cannot speak or the mic is unavailable ([[UI - Accessibility]]).

---

## 10. Open Questions

- [ ] Should we show a confidence indicator on the transcript (e.g. dim low-confidence words) so the
      user knows what to check? Useful, but risks looking broken.
- [ ] Hold-to-talk as an alternative to tap-to-start/tap-to-stop? Common on messaging apps here, so
      it may be the more familiar gesture.
- [ ] Do we offer a language hint selector before recording, or always auto-detect?

## Related

[[Speech to Text]] · [[API Contract]] · [[UI - Home Page]] · [[UI - States and Errors]] ·
[[UI - Accessibility]] · [[Security and Privacy]]

## Revision, 2026-08-27 — inline and live

The dialog is gone. The field itself is the recorder: the microphone button turns red and
breathes, a four-bar level meter beside it shows the microphone is hearing you, the placeholder
says "Listening…", and **the words appear in the box while you speak** — every 1.5 s the audio so
far is sent again to our own transcriber and the box shows the newest reading, dimmed; on stop
(the button again, or Esc to cancel) one last pass gives the final words, undimmed and editable.
Still never submitted for you.

Failure is visible now. The first version announced "unavailable" in a screen-reader-only span,
so a page with no transcriber looked like a mute button. A 503/404 on the first partial stops the
recording at once and puts the message in red under the field.

Verified with a synthetic microphone (an oscillator handed to `getUserMedia`) against a stub
sidecar: 2 words at 5 s, 4 at 7 s, final on stop; and the red message with the sidecar down.
Not yet verified against Whisper itself — the sidecar needs `faster-whisper` and the `small`
weights (~480 MB), which are not on this machine.

## Revision, 2026-08-27 (later) — live for real

Live partials on CPU took ~1.2 s each: Whisper encodes a fixed thirty-second window, so a
partial costs a whole encoder pass whatever the clip's length, and no decode setting changes
that. Two things made it live. The sidecar now loads two models — `base` for partials
(`?partial=1`: greedy, no timestamps, no context) and `small` for the final pass — and runs on
the GPU when CTranslate2 sees one. On the Quadro T1000, measured on a 10 s Arabic clip through
the API: a partial in 0.35–0.5 s, the final in 1.0–1.5 s. A finding worth keeping: on that card
`float16` is *slower* than `float32` (435 ms vs 128 ms for the `base` encoder), so float32 is
the launcher's default. The box asks every 400 ms and sends the audio so far in 200 ms slices.
