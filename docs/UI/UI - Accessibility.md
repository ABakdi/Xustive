---
tags:
  - ui
  - a11y
type: ui
status: implemented-partially
updated: 2026-08-27
---

# UI - Accessibility

> Target: **WCAG 2.2 Level AA**. Not a checklist appended at the end — the commitments below are
> constraints on the component specs in [[UI - Component Library]].
> Parent: [[UI Specification]]
>
> **Audited against the code on 2026-08-27.** The target is unchanged. What changed is the
> "verified by" column: the automated harness the original note assumed (axe in CI, a touch-target
> test, a scripted Tab walk) was never built, and several behaviours it specified (skip links, the
> `/` shortcut, a 25 s recording warning) do not exist. Where a commitment is met by the code it says
> so; where it is met by a script, that script is named; where it is a hope, it says *not yet*.

---

## 1. Commitments

| # | Commitment | Verified by (today) |
|:---|:---|:---|
| A1 | Every function is keyboard-operable | by construction: every control is a native `<a>`, `<button>`, `<input>`, `<select>` or `<form>` — no div-buttons anywhere. Manual pass only. |
| A2 | Visible focus on every interactive element | the global `:focus-visible` rule in `globals.css` (2 px accent outline). No scripted Tab walk. |
| A3 | Text contrast ≥ 4.5:1 (≥ 3:1 for large/UI) | `scripts/contrast-audit.mjs`, both themes, run by `make ui-gates` — **not by CI** |
| A4 | No information conveyed by colour alone | sentiment = glyph + text; current chip = fill **and** `aria-current`; highlight = weight + wash. Design review. |
| A5 | Touch targets ≥ 44 × 44 px | **not met.** Chips are 34 px tall, suggestion rows 40 px, header ghost buttons 32 px. No automated test. |
| A6 | Every image has a meaningful `alt` or `aria-hidden` | icons are `aria-hidden` (`Icon.tsx`, every lucide use); media tiles carry the result title. Lint: none. |
| A7 | Correct heading hierarchy, no skipped levels | **not met on the results page**: it has no `<h1>` — result titles, the entity panel and the relation row are all `<h2>`. Settings, privacy and the OCR tool have an `<h1>`. |
| A8 | Live regions announce changes without flooding | see §3 — by inspection, not by a screen-reader pass |
| A9 | Works at 200 % zoom and 320 px width with no horizontal scroll | `rem` type scale; the relation row and video list scroll inside their own container. Manual only. |
| A10 | `prefers-reduced-motion` honoured everywhere | partially: `.rise` is gated by `no-preference`, the voice button's breathing by `reduce`. Tailwind `animate-pulse` on the summary dots and the entity-panel skeleton is **not** gated. |
| A11 | Works with JS disabled for core search | `scripts/no-js-check.sh` (curl the results page: results in HTML, GET form, filter and page links) via `make ui-gates` |

---

## 2. Keyboard

| Key | Behaviour (today) |
|:---|:---|
| `Tab` / `Shift+Tab` | document order: wordmark → search box (input, clear, mic, camera) → header toggles → verticals → tool card → filters → results → related → pagination → footer. **No skip links.** |
| `↓` `↑` | move through suggestions (`aria-activedescendant`); wrapping past the end returns to what was typed |
| `Enter` | submit the box (the highlighted suggestion if there is one); activate a link or button |
| `Esc` | close suggestions and restore the typed text; while recording, cancel the recording ([[UI - Voice Search]]) |
| `Tab` (list open) | closes the suggestion list |

Rules:
- **No keyboard traps.** There is no `<dialog>` anywhere in the app any more — the voice modal was
  replaced by an inline recorder (2026-08-27), so nothing traps or restores focus.
- Filter chips are **links**, not toggle buttons: `Enter` follows them, `Space` does nothing
  (2026-08-27 — supersedes the earlier "`Space` toggles a chip"). Toggling a filter is a full
  navigation, so focus does not survive it; it lands at the top of the new document.
- The `/` shortcut to focus the search box specified in the first version was never implemented.
  Still worth doing; recorded here so nobody assumes it exists.

---

## 3. Screen Readers

Test set unchanged — **NVDA + Firefox**, **VoiceOver + Safari (iOS)**, **TalkBack + Chrome
(Android)**, with TalkBack mattering most for this audience. No pass has been recorded yet
(§9).

Accessible names as the code sets them (`web/lib/i18n/messages.ts` keys in brackets):

| Element | Accessible name |
|:---|:---|
| Search input | `aria-label` = `searchLabel` ("بحث" / "Search"), `role="combobox"` |
| Clear button | `aria-label="Clear"` — **hard-coded English, not translated** (gap) |
| Mic button | `voiceSearch` when idle, `voiceStop` while recording; `aria-pressed` carries the state |
| Camera link | `ocrByImage` — a link to `/{lang}/tools/ocr`, not a button |
| Filter group | `role="group"` + `aria-label` = the facet name; each chip's text is "label + count" (e.g. "Facebook 700"), so the count is in the name |
| Sentiment | glyph `aria-hidden`, label text visible ("Sentiment: negative" is **not** how it reads — it reads as the bare label) |
| Result title | the title text alone (an `<h2>` wrapping the link) |
| Vertical tabs | `<nav aria-label={verticalAll}>`, current tab `aria-current="page"` |
| Pagination | `<nav aria-label={page}>`; current page is a `<span aria-current="page">`, not a link; prev/next are **words** (`previous`/`next`) |
| Theme / density toggle | `aria-label` names the **current** state (`themeSystem`… / `densityComfortable`…), not the action |
| Language menu | button `aria-haspopup="menu"` + `aria-expanded`; items are `role="menuitem"` links with their own `lang`/`dir`, current one `aria-current` |
| AI summary | `<section aria-label="Summary">` and citation links `aria-label="Source N"` — **both hard-coded English** (gap) |
| Relation row | `<section aria-label>` = the relation heading; scroll arrows `listScrollBack` / `listScrollForward` |
| Entity panel | `<aside aria-label>` = the entity title |
| Tool card | `<section aria-label>` = the tool's translated name |
| Settings toggle | `aria-pressed` + `aria-label` "Enable/Disable: tool" + `aria-describedby` the row label |

The "Similarity tile" row from the first version (reverse image search) is described in
[[UI - Image Search]]; its tiles carry the qualitative label as text.

### Live regions — the part that usually goes wrong

| Region | Politeness | Announces |
|:---|:---|:---|
| Result count | — | **none.** A search is a full page load, so the count is read in document order. |
| AI summary | polite, `aria-busy` while loading | the loading line once, then the finished text once. The summary is **not streamed** — it arrives in one response — so "never per token" is satisfied trivially. |
| Suggestions | — | no "N suggestions available" message; the combobox exposes the list itself |
| Voice state | polite (`<span aria-live="polite">` beside the button) | elapsed seconds while recording, then `voiceTranscribing` |
| Voice errors | `role="status"` (polite) under the field | permission / unavailable / failed messages — visible text, not screen-reader-only (2026-08-27) |
| Offline banner | `role="status"`, **assertive** while offline, polite for "back online" | `offline` + `offlineHint`, then `backOnline` for 3 s |
| Entity panel / relation row | polite, `aria-busy` while loading | the skeleton, then the panel |
| OCR / translation | polite (`ImageOcr.tsx`, `TranslateCard.tsx`) | progress and result text |

The first version specified the voice state as *assertive* ("Recording", "Transcribing"); the
inline recorder uses polite, because the person just pressed the button and already knows.
Errors are `role="status"` rather than `role="alert"` for the same reason: they sit directly
under the field the reader is in.

---

## 4. Colour and Contrast

- `scripts/contrast-audit.mjs` reads the tokens straight out of `globals.css` (no second list to
  drift), converts oklch → WCAG luminance and checks ten pairs per theme: `--fg`, `--fg-muted` and
  `--accent` on `--bg` / `--surface` / `--bg-sunk` at 4.5:1; `--accent-fg` on `--accent` at 4.5:1;
  `--fg-faint` and `--line-strong` on `--bg` at 3:1. It exits non-zero on a failure. Run on
  2026-08-27: all pass. The tightest pairs are the 3:1 ones — `--fg-faint` on `--bg` at 3.12:1
  light and `--line-strong` at 3.01:1 dark — not the 4.6:1 muted pair the first version named.
- It runs from `make ui-gates`, not from `.github/workflows/ci.yml`. "The build fails on a
  regression" is therefore true of the *make target*, not of every PR.
- **Sentiment is never colour alone**: `aria-hidden` glyph + text label + colour (A4).
- The current filter chip, vertical tab and pagination page are filled *and* carry `aria-current`.
- Search-term highlighting (`<em>` from the engine) is **weight 550 plus an inset `--accent-wash`
  underline**, not italic — an underline survives greyscale as well as italics did and reads
  better in Arabic, where italics are not a thing.
- Links inherit colour and are distinguished by placement and hover underline; the accent is used
  for links that are actions (retry, related searches, citations).
- Dark mode is a separate token set (`:root[data-theme='dark']`) and is audited independently.

---

## 5. Motion and Timing

- Motion budget in `globals.css`: `.rise` = 100 ms ease-out, opacity + 3 px translate, inside
  `@media (prefers-reduced-motion: no-preference)`; the recording button's 1.4 s breathe is turned
  off under `reduce`. The summary's pulsing dots and the entity skeleton use Tailwind's
  `animate-pulse` and are **not** gated — a known gap (A10).
- No auto-advancing carousels, no auto-refresh. The relation row scrolls only when you ask it to.
- The 30 s voice cap (`MAX_MS` in `VoiceButton.tsx`) ends the recording; **there is no 25 s
  warning** (superseded 2026-08-27 — the inline recorder shows elapsed seconds instead, and stop
  is always one tap or `Esc`). Stop = search, which is a time-limited outcome the reader can see
  coming from the counter; whether that satisfies 2.2.1 for a screen-reader user is open.
- There are no toasts. The one transient message is the 3 s "back online" note, and it carries no
  required information.

---

## 6. Forms and Errors

- Every input has a programmatically associated label (`aria-label` on the search box, a visible
  `<label>` on `Select`, `aria-describedby` on the settings `Toggle`).
- Search errors (504 / 429 / 503 / unreachable, [[UI - States and Errors]]) are page content with a
  "Try again" link — a full-page state, so no live region is needed.
- Voice and OCR errors are text under the control, `role="status"` / `aria-live="polite"`.
- `aria-invalid` is used nowhere: there is no field validation in the app (the search box accepts
  anything). Keep it for the day a form has one.
- Autocomplete follows the WAI-ARIA combobox pattern: `role="combobox"`, `aria-expanded`,
  `aria-controls`, `aria-autocomplete="list"`, `aria-activedescendant`, `role="listbox"` /
  `role="option"` with `aria-selected` ([[UI - Component Library]] §2).

---

## 7. Zoom, Reflow, and Text Spacing

- Type scale in `rem` (`--text-xs` … `--text-2xl`); Arabic gets `1.04em` and line-height 1.85 by
  `:lang()`, so browser font-size settings scale everything.
- Some control heights are in `px` (`min-block-size: 34px` chips, 38/48 px field, 32 px ghost
  buttons). They do not clip text at 200 % because they are minimums, but they are also why A5
  fails — raising them to 44 px fixes both.
- Cards use padding, not fixed heights, so text-spacing overrides (WCAG 1.4.12) do not clip.
- Nothing has been checked at 320 px / 200 % on a device. Manual, per milestone, still unscheduled.

---

## 8. RTL and Accessibility Together

- `lang` and `dir` are set on `<html>` server-side ([[UI - RTL and Localization]] §3). The
  language-menu items carry their own `lang` and `dir`. **Result cards carry `dir="auto"` but not
  `lang`** — an Arabic title in a French page is laid out correctly but a screen reader will read
  it with French phonetics. The API returns `language` per result; passing it to `lang` on the
  card is the fix.
- Accessible names are built from separated fields, never concatenated across directions
  (the tool toggle label is the one concatenation, and both halves are in the UI language).
- Directional icons mirror via `rtl-flip`; the wordmark and semantic icons do not
  ([[UI - RTL and Localization]] §6).
- `app/not-found.tsx` is `lang="en"`, LTR, English-only — it sits outside the locale layout.

---

## 9. Testing

| Layer | Method (today) | Gate |
|:---|:---|:---|
| Contrast | `node scripts/contrast-audit.mjs` — token pairs, both themes | `make ui-gates` |
| Directional icons | `scripts/rtl-icons.sh` — a lucide arrow/chevron without `rtl-flip` fails | `make ui-gates` |
| No-JS | `scripts/no-js-check.sh` — curl the results page, assert results/form/filter/page links | `make ui-gates` (needs web on :3000) |
| Bidi | `scripts/lint-bidi.sh` — URLs and bracketed numbers in `<bdi>`, no physical CSS sides | `make lint` |
| Automated a11y (`axe-core`) | **not set up** | — |
| Touch targets | **not set up** | — |
| Keyboard Tab walk | manual | — |
| Screen reader | manual NVDA / VoiceOver / TalkBack — **no pass recorded yet** | per milestone (unscheduled) |
| Zoom/reflow, reduced motion | manual | per milestone (unscheduled) |

The original table promised axe in four languages × two themes with zero violations in CI. That
is still the right target and it is the cheapest of the missing pieces; recorded as not done rather
than left implied. Automated tools catch roughly a third of real issues; the manual screen-reader
pass is the one that finds the problems that matter, and it remains a milestone gate on paper only
([[Testing Strategy]]).

---

## 10. Open Questions

- [ ] Who performs the manual screen-reader passes, and are any of them daily AT users? Testing by
      non-users finds different (and fewer) problems.
- [ ] Do we publish an accessibility statement with known gaps? (Leaning yes — honest and useful.
      This note is most of the content already.)
- [ ] Is AAA contrast (7:1) worth targeting for body text given the low-quality screens common on the
      target devices? Body text is at 18:1 already; the question is really about `--fg-muted` (5.3:1).
- [ ] Add axe to `make ui-gates`, then to CI. Add an `<h1>` to the results page (visually hidden
      is fine). Translate the three hard-coded English names ("Clear", "Summary", "Source N").
      Gate `animate-pulse` on reduced motion. Raise touch targets to 44 px.

## Related

[[UI - Design Language]] · [[UI - Component Library]] · [[UI - RTL and Localization]] ·
[[UI - States and Errors]] · [[UI - Results Page]] · [[UI - Voice Search]] · [[Testing Strategy]]
