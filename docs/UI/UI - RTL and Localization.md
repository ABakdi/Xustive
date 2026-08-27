---
tags:
  - ui
  - i18n
type: ui
status: implemented
updated: 2026-08-27
---

# UI - RTL and Localization

> Four UI languages, two writing directions, and text that mixes both in the same paragraph.
> Parent: [[UI Specification]] · Related backend: [[Language Detector]], [[Query Expander]]
>
> Audited against the code on 2026-08-27 (`web/lib/i18n/*`, `web/app/[lang]/layout.tsx`,
> `globals.css`, `scripts/lint-bidi.sh`, `scripts/rtl-icons.sh`). Where the first version described a
> mechanism that was built differently, the built one is described and the old one marked
> superseded.

---

## 1. The Actual Problem

Algerian text is not neatly one language. A single Facebook post routinely contains Arabic script,
French words, Latin-script Darija, and ASCII digits — sometimes in one sentence. The UI has to render
that correctly *without knowing in advance* what any given string contains.

So the rule is: **direction is determined per-string at render time, not per-page**.

---

## 2. UI Languages

`web/lib/i18n/config.ts`: `LOCALES = ['ar', 'ary', 'fr', 'en']`, `DEFAULT_LOCALE = 'ar'`,
`RTL = ['ar', 'ary']`.

| Code | Language | UI direction | Notes |
|:---|:---|:---|:---|
| `ar` | العربية (MSA) | RTL | the default when nothing else matches |
| `fr` | Français | LTR | |
| `en` | English | LTR | |
| `ary` | Darija | RTL | UI strings in Darija written in Arabic script; falls back to MSA per key (§8) |

Darija as a *UI* language is a real choice, not a token one: administrative Arabic reads as formal
and distant to many users, and Darija chrome ("قلب على…", "ما لقينا والو") is warmer and clearer.
It also signals what the product is for.

**Selection — superseded 2026-08-27.** The first version had `navigator.language` → override →
`localStorage` `xustive.lang`. What was built is URL-first: the language is the **first path
segment** (`/ar/search?q=…`), so every language is a linkable URL and nothing is stored
client-side. The bare root (`web/app/page.tsx`) negotiates from `Accept-Language`
(`negotiate()` — Darija matched before Arabic, because `ary` starts with `ar` and a naive prefix
match would silently give a Darija reader MSA) and redirects. The header's `LangSwitcher` is a
menu of four real links to the *same path and query* under another locale, so switching
mid-results re-runs the search (§12).

---

## 3. Layout Direction

`<html lang="ar" dir="rtl">` is set server-side in `app/[lang]/layout.tsx` from the path segment
(`dirOf(lang)`), so the first paint is already correct — no flash of mirrored layout. The same
layout preloads **one** font file chosen by direction (Arabic 400 for RTL, Latin variable for LTR).

**Every layout property is logical**, no exceptions:

| Never use | Always use |
|:---|:---|
| `margin-left` / `right` | `margin-inline-start` / `-end` |
| `padding-left` / `right` | `padding-inline-start` / `-end` |
| `text-align: left` | `text-align: start` |
| `left` / `right` | `inset-inline-start` / `-end` |
| `border-left` | `border-inline-start` |
| `float: left` | `float: inline-start` |

Tailwind's logical utilities (`ms-*`, `me-*`, `ps-*`, `pe-*`, `start-*`, `end-*`, `text-start`) are
what the components use — a grep on 2026-08-27 finds no `ml-`/`mr-`/`pl-`/`pr-`/`left-`/`right-`
utility anywhere outside the admin console. `scripts/lint-bidi.sh` §4 fails on a physical side
property in `app/*.css`; it does **not** yet check Tailwind class names, so the utility rule is held
by review. There is **no separate RTL stylesheet** — that approach always drifts.

Flexbox and grid handle direction automatically; the two-column results grid puts the knowledge
rail on the inline-end side in both directions for free.

Two things logical CSS cannot do, and how they are handled:

- **Scrolling a row sideways.** `scrollBy({ left })` is physical. The relation row's arrows
  (`ListPanel.tsx`, M8-T11) read `getComputedStyle(el).direction` and flip the sign, so the
  "forward" arrow always advances in reading order. The arrows themselves sit at `start-1` /
  `end-1` and use the logical `chevron-start` / `chevron-end` glyphs with `rtl-flip` (§6).
- **The inside of an SVG.** See §6.

---

## 4. Per-String Direction (`dir="auto"`)

Every element rendering user or corpus content gets `dir="auto"`, which sets direction from the
string's first strong directional character:

| Element | Where |
|:---|:---|
| Search input | `SearchBox.tsx` — flips the moment the first Arabic character is typed, including live voice partials |
| Result card | the whole `<li>` in `ResultCard.tsx`, so title, excerpt and URL line resolve together |
| Suggestion item | each `role="option"` |
| AI summary | badge, text and note in `Summary.tsx` |
| Entity panel | title, description, extract, fact rows, sources line |
| Relation row | heading, "see also" chips, card captions |
| Related-search chips, error detail, voice error line | `search/page.tsx`, `SearchBox.tsx` |

Without this, an Arabic title in an LTR page renders with its punctuation on the wrong side — the
classic "why does the question mark appear at the start" bug.

---

## 5. Bidi Hazards

The cases that break in practice, and their fixes:

| Case | Problem | Fix |
|:---|:---|:---|
| URL inside RTL text | `elkhabar.com/economie` reorders and becomes unreadable | `<bdi>` — `display_url` on every card; `lint-bidi.sh` §1 fails a rendered `display_url`/`host`/`domain` without one |
| Number + Arabic unit | `20 نتيجة` may reorder | `<bdi class="numeric">` around the count; `numeric` sets `tabular-nums` |
| Bracketed number | `(12 ms)` renders as `(ms 12` — the *brackets* are the neutral characters | `<bdi>` around the whole group; `lint-bidi.sh` §2 fails a `(…{formatNumber|formatDate` without one nearby |
| Date in a card | `08/08/2026` flips to `2026/08/08` | `<bdi>` around the formatted date |
| Citation marker `[3]` | same as brackets | `<bdi>` in `Summary.tsx` |
| Mixed excerpt with `<em>` | highlight spans can straddle a direction run | the engine emits `<em>` per whole matched token; never split a word |
| Truncation with ellipsis | ellipsis lands on the wrong side | `truncate` / `line-clamp` only on elements whose `dir` is already resolved (the card, the tile) |
| User content in `aria-label` | screen readers read a flattened string | build labels from separated fields, not concatenated ones |

Rule of thumb: **anything that concatenates a number, a URL, or a Latin token into an Arabic string
needs `<bdi>`.** `dir="auto"` on the container is not a substitute — it resolves the container, it
does not isolate the runs inside it.

---

## 6. Icons and Direction

| Icon | RTL behaviour |
|:---|:---|
| Chevrons, arrows, next/prev | **mirror** |
| Back / forward | mirror |
| Search, mic, camera, filter, calendar | do **not** mirror |
| Wordmark | never mirrors — `dir="ltr"`, always Latin |
| Sentiment ▲ ▼ | do not mirror (vertical semantics) |

Implemented via `[dir='rtl'] .rtl-flip { transform: scaleX(-1) }` in `globals.css`
(superseded 2026-08-27: the class is `rtl-flip`, not `icon-directional`). Only icons explicitly
marked directional get it — mirroring everything is the common mistake and it produces backwards
logos. `scripts/rtl-icons.sh` fails the build if a file imports a directional lucide icon
(`ChevronLeft`, `ArrowRight`, `Reply`, …) without `rtl-flip` appearing in it.

Two design choices keep the mirrored set tiny:

- Pagination uses the **words** "previous" / "next", not chevrons.
- The hand-drawn `Icon` set (`components/ui/Icon.tsx`, used by the server-rendered panels) names
  its chevrons **logically** — `chevron-start` points against the writing direction — and they are
  the only directional glyphs in it.

---

## 7. Numbers, Dates, Currency

`web/lib/i18n/format.ts`:

| Item | Rule |
|:---|:---|
| Digits | **Western Arabic numerals (0–9) everywhere**, including in Arabic UI — `numberingSystem: 'latn'` set explicitly, because `Intl` defaults Arabic to ٤٥. Algerian print and signage use Western digits. |
| Locale passed to `Intl` | `ar` and `ary` → `ar-DZ`; `fr`, `en` as-is. Darija has no CLDR data, and a bare `ary` yields the root locale and silently loses Arabic month names. |
| Number formatting | `formatNumber` → `Intl.NumberFormat` |
| Dates | `formatDate` → `Intl.DateTimeFormat`, long month |
| Arabic month names | come from CLDR `ar-DZ` — **أوت** not أغسطس, **جويلية** not يوليو. Still to be verified by a native reader against every month. |
| Relative time | not used (no `Intl.RelativeTimeFormat` in the app) |
| Calendar | Gregorian (Hijri is an open question) |

The month-name detail is small and it matters: أغسطس reads as foreign to an Algerian reader. The same
applies to [[Content Parser]] §4.3, which must *parse* both forms.

**Known drift (2026-08-27), outside the shared helpers:** `EntityPanel.tsx` formats numbers and
dates with `new Intl.NumberFormat(lang)` / `DateTimeFormat(lang)` — raw locale, no `latn` — so an
Arabic entity panel shows Eastern digits and a Darija one falls to the root locale.
`WeatherDetail.tsx` does the same for weekday/hour; `ToolCard.tsx` maps `ary → ar` but not to
`ar-DZ`. All three should call `format.ts`. Belongs to [[UI - Results Page]] / [[UI - Tool Cards]].

---

## 8. String Management

**Superseded 2026-08-27** — not JSON files. Strings live in **`web/lib/i18n/messages.ts`** as four
TypeScript objects, flat keys (`resultsCount`, `voiceStop`, `lang_ar`, `'unit-converter'`, …).

- `Messages` is typed from the Arabic catalogue, so **a key missing from `fr` or `en` is a compile
  error** — that is the "missing key fails CI" rule, enforced by `tsc` rather than a build warning.
- `ary` is `{ ...ar, …overrides }`: about seventy keys are Darija today; the rest read as MSA. A
  missing Darija string is therefore invisible, which is the trade for shipping Darija at all
  before a native reviewer exists (B7).
- Pluralisation uses **`Intl.PluralRules`** (`plural()` in `format.ts`). Arabic has six plural
  categories — `zero`, `one`, `two`, `few`, `many`, `other` — and `count === 1 ? x : y` is wrong
  for four of them. Today every category maps to the same word (`resultsCount` = "نتيجة"), so the
  mechanism is in place but not yet exercised.
- No named-placeholder interpolation exists. The one composed sentence — "About **1,800** results
  (12 ms)" — is assembled in JSX from `resultsApprox`, the number, `resultsCount` and `took`, with
  `<bdi>` around the number groups. That works for the four languages we have because all of them
  put those pieces in the same order; it is the one place the "no concatenation" rule is bent, and
  it should move to a placeholder string if a fifth language ever breaks the order.
- Three names are still hard-coded English in components: "Clear", "Summary", "Source N"
  ([[UI - Accessibility]] §3).

---

## 9. Fonts

Per [[UI - Design Language]] §4: `--font-arabic` (IBM Plex Sans Arabic, Cairo, Noto fallbacks)
applied via `:lang(ar), :lang(ary)` in `globals.css`, with `font-size: 1.04em` and
`line-height: 1.85`, because Arabic at nominally equal size reads smaller and needs more leading.
(Values updated 2026-08-27; the first version said 1.08em / 1.75.)

Arabic must never be rendered in a font lacking proper shaping — broken letter joining is instantly
recognisable as amateur work and is a real risk with default Android fallbacks. The stack is honest
about what exists: leading it with an uninstalled `Inter` once broke shaping outright.

---

## 10. Testing

What exists (2026-08-27):

- `scripts/lint-bidi.sh` (`make lint`): URLs rendered without `<bdi>`, bracketed `formatNumber` /
  `formatDate` without `<bdi>`, a page tree with no `dir`, physical side properties in CSS.
- `scripts/rtl-icons.sh` (`make ui-gates`): directional lucide icon without `rtl-flip`.
- `tsc`: a message key missing from any catalogue.

What the first version promised and does not exist: visual regression snapshots in four languages,
a `?pseudo=1` pseudo-localisation mode, a bidi fixture set, a CI lint for missing `dir="auto"` on
content slots, and a native-speaker review step. The last is the one that matters most and is
tracked as B7.

---

## 11. Open Questions

- [ ] Who writes and reviews the Darija UI strings? This needs a named person, not a hope (B7).
- [ ] Should the UI language default follow the *query* language rather than `Accept-Language`?
- [ ] Hijri dates as a display option?
- [ ] Do we support Tamazight (Latin or Tifinagh) chrome? It is an official national language, so the
      honest answer is "eventually" — and the scope should be decided rather than drifted into.
- [ ] Extend `lint-bidi.sh` to Tailwind physical utilities, and to `Intl.*` calls outside
      `format.ts` (§7 drift).

---

## 12. What the language switch changes besides the words

Added 2026-08-27. Choosing a language is not cosmetic; the page sends it to the engine as `ui`
(`search/page.tsx` → `/api/v1/search?…&ui=ar`) and three things follow:

| Effect | Where | Behaviour |
|:---|:---|:---|
| **Ranking** | `crates/xustive-search/src/rank.rs`, weight `ui_language` (default 0.10) | a result whose detected `language` matches the UI language gets the bonus; `ary` counts as `ar`. A French reader sees French pages nudged up for the same query. |
| **Summary language** | `search.rs` → `OutputLang::from_ui(ui)` | the AI summary is written in the **reader's** language, not the query's or the sources': `fr` → French, `en` → English, `ar`/`ary`/anything else → Modern Standard Arabic. A Darija reader gets MSA — there is no Darija output mode. |
| **Instant answers** | `xustive_tools::best_in(raw, ui_lang)`, currency, weather | unit names, place names and interpretations render in the UI language. |

Because `LangSwitcher` keeps the path and query, switching on a results page is a new search with
a new `ui` — the ranking shifts and the summary is regenerated in the new language. That is the
intended behaviour and worth knowing when comparing results across languages.

The voice recorder also passes the UI language as a hint to the transcriber (`transcribe(blob,
uiLang)`), so a short Arabic clip is not mis-detected as something else ([[UI - Voice Search]]).
Its strings (`voiceListening`, `voiceTranscribing`, `voiceStop`, the error lines) are all in the
catalogues; the meter, the seconds counter and the stop button are flex children, so they sit on the
inline-end side of the field in both directions with no direction-specific rule.

## Related

[[UI - Design Language]] · [[UI - Component Library]] · [[UI - Accessibility]] · [[Language Detector]] ·
[[Query Expander]] · [[Content Parser]] · [[UI Specification]]
