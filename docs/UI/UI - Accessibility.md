---
tags:
  - ui
  - a11y
type: ui
status: specified
updated: 2026-08-06
---

# UI - Accessibility

> Target: **WCAG 2.2 Level AA**. Not a checklist appended at the end — the commitments below are
> constraints on the component specs in [[UI - Component Library]].
> Parent: [[UI Specification]]

---

## 1. Commitments

| # | Commitment | Verified by |
|:---|:---|:---|
| A1 | Every function is keyboard-operable | manual keyboard pass per screen |
| A2 | Visible focus on every interactive element | `:focus-visible` token + axe scan |
| A3 | Text contrast ≥ 4.5:1 (≥ 3:1 for large) | automated contrast test on token pairs |
| A4 | No information conveyed by colour alone | design review + greyscale screenshots |
| A5 | Touch targets ≥ 44 × 44 px | automated bounding-box test |
| A6 | Every image has a meaningful `alt` or `aria-hidden` | lint + axe |
| A7 | Correct heading hierarchy, no skipped levels | axe |
| A8 | Live regions announce changes without flooding | manual screen reader pass |
| A9 | Works at 200 % zoom and 320 px width with no horizontal scroll | responsive test |
| A10 | `prefers-reduced-motion` honoured everywhere | CSS audit |
| A11 | Works with JS disabled for core search | [[UI Specification]] §8 |

---

## 2. Keyboard

| Key | Behaviour |
|:---|:---|
| `Tab` / `Shift+Tab` | logical order: skip link → search → filters → results → pagination |
| `/` | focus the search box from anywhere |
| `↓` `↑` | move through suggestions |
| `Enter` | submit / activate |
| `Esc` | close suggestions, sheet, or dialog; restore focus to the trigger |
| `Space` | toggle a filter chip |

Rules:
- **No keyboard traps.** Modals (`<dialog>`) trap focus deliberately and release it on close.
- Focus returns to the element that opened any overlay.
- Focus survives re-renders: toggling a filter re-renders results but the toggled chip keeps focus
  ([[UI - Filters and Facets]] §4).
- Skip links: "Skip to results", "Skip to filters" — visible on focus.

---

## 3. Screen Readers

Tested with **NVDA + Firefox**, **VoiceOver + Safari (iOS)**, and **TalkBack + Chrome (Android)**.
TalkBack matters most: it is the realistic assistive technology for this audience.

| Element | Accessible name |
|:---|:---|
| Search input | "Search" (`aria-label`) |
| Mic button | "Search by voice" |
| Camera button | "Search by image" |
| Filter chip | "Facebook, 700 results" (includes the count) |
| Result title | the title text alone; the card provides context |
| Sentiment badge | "Sentiment: negative" |
| Platform badge | "Source: Facebook" |
| Pagination | "Page 2 of 92", current has `aria-current="page"` |
| Similarity tile | "Result 3 of 12, very similar, from Instagram, 4 August 2026" |

### Live regions — the part that usually goes wrong

| Region | Politeness | Announces |
|:---|:---|:---|
| Result count | polite | once per search: "About 1,800 results" |
| Summary | polite | **once on completion**, never per token |
| Suggestions | polite | "8 suggestions available" |
| Voice state | assertive | "Recording", "Transcribing", "Transcript ready" |
| Errors | assertive | the error message |
| Upload progress | polite | 0 %, 50 %, 100 % only |

Announcing the streaming summary token-by-token would make the page unusable with a screen reader.
It streams visually and announces once — this is specified in [[UI - Results Page]] §8 for exactly
this reason.

---

## 4. Colour and Contrast

- Every semantic token pair in [[UI - Design Language]] §2 is contrast-tested in CI; the build fails on
  a regression. The tightest pair (`--color-text-muted` on `--color-surface`, 4.6:1) is the canary.
- **Sentiment is never colour alone**: ▲/●/▼ icon + text label + colour (A4).
- Platform badges carry text, not just a logo colour.
- Search-term highlighting uses a background *and* italic emphasis, so it survives greyscale.
- Link text is distinguishable from body text without relying on colour (weight + underline on
  hover/focus).
- Dark mode is contrast-tested independently — it is a separate set of pairs, not a filter.

---

## 5. Motion and Timing

- `prefers-reduced-motion: reduce` → all durations 1 ms, shimmer replaced by static placeholders,
  waveform replaced by a discrete level meter ([[UI - Voice Search]] §4).
- No auto-advancing carousels, no auto-refresh, no time-limited interactions.
- The 30 s voice recording cap is announced at 25 s and can be stopped at any time (WCAG 2.2.1).
- Toasts auto-dismiss at 5 s but are always manually dismissable and never carry required
  information (WCAG 2.2.3).

---

## 6. Forms and Errors

- Every input has a programmatically associated label (visible or `aria-label`).
- Errors are announced, described in text, and placed adjacent to the field — never colour-only,
  never placeholder-only.
- `aria-invalid` and `aria-describedby` link an input to its error message.
- No error is a dead end: each provides a recovery path ([[UI - States and Errors]]).
- Autocomplete follows the WAI-ARIA combobox pattern exactly ([[UI - Component Library]] §2).

---

## 7. Zoom, Reflow, and Text Spacing

- 320 px width at 200 % zoom: no horizontal scrolling, no clipped content (WCAG 1.4.10).
- Text spacing overrides (line-height 1.5×, paragraph 2×, letter 0.12×, word 0.16×) must not clip or
  overlap (WCAG 1.4.12) — this is what breaks fixed-height cards, so cards use `min-height`.
- All sizing in `rem`, never `px`, for anything text-related, so browser font-size settings work.

---

## 8. RTL and Accessibility Together

- `lang` and `dir` are correct on `<html>` **and** on any element whose content differs
  ([[UI - RTL and Localization]]) — screen readers switch voice/pronunciation from `lang`, so an
  Arabic title inside a French page must carry `lang="ar"` or it is read with French phonetics.
- Accessible names are built from separated fields, never concatenated across directions, because a
  flattened bidi string reads as gibberish.
- Directional icons mirror; logos and semantic icons do not.

---

## 9. Testing

| Layer | Method | Gate |
|:---|:---|:---|
| Automated | `axe-core` on every page in 4 languages × 2 themes | zero violations in CI |
| Contrast | token-pair test | zero failures |
| Touch targets | bounding-box test | zero under 44 px |
| Keyboard | scripted `Tab` walk asserting a visible focus ring at each stop | per PR |
| Screen reader | manual NVDA / VoiceOver / TalkBack pass | per milestone |
| Zoom/reflow | 320 px @ 200 % | per milestone |
| Reduced motion | CSS audit + visual snapshot | per milestone |
| No-JS | core search flow | per milestone |

Automated tools catch roughly a third of real issues. The manual screen-reader pass is the one that
finds the problems that matter, and it is a milestone gate, not an optional extra
([[Testing Strategy]]).

---

## 10. Open Questions

- [ ] Who performs the manual screen-reader passes, and are any of them daily AT users? Testing by
      non-users finds different (and fewer) problems.
- [ ] Do we publish an accessibility statement with known gaps? (Leaning yes — honest and useful.)
- [ ] Is AAA contrast (7:1) worth targeting for body text given the low-quality screens common on the
      target devices?

## Related

[[UI - Design Language]] · [[UI - Component Library]] · [[UI - RTL and Localization]] ·
[[UI - States and Errors]] · [[UI - Results Page]] · [[Testing Strategy]]
