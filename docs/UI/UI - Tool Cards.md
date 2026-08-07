---
tags:
  - ui
  - tools
status: specified
updated: 2026-08-07
---

# UI — Tool Cards

> Routing, matching and data live in [[Instant Answers]]. This is what the user sees.

## 1. The shared frame

Every card is the same object with different contents. Consistency here is what makes an
unfamiliar tool immediately legible.

```
┌─ (2px accent rule, inline-start) ────────────────────────────┐
│  interpretation            45 × 1.19            ← --text-sm  │
│                                                              │
│  ANSWER                    53.55                ← --text-2xl │
│                                                              │
│  [ controls, if interactive ]                                │
│  ─────────────────────────────────────────────────────────   │
│  source · as of 14:20      [copy] [⋯]           ← --text-xs  │
└──────────────────────────────────────────────────────────────┘
```

**The interpretation line is not decoration.** It says how the query was read, so a misreading is
visible instead of silent. `20 dollar` interpreted as USD when the user meant Canadian is a wrong
answer; showing "20 USD → DZD" lets them see it in half a second.

**Rules:**
- One card. Never a stack.
- No reserved height. It appears or it does not ([[Performance Budgets]] CLS ≤ 0.05).
- The answer is selectable text, always, even when it is also a widget.
- Copy copies the answer alone, not the frame.
- The `⋯` menu holds: the explicit invocation (`!calc`), a link to how the tool works, and
  "don't show this tool" — a per-tool opt-out that persists.
- The accent rule means *the engine is asserting this* ([[UI - Design Language]] §6). Result
  cards never carry it.

## 2. Layout

| Breakpoint | Card |
|:---|:---|
| ≥ 1024 px | Full content width, above results |
| 640–1023 px | Same, controls may wrap |
| < 640 px | Full bleed to the gutters; controls stack; answer stays one line and shrinks a step before it wraps |

## 3. Per-tool

### Calculator
Answer is the result. Interpretation is the normalised expression with real operators — `×` and
`÷`, not `*` and `/`. Thousands separated per locale; **tabular numerals**. No keypad: the user
already has a keyboard and the search box is the input.

### Unit converter
Two selects and a number input, all live — changing a unit does not re-run the search. The
reciprocal conversion sits beneath in small text, because half of all conversions are immediately
re-done in the other direction. Algerian units (**qintar**, **sa'a**) appear in a group labelled
as such rather than buried in an alphabetical list.

### Currency
**Two rows, equal weight**: official and parallel. Each with its own value, source and `as_of`.

Never a single "the rate", never one visually dominant. A row older than 6 hours shows its age in
words ("measured 9 hours ago"); older than 48 hours the row is **absent**, and the card explains
which rate is missing rather than quietly showing one.

A one-line note states that the parallel rate is aggregated from reporting and is not an official
figure. It is body text, not a footnote.

### Translator
Source and target selects, a text area, and the output. Source language is detected and shown as
an editable guess. Output streams; a cancel control is present from the first token because on
CPU this takes tens of seconds ([[Summarizer]] §8).

A persistent line states that translation runs on this machine and the text is not sent anywhere.
That is the entire reason to use it instead of a hosted translator, so it is stated, not implied.

Darija appears in the language list at the top with the other Algerian-relevant options, not
alphabetically in the tail.

### Weather
Current temperature large, condition icon beside it, then five days as a row of compact columns:
day, icon, high/low.

Icons are a **custom line set matching the interface stroke weight** — not a third-party weather
font, not photographs. Every icon has a text label for assistive tech and for the case where the
icon is ambiguous (mist versus fog is not obvious in line art).

The wilaya is shown and is a control: changing it updates the card without a new search. Never
geolocated without an explicit action.

### Prayer times
Five times in a row, the next one marked with the accent and a relative countdown. The
**calculation method is on the card**, not in settings — times differing by minutes from a local
mosque is normal, and a card that hides its method makes that look like an error.

Hijri date beneath the Gregorian.

### Time & date
World clock: a large time, the zone, the date. Date arithmetic states the interpretation clearly
("from 7 August 2026 to 20 March 2027 — 225 days"). Hijri conversion shows both calendars and
names the reckoning used.

### Wilaya reference
A small definition list: code, seat, postal range, dial code. A static map thumbnail only if it
can be served from our own origin; otherwise no map.

### Dictionary
Headword, script variants, then senses per language. **Darija senses are marked as such** and
carry a note that coverage is incomplete, because a dictionary that silently lacks a word looks
like the word does not exist.

### Transliterator
Input and output side by side, both editable, converting in both directions. Alternatives are
listed when the transliteration is ambiguous — Arabizi frequently is, and picking one silently
hides that.

### Utility tools
QR, hashes, encoders, counters, converters. Single input, single output, copy. No card chrome
beyond the frame — these are one-line answers and should look like it.

## 4. Accessibility

- The card is `role="region"` with an accessible name naming the tool ("Currency converter").
- Its arrival is announced once via a polite live region — **the answer, not the frame**.
- Every control is keyboard-reachable in visual order; the card is skippable so a keyboard user
  reaching results does not tab through a converter first.
- Weather icons: `role="img"` with a label. Never icon-only meaning.
- Contrast holds in both themes including the accent rule ([[UI - Accessibility]]).
- Streaming translation output uses `aria-live="polite"` with `aria-busy` while generating, so a
  screen reader is not read a partial word at a time.

## 5. RTL

Everything uses logical properties, so the accent rule moves to the right edge in Arabic with no
direction-specific rule ([[UI - RTL and Localization]]).

Two things do **not** mirror:
- **Numbers and expressions.** `45 × 1.19 = 53.55` reads left-to-right inside an RTL line and is
  wrapped in `<bdi>` so bidi reordering cannot mangle it. A conversion rendered as `55.35 = 91.1 × 54`
  is the classic bidi failure and it is unreadable.
- **The five-day forecast.** Chronological order follows the reading direction — earliest at the
  start edge — but each column's internal layout is unchanged.

## 6. Failure

| Case | Rendering |
|:---|:---|
| No tool matched | Nothing. No empty state |
| Tool matched, data stale | Card renders without that value, and says which is missing |
| Tool matched, data absent | Nothing |
| Interactive tool errors after load | Inline message in the card; results untouched |
| Translation model busy | Retry affordance in the card |

The card is never an error surface. The results are the product; this is an addition to them.

## 7. Open questions

- [ ] Should a card be dismissible for the session, or only per-tool permanently? A dismissible
      card implies it might come back and invites fiddling.
- [ ] Weather icon set: draw one, or adapt an open set to the stroke weight? Drawing ~14 icons is
      a day and gives exact consistency.
- [ ] Does the currency card need a small history sparkline? Useful, and one more thing that can
      be stale or wrong.

## Related

[[Instant Answers]] · [[Tool Data Plane]] · [[UI - Design Language]] · [[UI - Accessibility]] ·
[[UI - RTL and Localization]] · [[UI - Results Page]] · [[Performance Budgets]]
