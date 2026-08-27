---
tags:
  - ui
type: ui
status: built
updated: 2026-08-27
---

# UI - Voice Search

> The microphone flow. Backend: [[Speech to Text]] · API: [[API Contract]] §5
> Parent: [[UI Specification]] · Code: `web/components/search/VoiceButton.tsx`, the voice bits of
> `web/components/search/SearchBox.tsx`, `transcribe()` in `web/lib/api.ts`, `.voice-*` in
> `web/app/globals.css`

---

## 0. Current behaviour (2026-08-27)

The sections below carry the original 2026-08-06 design and three dated revisions from
2026-08-27. Read together they contradict each other in places (dialog vs inline, "never
auto-submit" vs "stop means search"), so this is what the code does today — the revisions at the
end are the history of how it got here.

- **Inline, no dialog.** `VoiceButton` sits inside the search field (`SearchBox`) and reports its
  state upward through `onState`; the field draws everything around it. There is no `<dialog>`,
  no waveform overlay, no timer readout beyond a seconds counter.
- **Rendered only where it can work.** The button returns `null` unless
  `navigator.mediaDevices.getUserMedia` and `window.MediaRecorder` exist. Microphone permission is
  asked on tap, never on load. `Permissions-Policy: microphone=(self)` is set in
  `web/next.config.ts`.
- **Recording.** The button gets `.voice-button.is-recording` (red, `voice-breathe` animation;
  `animation: none` under `prefers-reduced-motion: reduce`) and shows a Square icon;
  `aria-label`/`title` switch from `t.voiceSearch` to `t.voiceStop`, `aria-pressed` is true.
  Beside it a four-bar `.voice-meter` (`aria-hidden`) lights bars from an `AnalyserNode` RMS on
  every animation frame, and the elapsed seconds tick every 250 ms in an `aria-live="polite"`
  span. The input placeholder becomes `t.voiceListening`.
- **Live partials.** `MediaRecorder.start(200)` delivers 200 ms slices; whenever a slice lands, at
  least `PARTIAL_EVERY_MS = 400` ms have passed and no request is in flight, the whole audio so
  far is POSTed to `/api/v1/transcribe?lang=<ui>&partial=1`. The reading replaces the box text
  (`onInterim`), drawn in `--fg-muted` to show it is provisional; suggestions close.
- **Stop = search.** Tapping the button again (or the 30 s cap, `MAX_MS`) stops the recorder;
  the phase becomes `finishing` (button disabled, meter text `t.voiceTranscribing`) and one final
  non-partial pass runs. The final text is put in the box undimmed and **submitted** via
  `router.push('/<lang>/search?q=…')`. If the final pass fails but a live reading exists, the
  search runs with that reading instead.
- **Cancel.** `Esc` while recording discards the audio: recorder stopped, tracks stopped, nothing
  uploaded, nothing searched. There is no Cancel button.
- **Errors are visible.** Under the field, `<p role="status">` in `--danger` red with `dir="auto"`:
  `t.voicePermission` (getUserMedia threw — denied, no device, insecure context),
  `t.voiceUnavailable` (503/404 from the transcriber — a 503/404 on a *partial* stops the
  recording immediately), `t.voiceFailed` (anything else, or an empty recording).
- **Release.** On stop, cancel or unmount every track is `stop()`ped, the `AudioContext` closed
  and any in-flight request aborted, so the browser's own mic indicator clears.
- **Codec.** First supported of `audio/webm;codecs=opus`, `audio/webm`, `audio/ogg;codecs=opus`,
  `audio/mp4`, at 24 kbps; browser default otherwise.
- **i18n keys used:** `voiceSearch`, `voiceStop`, `voiceListening`, `voiceTranscribing`,
  `voiceUnavailable`, `voiceFailed`, `voicePermission`. `voiceCancel` and `voiceHint` exist in
  `messages.ts` but nothing renders them today.

Everything from §2 to §9 is the original specification; where it differs from the list above,
the list wins.

---

## 1. Why This Matters Here

Voice is not a novelty in this product. Typing Arabic on a phone keyboard is slow, and many users are
far more fluent speaking Darija than writing it in any script. For a meaningful share of the audience
this is the *primary* input, not the fallback.

Which also means: it has to be forgiving, because transcription of Darija will be imperfect
([[Speech to Text]] §4).

---

## 2. Flow (original, 2026-08-06 — superseded 2026-08-27, see §0)

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

> Superseded 2026-08-27: the words are now shown live while speaking, so stop submits (§0 and the
> third revision below). There is no silence auto-stop; only the button and the 30 s cap end a
> recording.

---

## 3. States

What ships is three phases in `VoiceButton` (`idle` → `recording` → `finishing` → `idle`) plus an
error string; the field renders them. The original table, annotated:

| State | Original spec | Today |
|:---|:---|:---|
| `idle` | mic icon in the search box | same (`Mic` icon, `--fg-faint`) |
| `requesting-permission` | icon dimmed; browser's own prompt | no distinct state — still `idle` until the stream arrives |
| `permission-denied` | inline message + per-browser re-enable instructions | `t.voicePermission` under the field; no per-browser text |
| `recording` | overlay: waveform, timer, "Speak now", Stop | inline: red breathing button, 4-bar meter, seconds, "Listening…" placeholder, live words in the box |
| `processing` | "Transcribing…" + indeterminate bar | `finishing`: button disabled, meter span reads `t.voiceTranscribing` |
| `done` | transcript in the box, focused, not submitted | final words in the box **and the search runs** |
| `no-speech` | "We didn't hear anything" + Retry | an empty blob → `t.voiceFailed`; the server's own no-speech answer is not distinguished |
| `error` | "unavailable" + Retry | `t.voiceUnavailable` (503/404) or `t.voiceFailed`; the mic stays usable |
| `unsupported` | button not rendered | same |

Support detection: `navigator.mediaDevices?.getUserMedia` and `'MediaRecorder' in window`. If
either is missing, the button never appears — a button that fails on tap is worse than no button.
(The original also required an Opus mime type; today Opus is preferred but any recorder mime is
accepted, see §5.)

---

## 4. Recording UI (original — superseded 2026-08-27)

| Property | Original spec | Today |
|:---|:---|:---|
| Presentation | bottom sheet on `sm`, centred `<dialog>` on `lg` | inline in the field, no dialog |
| Waveform | 24 bars, 60 fps, `--color-accent` | 4 bars (`.voice-meter`), rAF-driven, red |
| Reduced motion | static meter at 4×/s | the meter is already static bars; only the button's breathing animation is removed |
| Timer | `0:03 / 0:30` | `3s` |
| Auto-stop | 2 s of silence | none |
| Hard cap | 30 s with a visible countdown | 30 s, no countdown |
| Cancel | `Esc`, backdrop, Cancel button — discards | `Esc` only — discards |
| Stop | uploads what was captured | final pass, then searches |

The meter is not decoration: it is the only feedback that the microphone is actually picking up
sound. A user speaking into a muted mic needs to see dark bars, not a spinner — and with live
partials, the words themselves are the second signal.

---

## 5. Capture Settings

| Setting | Value |
|:---|:---|
| Codec | first supported of `audio/webm;codecs=opus`, `audio/webm`, `audio/ogg;codecs=opus`, `audio/mp4`; else the browser default |
| Bitrate | 24 kbps (`audioBitsPerSecond`) — plenty for speech, small on 3G |
| Timeslice | 200 ms (`rec.start(200)`), so audio arrives as it is spoken |
| Constraints | `{ audio: true }` — the original `echoCancellation`/`noiseSuppression`/`autoGainControl` flags are not passed explicitly (browser defaults apply) |
| Upload | raw POST body, `Content-Type` = the blob's mime type, `cache: 'no-store'` |

Upload uses `fetch` with an `AbortController`: a new partial aborts the previous one, and cancel
aborts whatever is in flight, so cancelling mid-upload actually stops it.

---

## 6. Permissions

- Requested **only on tap**, never on page load. A page that asks for the microphone unprompted
  destroys trust in a product whose main claim is privacy.
- The original spec detected a remembered denial via `navigator.permissions.query` and showed a
  muted state; today a denied tap simply throws and shows `t.voicePermission`. Not built.
- Per-browser re-enable instructions: not built; one sentence for all browsers.
- `Permissions-Policy: geolocation=(), microphone=(self), camera=(self)` is set by
  `web/next.config.ts` on every route ([[Security and Privacy]] §3).

---

## 7. Privacy Surface

The original spec put a line in the recording overlay:

> Audio is processed on our servers in Algeria and is never stored.

There is no overlay now and the field shows no such line — the only privacy copy near the box is
the home page's `privacyLine`. The claim itself still holds and [[Speech to Text]] §6 must keep it
true — audio is decoded from an in-memory buffer, never written to disk.

The recording indicator (red button + meter) is always visible while capturing. The stream's tracks
are stopped (`track.stop()`) immediately on stop or cancel so the browser's own mic indicator clears
— leaving a live track open after recording looks, correctly, like eavesdropping.

---

## 8. Error Handling

What the code does today; the original per-status table is folded in.

| Error | Message (key) | Behaviour |
|:---|:---|:---|
| `getUserMedia` rejects (`NotAllowedError`, `NotFoundError`, `NotReadableError`, insecure context) | `voicePermission` | tracks released, back to idle; mic stays tappable |
| 503 / 404 on a **partial** | `voiceUnavailable` | recording stopped at once, no final pass — the server has no transcriber, no point recording thirty seconds into nothing |
| any other partial failure | — | ignored; the next partial may land |
| final pass fails, a live reading exists | — | search runs with the last live reading |
| final pass fails, nothing read yet | `voiceUnavailable` (503/404) / `voiceFailed` | message under the field |
| recording produced zero bytes | `voiceFailed` | message under the field |

The original spec's 413/415/422 wording and "Retry with the audio still buffered" are not built:
the audio is dropped once the recorder is released, so a retry re-records.

---

## 9. Accessibility

- The mic button has `aria-label` = `t.voiceSearch` (`t.voiceStop` while recording), a matching
  `title`, and `aria-pressed` while recording.
- No modal, so no focus trap; `Esc` cancels while recording.
- The meter/seconds span is `aria-live="polite"` (the original said assertive); the error line
  under the field is `role="status"`.
- The meter bars are `aria-hidden`; the button icons are `aria-hidden`.
- The 30 s limit is not announced ahead of time (original spec: at 25 s) — not built.
- **Voice search is not the only way to do anything** — it is an alternative input, so no
  functionality is lost if a user cannot speak or the mic is unavailable ([[UI - Accessibility]]).

---

## 10. Open Questions

- [ ] Should we show a confidence indicator on the transcript (e.g. dim low-confidence words) so the
      user knows what to check? Useful, but risks looking broken.
- [ ] Hold-to-talk as an alternative to tap-to-start/tap-to-stop? Common on messaging apps here, so
      it may be the more familiar gesture.
- [ ] Do we offer a language hint selector before recording, or always auto-detect? (Today the UI
      language is sent as `lang=` on every request.)
- [ ] Now that stop submits, is a visible Cancel control needed for touch users who cannot press
      `Esc`?

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

## Revision, 2026-08-27 (later still) — stop means search

Two changes asked for after using it. **Stop searches.** The words were in the box the whole
time they were spoken, so confirming them again is a second tap for nothing; editing is one tap
away on the results page. This overrides the earlier rule never to submit for the person — that
rule was written for a box that showed nothing until the end.

**"Unavailable" after a good recording.** The GPU is shared — the API's own models held 1.6 GB,
the sidecar 1.5 GB at float32, the desktop the rest of 4 GB — and the careful model's beam search
was the first thing to run out of room: CUDA OOM on the final pass, three of those, and the
breaker turned every request into an instant 503. Three fixes: the final model is quantised
(`int8_float16`, 756 MB for both models now, and no slower at beam 5); a final that hits OOM is
answered by the light model in-process rather than with a 500; and the box, if the final pass
fails anyway, searches with the last live reading instead of showing an error under text.
