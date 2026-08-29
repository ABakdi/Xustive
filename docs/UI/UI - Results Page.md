---
tags:
  - ui
type: ui
status: built
updated: 2026-08-27
---

# UI - Results Page

> Route `/[lang]/search?q=…`. The main surface of the product.
> Parent: [[UI Specification]] · Data: [[API Contract]] §2–3 · Components: [[UI - Component Library]]
> · Rail and relation row: [[UI - Knowledge Panel]] · Tabs: [[UI - Search Verticals]]

> **Audited 2026-08-27 against `web/app/[lang]/search/page.tsx`.** The 2026-08-06 text was a
> pre-build spec. Where it has been overtaken — the streaming summary, the filter rail, the
> `replaceState` model, the card slots that never shipped — the built behaviour is what is
> described below and the earlier position is kept in a "superseded" line so the reasoning stays.

---

## Did you mean

Above the vertical tabs, below the result count. Two forms, one line each, both `dir="auto"` so
the corrected query reads correctly in either script:

- **Offered** — *Did you mean **couscous algerien**?* — the correction is a link that re-runs the
  search with it.
- **Applied** — *Showing results for **annaba** · Search instead for annba* — the results below
  are the correction's; the link back carries `exact=1`, which tells the API to search what was
  typed and correct nothing.

Server-rendered with the rest of the page: the correction arrives in the search response
(`query.corrected`, `query.corrected_applied`), so it is there before any JavaScript runs. See
[[Query Pipeline]] §4.3.1 for when each form appears.

## 1. Layout

The page is one Server Component (`export const dynamic = 'force-dynamic'`) with a small `Shell`
around it. Three things in the tree are client components: the search box, the AI summary, and the
knowledge rail (entity panel + relation row). Everything else — the whole result list included —
ships as HTML in the first response.

### `lg` (≥ 1024 px)

```
┌────────────────────────────────────────────────────────────────────────────┐
│ XUSTIVE  [ 🔍 سونلغاز فاتورة              ✕ 🎤 📷 ]     [🌐 العربية] [☾] [≡] │ ← sticky header
├────────────────────────────────────────────────────────────────────────────┤
│ [ banner slot — the relation row (cast / books / films / albums), full width ]│
├──────────────────────────────────────────────────┬─────────────────────────┤
│ about 1,800 results (60 ms)                       │  Entity panel           │
│ [All] [News] [Files] [Images] [Videos]            │  (sticky, 360 px,       │
│ ┌ Tool card (calculator / weather / …) ─────────┐ │   `lg:top-20`)          │
│ ┌ ✦ AI summary — prominent when the query is a  │ │                         │
│ │   question, otherwise below the filters       │ │                         │
│ language ○ar ○fr  ·  source ○web  ·  tone ○…     │ │                         │
│ ┌ ResultCard ────────────────────┐               │ │                         │
│ …×20                                              │ │                         │
│ Related searches: [chip] [chip]                  │ │                         │
│ [Previous] [1] [2] [3] [Next]                     │ │                         │
├──────────────────────────────────────────────────┴─────────────────────────┤
│ Searches are never linked to you  (→ /[lang]/privacy)                      │ ← footer
└────────────────────────────────────────────────────────────────────────────┘
```

- **Header**: `sticky top-0`, `z-index: var(--z-sticky)`, `max-w-3xl`; `Wordmark size="sm"`, the
  `SearchBox` in `compact` mode, then `LangSwitcher`, `ThemeToggle`, `DensityToggle`.
- **Main**: `max-w-3xl`, widening to `lg:max-w-5xl`. The `banner` slot renders above both columns;
  the two-column grid is `lg:grid-cols-[minmax(0,1fr)_360px] lg:gap-8`. The `aside` wrapper is
  `self-start lg:sticky lg:top-20` so the rail stays in view while the list scrolls.
- **Footer**: a border-top line with the privacy statement linking to `/[lang]/privacy`. Added
  after BUG-030 — a shared results URL was a dead end, the only privacy link lived on the home page.
- The error shell (§6) is the same `Shell` without an `aside`.

*Superseded 2026-08-27*: the 2026-08-06 drawing put a **filter rail** on the right (date, source,
sentiment, language). Filters are a chip row inside the results column instead
([[UI - Filters and Facets]]), and the right column is the knowledge rail.

### Below `lg`

Single column. The rail falls **under** the results (`mt-6`), not beside them; the relation row
keeps its full width above. There is no bottom sheet and no "⚙ Filters" button — the chip row
wraps. The 680 px results measure in the old spec is not enforced; the column is whatever the
`max-w-3xl` / `max-w-5xl` container leaves it.

---

## 2. Render Sequence

```
t=0      server fetches GET /api/v1/search?q&page&hits_per_page=20&ui=<lang>[&lang&source&sentiment&v]
t≈70ms   HTML arrives with meta line, tabs, tool card, filters, all 20 cards, pagination
after paint (client effects):
         Summary       → POST /api/v1/summary {token}        (tens of seconds on CPU)
         EntityPanel   → GET /api/v1/knowledge → fallback GET /api/knowledge-live
         ListPanel     → GET /api/knowledge-list               (relation queries only)
```

The user reads and clicks results at ~70 ms. The three client fetches are additive: each shows a
small loading line, then either fills or collapses to nothing.

*Superseded 2026-08-27*: [[ADR-0004 - Stream Summary Separately from Results]] described an
`EventSource` stream with a **reserved 96 px box** to hold CLS at 0.05. The built summary is a
single POST that resolves once, and it **reserves no height**: most summaries never arrive (the
model refuses, the queue is full), and a placeholder that then collapses moves the results out
from under the reader — the exact defect the reservation was meant to prevent. The loading line
is short and sits where the summary will sit, so the shift when it arrives is one paragraph.

---

## 3. Summary Block

`components/search/Summary.tsx`. Three states, not two:

| State | Rendering |
|:---|:---|
| Loading (`data === null`) | `<section class="assert rise" aria-live="polite" aria-busy="true">` with a sparkle icon, three pulsing dots (`aria-hidden`) and `t.summaryLoading` ("Generating summary…") |
| Resolved, no summary | **removed entirely**, no error text |
| Resolved | badge chip `t.summaryBadge` ("AI summary", sparkle icon, accent wash — the same chip the entity panel uses for its kind), the text, the sources row, the note |

| Property | Spec |
|:---|:---|
| Placement | above the filters when `is_question` (`prominent`, `.summary-answer`, `text-lg`, lifted shadow) — "a question gets its answer first"; below the filters otherwise (`text-base`) |
| Insertion | React children — escaped by construction. `[n]` markers are split out by regex and become `<a href="#result-<id>">` superscripts (`aria-label="Source n"`); a marker with no matching citation is dropped, never rendered as a link to nothing |
| Citations | a row under the text: `t.sources` ("sources:") then one chip per citation, `[n]` in a pill linking to the result card's `id="result-<id>"` (cards have `scroll-mt-24` so the sticky header does not cover them) |
| Label | `t.summaryNote` — "Generated from the results below. Check the sources." — persistent, `text-xs`, muted |
| Language | the UI language: the API builds the prompt with `OutputLang::from_ui(ui)` from the `ui=` param the page sends, so switching the interface language switches the summary's language |
| Visual | carries the `.assert` rule (3 px inline-start accent border, tinted surface) because this is the engine asserting something; result cards never carry it |
| Copy button | **not built** |

Design position unchanged: the summary is deliberately quieter than the results when it is a
précis. When the query was a question the *placement* changes, not the content — the text and
citations are identical either way.

---

## 4. Result List

- 20 per page (`hits_per_page=20`), rendered in API order; the client never reorders.
- The list is `<ol class="list-none p-0">` as a one-column grid with `gap: var(--result-gap)`
  (12 px, 6 px in compact density). When the API returns an `interaction_token` the list is wrapped
  in `InteractionBeacon` — one delegated capture-phase click listener that `sendBeacon`s
  `{t, d}` to `/api/v1/interaction`; without the token there is no listener at all
  ([[Interaction Signals]], M6).
- Images and Videos tabs replace the list with `ImageGrid` / `VideoList` from `MediaGrid.tsx`
  ([[UI - Search Verticals]]).
- Keyword highlighting: `title` and `excerpt` arrive from the API already escaped with only `<em>`
  left in, and are inserted with `dangerouslySetInnerHTML`. The escaping boundary is the API, not
  the client ([[Security and Privacy]] T8).
- Meta line: `{about} {N} {results} ({X} ms)` — "about" (`t.resultsApprox`) only when
  `pagination.estimated`. The count is `Intl.NumberFormat` with Latin digits; the plural form is
  `Intl.PluralRules`; the parenthesised group is wrapped in `<bdi>` because unisolated brackets
  swap in an Arabic line.
- **Related searches** (M7): `data.related` renders as a chip row (`<section aria-label>`), each a
  real link to `/[lang]/search?q=<term>`.
- Dedup `+N similar` — `similar_count` is in the type but **not rendered**.

### Card anatomy (as built — `ResultCard.tsx`)

`<li id="result-<id>" class="card" dir="auto">`, a pure Server Component:

| Slot | Rules |
|:---|:---|
| source badge | `t[source_type]` in a bordered `--radius-sm` chip |
| "From the web" | `t.fromTheWeb` accent chip when `from_web` — a live SearXNG result not yet in the index (M7) |
| display URL | `<bdi class="truncate">` — isolated so an RTL line does not reorder it |
| date | `formatDate` (long month, Latin digits) inside `<bdi><time datetime=…>`; when `published_at_precision === 'unknown'` or `published_at ≤ 0` → `t.dateUnknown`. **Never a fabricated date** |
| sentiment | glyph ▲ ● ▼ (`aria-hidden`) + `t[label]`; omitted entirely when `sentiment` is `null` — the API already withholds low-confidence labels ([[Sentiment Engine]] §4.3) |
| title | `<h2>` with `<a href rel="noopener nofollow" data-doc=<id>>`; `overflow-wrap: anywhere` |
| excerpt | `<p class="text-sm">`, muted |

Not built (spec 2026-08-06, still open): thumbnails on cards, engagement footer,
`matched_comments`, line-clamping, relative date in `title`. Result cards are text-only today;
media lives in the Images/Videos verticals.

---

## 5. Interaction

| Action | Result |
|:---|:---|
| Click a title | navigate (same tab; `rel="noopener nofollow"`); the beacon, if any, fires on capture and never delays it |
| Middle-click / ⌘-click | new tab, native |
| Toggle a filter chip | **a real link** → full navigation with the new URL (scroll resets) |
| Change page | real link, scroll resets |
| Back button | ordinary browser navigation — the page is re-rendered from the URL; there is no in-memory session cache |
| Edit the query in the header box | `router.push` to the new URL (JS) or a plain `GET` form submit (no JS) |
| Switch language / theme / density | language: a link to the same route under another locale; theme and density: a Server Action writes a cookie and the DOM attribute flips immediately |

*Superseded 2026-08-27*: the `replaceState`-per-filter / `pushState`-per-search model and the
session cache were never built. Every state change is a URL and a navigation, which is simpler and
what keeps the no-JavaScript path identical to the JavaScript one.

No link is intercepted for tracking. There is no click-through redirect and no `ping` attribute —
the `href` is the destination ([[Security and Privacy]] P1). The interaction beacon carries an
opaque token and a document id, never the query.

---

## 6. States

| State | Rendering |
|:---|:---|
| Loading | there is no client-side loading state for the page itself: the browser's navigation is the loading state, and results are in the first response |
| Success | as above |
| Zero results, All tab | `t.noResults` + `t.noResultsHint` ("Try other words, or fewer") |
| Zero results, a vertical | the vertical is named (`t.noNews` / `noFiles` / `noImages` / `noVideos`) and a link (`t.noNewsHint`, "Show all results") drops the `v=` param |
| Facets dropped under load | `facets_degraded` → chips absent and a faint line `t.filtersUnavailable`, so a bare row reads as "resting", not "nothing to filter" |
| Summary unavailable | block absent |
| **Search failed** (BUG-041) | `t.errorTitle` + one of: 504 / `upstream_timeout` → `t.errorSlow`; 429 → `t.errorRateLimited`; 503 / `search_unavailable` → `t.errorUnavailable`; any other API error → its message; fetch threw (API down / restarting) → `t.errorUnreachable`. Under it, `t.errorRetry` ("Try again") linking to `/[lang]/search?q=<q>` — the query is preserved in the header box too |
| Offline mid-session | `OfflineBanner` (assertive `role="status"`); the rendered page stays readable and the form submits once back online |

The `Retry-After` countdown for 429 in the 2026-08-06 spec is not built; the text asks the reader
to wait a moment. Full catalogue in [[UI - States and Errors]].

---

## 7. Performance

| Metric | Budget | Enforced by |
|:---|:---|:---|
| JS, search route | ≤ 195 KB gzipped (~152 KB is React + Next) | `scripts/bundle-budget.sh` (`make check`) |
| No-JS path works | search, filter, page 2, real `<form>` | `scripts/no-js-check.sh` |
| CLS across the sequence | ≤ 0.05 | not measured in CI |
| DOM nodes | ≤ 1 500 | not measured |

The 2026-08-06 numbers (400 ms first render on 3G, 16 ms for 20 cards, INP ≤ 200 ms) are still
the intent; none is measured in CI today. Cards are server-rendered React, not a `<template>`
clone — that description belonged to the vanilla-JS renderer this page replaced.

---

## 8. Accessibility

- Results are an `<ol>` with each card an `<li>`; the title is an `<h2>`.
- The summary is an `aria-live="polite"` region and announces **once when it arrives** (there is
  no streaming, so nothing per token); while loading it is `aria-busy`.
- The entity panel's skeleton and the relation row's loading line are `aria-busy` + `aria-live`
  with a visible/sr-only "Loading…" ([[UI - Knowledge Panel]]).
- Filter chips are **links** with `aria-current="true"` when selected, grouped in
  `role="group" aria-label` per facet — not `role="switch"` (that promised an in-place toggle;
  these navigate). The vertical tabs are links with `aria-current="page"`.
- Pagination is a `<nav aria-label={t.page}>`; the current page is a `<span aria-current="page">`,
  not a link to itself.
- Directional icons are `.rtl-flip`; the pagination uses **words** (Previous / Next) rather than
  chevrons, which sidesteps the mirroring question entirely.
- **Not built**: the "Skip to results" link and the `aria-live` result-count announcement.
- Full commitments in [[UI - Accessibility]].

---

## 9. Open Questions

- [ ] On `sm`, should the summary be collapsed by default so links are above the fold?
- [x] Do citation chips scroll to the card, or highlight it in place? — **scroll**: `#result-<id>`
      anchors with `scroll-mt-24`. (2026-08-27)
- [ ] Should social results have a visually distinct card shape? (no social connector yet)
- [ ] Is "about N results" useful to users at all, or is it search-engine cargo cult?
- [ ] `Filters` and `Pagination` build hrefs from `q`, `lang`, `source`, `sentiment` — **not `v`**:
      paging or filtering inside the Images/Videos/News tab drops the reader back to All. Bug or
      decision? (observed 2026-08-27)
- [ ] Skip link and the result-count live region (§8) — small, both still worth doing.

## Related

[[UI - Component Library]] · [[UI - Knowledge Panel]] · [[UI - Filters and Facets]] ·
[[UI - Search Verticals]] · [[UI - States and Errors]] · [[API Contract]] · [[Summarizer]] ·
[[Ranking and Relevance]] · [[UI - Accessibility]]
