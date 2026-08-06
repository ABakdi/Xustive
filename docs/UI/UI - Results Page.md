---
tags:
  - ui
type: ui
status: specified
updated: 2026-08-06
---

# UI - Results Page

> Route `/search?q=…`. The main surface of the product.
> Parent: [[UI Specification]] · Data: [[API Contract]] §2–3 · Components: [[UI - Component Library]]

---

## 1. Layout

### `lg` (≥ 1024 px)

```
┌────────────────────────────────────────────────────────────────┐
│ XUSTIVE  [ 🔍 سونلغاز فاتورة            🎤 📷 ]      [عربية ▾] │ ← sticky header
├────────────────────────────────────────────────────────────────┤
│ [الكل] [ويب] [فيسبوك] [إنستغرام] [تيك توك] │ [التاريخ ▾] [الشعور ▾] │ ← filter bar
├──────────────────────────────────────┬─────────────────────────┤
│ ┌──────────────────────────────────┐ │  Filters (sticky rail)  │
│ │ ✦ AI summary (streaming)      [1]│ │  ◻ Date range           │
│ │   …2-3 sentences…             [2]│ │  ◻ Source               │
│ │   AI summary — check sources     │ │  ◻ Sentiment            │
│ └──────────────────────────────────┘ │  ◻ Language             │
│                                      │                         │
│ about 1,800 results (0.06 s)         │                         │
│                                      │                         │
│ ┌ ResultCard ────────────────────┐   │                         │
│ ┌ ResultCard ────────────────────┐   │                         │
│ …×20                                 │                         │
│         ‹ 1 2 3 … ›                  │                         │
└──────────────────────────────────────┴─────────────────────────┘
```

Results column: max 680 px. Rail: 260 px, sticky below the header.

### `sm` (< 640 px)

Single column. Filter bar becomes a horizontally scrollable chip row with a `[⚙ Filters]` button
opening the bottom sheet. Summary block sits above results, collapsed to 3 lines with a "more"
affordance if it exceeds them.

---

## 2. Render Sequence

This ordering is the whole point of the two-request design ([[ADR-0004 - Stream Summary Separately from Results]]):

```
t=0     navigate → skeleton (header + filter bar real, summary + 5 card skeletons)
t≈70ms  GET /search returns → render facets, counts, and all 20 cards
        summary block enters `reserved` state with a fixed 96px min-height
t≈80ms  open EventSource /search/summary?token=…
t≈600ms first `delta` → text begins filling the reserved box
t≈2s    `done` → citation chips render
```

The user can read and click results at ~70 ms. Nothing below the summary ever moves, because the box
reserved its height before the first token — this is the CLS ≤ 0.05 requirement
([[UI Specification]] §4).

If `summary_token` is `null`, or the stream errors, the block is **removed** and the results move up
once, immediately, before the user has settled — never mid-read.

---

## 3. Summary Block

| Property | Spec |
|:---|:---|
| Max height | 3 lines at `--text-lg`, expandable |
| Reserved height | 96 px (`sm`: 84 px) |
| Insertion | `textContent` only — never `innerHTML` ([[Security and Privacy]] §5) |
| Citations | `[1] [2]` chips after the relevant sentence, linking to the matching card |
| Label | "AI summary — check the sources below" (persistent, `--text-xs`, muted) |
| Failure | block removed silently; **no error message** |
| Copy | a copy button that copies the text plus the cited source URLs |

Design position: the summary is deliberately visually *quieter* than the results — a tinted surface,
not a hero box. It is an aid, not the answer ([[UI Specification]] §2).

---

## 4. Result List

- 20 per page ([[API Contract]] §2), rendered in the order the API returns; the client never reorders.
- Card anatomy in [[UI - Component Library]] §5.
- Keyword highlighting: the server sends `<em>` inside `excerpt`; the client escapes everything else
  and permits only that one tag. This is the XSS boundary ([[Security and Privacy]] T8).
- Result meta line: "about 1,800 results (0.06 s)" — the word "about" appears whenever
  `pagination.estimated` is true, which for Meilisearch is usually.
- Dedup clusters render as one card with `+N similar`, expanding inline
  ([[Deduplication Service]] §4.5).

### Date rendering

| `published_at_precision` | Display |
|:---|:---|
| `second` / `day` | absolute date, relative in `title` |
| `month` | "August 2026" |
| `unknown` | **"date unknown"** — we never render a guessed date as fact ([[Data Model]] §2) |

### Sentiment badge

Icon + text label + colour, all three. Omitted entirely when confidence is below threshold —
absence is more honest than a shrug ([[Sentiment Engine]] §4.3, [[UI - Accessibility]] §4).

### Thumbnails

Fixed 96 × 96 box, `loading="lazy"`, `referrerpolicy="no-referrer"`, `decoding="async"`. A failed
load leaves the box empty rather than collapsing it — no layout shift. Pending the decision on
proxying thumbnails ([[Security and Privacy]] §9).

---

## 5. Interaction

| Action | Result |
|:---|:---|
| Click a title | navigate (same tab; `rel="noopener"`) |
| Middle-click / ⌘-click | new tab, native behaviour preserved |
| Toggle a filter chip | re-fetch, `replaceState`, results re-render, scroll position kept |
| Change page | fetch, `pushState`, scroll to the top of the results |
| Back button | restore from the in-memory session cache; no re-fetch, no re-render flicker |
| Edit the query in the header box | full new search, `pushState` |
| Expand `matched_comments` | in-place, no request |

No link is intercepted for tracking. There is no click-through redirect and no `ping` attribute —
the `href` is the destination ([[Security and Privacy]] P1).

---

## 6. States

| State | Rendering |
|:---|:---|
| Loading | skeleton summary + 5 skeleton cards; filter bar already real |
| Success | as above |
| Zero results | [[UI - States and Errors]] §3 — with actionable suggestions, not a dead end |
| Partial (facets dropped) | chips render without counts, no error shown |
| Summary unavailable | block absent |
| Search 503 | full-page error with retry, query preserved in the box |
| 429 | "Too many searches — wait a moment", with the `Retry-After` seconds counted down |
| Offline mid-session | banner; cached page stays readable |

---

## 7. Performance

| Metric | Budget |
|:---|:---|
| Time to first result rendered | ≤ 400 ms on 3G after response |
| Render 20 cards | ≤ 16 ms (one frame) |
| CLS across the full sequence | ≤ 0.05 |
| INP on filter toggle | ≤ 200 ms |
| DOM nodes | ≤ 1 500 |

Cards render via a `<template>` clone + `textContent` assignment — no string concatenation into
`innerHTML`, which is both slower and the XSS footgun.

---

## 8. Accessibility

- Results are an `<ol>` with each card an `<li>`/`<article>`; the count is announced via
  `aria-live="polite"` once per search ("About 1,800 results").
- The streaming summary is in an `aria-live="polite"` region that announces **once on completion**,
  not per token — per-token announcement makes a screen reader unusable.
- Skip link: "Skip to results" jumps past the header and filter bar.
- Filter chips are `role="switch"` with `aria-checked` and include the count in their accessible name.
- Focus is preserved across filter re-renders — the chip the user just toggled keeps focus.
- Full commitments in [[UI - Accessibility]].

---

## 9. Open Questions

- [ ] On `sm`, should the summary be collapsed by default so links are above the fold?
- [ ] Do citation chips scroll to the card, or highlight it in place?
- [ ] Should social results have a visually distinct card shape given their different content shape
      (short text, heavy engagement, image-led)?
- [ ] Is "about N results" useful to users at all, or is it search-engine cargo cult?

## Related

[[UI - Component Library]] · [[UI - Filters and Facets]] · [[UI - States and Errors]] ·
[[API Contract]] · [[Summarizer]] · [[Ranking and Relevance]] · [[UI - Accessibility]]
