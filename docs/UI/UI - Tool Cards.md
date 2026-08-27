---
tags:
  - ui
  - tools
status: implemented
updated: 2026-08-27
---

# UI — Tool Cards

> Routing, matching and data live in [[Instant Answers]]. This is what the user sees.
>
> **Audited against the code 2026-08-27.** The card is `web/components/tools/ToolCard.tsx`
> (server-rendered) with `CopyButton.tsx`, `DismissTool.tsx` and `WeatherDetail.tsx`; translation
> has its own client card, `TranslateCard.tsx`. Preferences: `web/lib/tools.ts`, `web/lib/prefs.ts`,
> and the `/settings` page. Where the 2026-08-07 spec and the shipped card differ, the shipped card
> is described and the spec is marked superseded.

## 1. The shared frame

Every card is the same object with different contents. Consistency here is what makes an
unfamiliar tool immediately legible.

```
┌─ (3px --assert-accent rule, inline-start; tinted surface) ───────┐
│  interpretation            45 × 1.19            ← --text-xs, muted│
│                                                                   │
│  ANSWER    53.55            [copy] [hide this tool]  ← --text-2xl │
│  alternatives · other readings                  ← only if any     │
│  tool body (currency line · weather forecast)   ← only for those  │
│  as of 14:20  |  administered price, not a quote ← --text-xs      │
└───────────────────────────────────────────────────────────────────┘
```

`<section class="assert group mb-7" aria-label={tool name}>`. The `.assert` rule is now a tinted,
elevated surface (`--surface`, `--shadow`, `--radius`) with a 3 px `border-inline-start` in
`--assert-accent` — a **second hue** used only here and on the summary, so the engine's assertions
are one colour and nothing else is ([[UI - Design Language]] §6). Result cards never carry it.

**The interpretation line is not decoration.** It says how the query was read, so a misreading is
visible instead of silent. `20 dollar` interpreted as USD when the user meant Canadian is a wrong
answer; showing "20 USD → DZD" lets them see it in half a second. It is wrapped in `<bdi>`.

**Server-rendered.** The answer is computed by the API and arrives with the search response, so the
card is visible before any JavaScript runs and survives the no-JS path (`scripts/no-js-check.sh`).
Only the copy button hydrates.

**Rules:**
- One card. Never a stack. (The page renders `data.instant` — a single answer.)
- No reserved height. It appears or it does not ([[Performance Budgets]] CLS ≤ 0.05).
- The answer is selectable text, always: `<p class="numeric text-2xl"><bdi>value</bdi></p>`,
  weight 550. Exam results are the exception — the value is an official URL rendered as a link
  (§3).
- **Copy** (`CopyButton`, `.ghost` icon button, lucide `Copy` → `Check` for 1.6 s) copies the
  answer alone, not the frame. Its `aria-label`/`title` flips from `copy` to `copied` so a screen
  reader hears the result. A clipboard refusal is swallowed — the text is selectable anyway.
- **Hide this tool** (`DismissTool`, `hideTool`) is a real `<form>` posting a Server Action
  (`setToolEnabled(tool, false)`), so it works without JavaScript. It is `variant="quiet"` and
  `opacity-0` until the card is hovered or the button is focused (`group-hover`, `focus-visible`)
  — a control for the rare case, kept from making every answer look provisional. The set of
  disabled tools lives in the `xustive-tools-off` cookie (sorted, capped at 32 ids of `[a-z-]{1,32}`),
  and is re-enabled per tool on `/settings`, which lists the inventory from `GET /api/v1/tools`
  with each tool's `!keyword`. It is a cookie and not a query parameter on purpose: the set of
  tools someone has switched off is small, stable and unusual enough to fingerprint them, so it is
  filtered *here* after the API answers rather than sent to it (`lib/tools.ts`).
- **Superseded (2026-08-27):** there is no `⋯` menu. The spec's three items — the explicit
  invocation, a "how this works" link, and the opt-out — became one visible opt-out on the card;
  the `!keyword` is shown on the settings page instead.
- There is no "source · as of" footer row. `asOf` + a localised time (`Intl.DateTimeFormat`, `ar`
  for Darija) is shown **only when `as_of` is present** — arithmetic has no age, a rate always
  does — and `administered` ("Administered price, not a live quote") is shown instead for values
  fixed by an authority. The two are mutually exclusive by construction.

## 2. Layout

The card sits above the results and below the verticals, full content width, inside the results
column (not across the knowledge rail). Header row is `flex items-baseline gap-3`; on narrow
screens the copy/hide controls stay on the answer line and the answer wraps rather than shrinks —
the spec's "shrinks a step before it wraps" was never implemented.

| Breakpoint | Card |
|:---|:---|
| ≥ 1024 px | Results column width, above results, beside the rail |
| < 1024 px | Full content width; same layout |

## 3. Per-tool

Tool ids as the API names them (and as the `Messages` keys that title each card): `calculator`,
`unit-converter`, `currency`, `weather`, `prayer-times`, `date`, `wilaya`, `exam`, `fuel`,
`transliterate`, `utility`, `translate`. Every tool except `translate` uses the generic frame with
the API's `interpretation` and `value`; the extras below are the only per-tool rendering.

### Calculator
Answer is the result. Interpretation is the normalised expression with real operators — `×` and
`÷`, not `*` and `/`. Tabular numerals via `.numeric`. No keypad: the user already has a keyboard
and the search box is the input. Thousands separation is whatever the API emitted — the card does
not re-format `value`.

### Unit converter
**Superseded (2026-08-27):** the live two-selects-and-an-input widget was not built. The card
shows the API's conversion as a static answer; changing a unit means a new search. The reciprocal
line and the "Algerian units" group are not shown.

### Currency
**Superseded (2026-08-27, M8-T06.4): one row, not two.** The spec wanted official and parallel
rates with equal weight. What ships is the official rate only, and the card says so:

- the answer, then a `currencyRate` line — `1 USD = 134.52 DZD · official` (`currencyOfficial`),
  from `detail.unit_rate` / `from` / `to`, 2 decimals for rates ≥ 1 and 6 below;
- when `detail.parallel_available === false`, a faint `currencyNoParallel` note: *"Parallel-market
  rate not shown: no source publishes it verifiably."*

The square-market rate is absent because nothing publishes it in a way that can be checked — not
because it does not matter — and a reader who assumed this is what a bureau would give them would
have been misled by omission, so the card names the gap rather than hiding it. `asOf` shows the
rate's age.

### Translator
`TranslateCard`, the **one** client component in the instant-answer path, rendered instead of
`ToolCard` when `instant.tool === 'translate'` — and only after the page has fetched the language
list (`translateLanguages()`, cached an hour, fetched only on translation queries). Without
JavaScript there is no card at all rather than a broken one.

- A `<textarea rows=2 dir="auto">` pre-filled from the query (`translatePlaceholder` as label).
  Enter translates; Shift+Enter is a newline.
- Two `Select`s: `translateFrom` (with `translateAuto` = detect) and `translateTo` (default `ar`).
  Names come from the API list in the UI language (`name_ar` for ar/ary, `name_en`, `name_fr`).
- `translateButton` / `stop` toggle. Output streams token by token from `POST /api/v1/translate`
  (SSE; body, never query string — the most sensitive field the service handles). Changing either
  language re-runs; typing does not (a stream per keystroke would thrash the two model slots).
  Any abort — new run, Stop, unmount — closes the connection, which frees the model worker.
- Output `<p aria-live="polite" aria-busy dir="auto"><bdi>…</bdi></p>`, `translating` shown until
  the first token. States: idle · running · done · failed · truncated.
- A persistent footer line: `translateLocal` ("Translated on this server. The text does not leave
  it.") or `translateApprox` when the target language is flagged `approximate`; `translateTruncated`
  / `translateFailed` appended on those states. Stated on every translation, not only the doubtful
  ones: a 3B model has no way to signal which pair it is bad at.

The language order is whatever the API returns; the spec's "Darija at the top" is not enforced in
the component.

### Weather
`WeatherDetail` (server-rendered, no JavaScript — an inline SVG and a `<details>`, because a chart
library would cost more than the page's bundle budget and leave the no-JS path an empty box).

- `weatherAssumed` first, when `detail.assumed_place` is true: *"Approximate location from your
  connection — type a wilaya name to change it."* The place is a guess from the connection, never
  a geolocation prompt, and correcting it is one word in the query.
- **Hourly graph** (`HourlyGraph`, `weatherNext24`): a `280×64` SVG `polyline` of the next 24
  temperatures in `--accent`, with translucent bars behind the hours whose precipitation chance is
  ≥ 30 %. `role="img"` with an `aria-label` of the range ("Next 24 hours: 12° – 21°"). First hour,
  min–max, last hour as a caption row. **Drawn LTR regardless of page direction** — a time axis
  does not mirror.
- **Day strip**: the first three days as `DayChip`s (weekday, glyph, `high° / low°` in `<bdi>`);
  days 4+ behind `<details><summary>weatherWeek</summary>`.
- **Superseded (2026-08-27, M8-T05.7): glyphs, not a drawn icon set.** WMO codes map to text
  glyphs (☀ ⛅ ☁ 🌫 🌦 🌧 ❄ ⛈) with an `sr-only` label (`wmoClear` … `wmoStorm`). They inherit
  the reader's font and colour, cost no bytes and cannot fail to load; the custom line set is still
  §7's open question. The current-conditions block ("temperature large, icon beside it") is the
  generic answer line — `value` carries the temperature and condition.
- The wilaya is **not** a control on the card; it is part of the query.

### Prayer times
Generic frame only. **Superseded (2026-08-27):** the five-times row, the next-prayer accent and
countdown, the on-card calculation method and the Hijri line are not rendered — whatever the API
puts in `value` and `interpretation` is what shows. The spec stays as the target because the
argument still holds: a card that hides its method makes a minutes-off time look like an error.

### Time & date
Generic frame (`date`). Interpretation carries the reading; `value` the answer.

### Wilaya reference
Generic frame (`wilaya`). No definition list, no map.

### Exam results
`detail.official === true` turns the answer into a link: `value` is the authoritative portal URL,
rendered `<a target="_blank" rel="noopener noreferrer" dir="ltr">` in accent, underlined. No result
is shown or stored — only the way to the one official source, and the student's visit stays their
own.

### Fuel prices
Generic frame plus `administered` — the ARH sets the price and changes it without announcement, so
there is no `as_of` and the card says the number is an administered price, not today's quote.

### Dictionary
**Not built** (no dictionary tool exists in `xustive-tools`). The spec is kept for when it is.

### Transliterator
Generic frame plus an `alternatives` line ("Other reading: …", joined with `·`, in `<bdi>`) when
`detail.alternatives` is a non-empty string array. Shown inline rather than behind a control:
Arabizi is genuinely ambiguous, the runner-up is often the one the user meant, and presenting one
reading as settled would be worse than admitting the choice. The spec's side-by-side editable
input/output is not built.

### Utility tools
Generic frame (`utility`). Single answer, copy. No chrome beyond the frame.

## 4. Accessibility

- The card is a `<section>` with `aria-label` = the tool's translated name (`t[tool]`, falling
  back to the id) — a landmark region a keyboard user can skip past.
- **No live-region announcement on arrival**: the card is in the initial HTML, so there is nothing
  to announce. The spec's "announce the answer once" applies only to the translation output,
  which is `aria-live="polite"` with `aria-busy` while streaming so a screen reader is not read a
  partial word at a time.
- Copy button: icon-only with an `aria-label` that changes to "Copied" on success. Hide button:
  visible text, `focus-visible` brings it back from `opacity-0`.
- Weather: the hourly SVG is `role="img"` with a labelled range; day glyphs are `aria-hidden` with
  an `sr-only` WMO label. Never icon-only meaning.
- Contrast holds in both themes including the assert rule (`--assert-accent` is redefined for
  dark) ([[UI - Accessibility]]).

## 5. RTL

Everything uses logical properties, so the accent rule moves to the right edge in Arabic with no
direction-specific rule ([[UI - RTL and Localization]]).

Things that do **not** mirror:
- **Numbers and expressions.** `45 × 1.19 = 53.55` reads left-to-right inside an RTL line and is
  wrapped in `<bdi>` so bidi reordering cannot mangle it — the interpretation, the value, the rate
  line, the alternatives and the high/low pairs are all isolated.
- **The hourly graph** is forced `direction: ltr` (SVG and its caption row).
- **The day strip** is a `flex-wrap` list, so it follows the reading direction — earliest at the
  start edge — with each chip's internal layout unchanged, as the spec asked.
- Exam URLs are `dir="ltr"`; translation output is `dir="auto"` because its direction follows the
  target language, not the interface.

## 6. Failure

| Case | Rendering |
|:---|:---|
| No tool matched (`instant` absent) | Nothing. No empty state |
| Tool dismissed by the reader | Nothing — filtered from the cookie before render |
| Tool matched, data stale | `asOf` shows the age; nothing is withheld by age on the client |
| Tool matched, `detail` malformed | The extra body is skipped (every `detail` field is type-checked); the answer line still renders |
| Translation request fails / stream ends early | `translateFailed` on the footer line; the Translate button is the retry |
| Translation hits the token limit | `translateTruncated` |
| Translation languages list unavailable | No card at all (the generic frame is not used for `translate`) |

The card is never an error surface. The results are the product; this is an addition to them.

## 7. Open questions

- [ ] Should a card be dismissible for the session, or only per-tool permanently? *(Built:
      per-tool, persistent for a year, reversible on `/settings`.)*
- [ ] Weather icon set: draw one, or adapt an open set to the stroke weight? Drawing ~14 icons is
      a day and gives exact consistency. *(Interim: text glyphs, §3.)*
- [ ] Does the currency card need a small history sparkline? Useful, and one more thing that can
      be stale or wrong.
- [ ] Prayer times: build the five-times row with the method on the card, now the data is there.
- [ ] Unit converter: is the live widget worth its JavaScript, or is "search again" fine?

## Related

[[Instant Answers]] · [[Tool Data Plane]] · [[UI - Design Language]] · [[UI - Accessibility]] ·
[[UI - RTL and Localization]] · [[UI - Results Page]] · [[UI - States and Errors]] ·
[[Performance Budgets]]
