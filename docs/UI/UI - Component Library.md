---
tags:
  - ui
  - design
type: ui
status: specified
updated: 2026-08-06
---

# UI - Component Library

> Every reusable UI component: markup contract, states, and behaviour. Tokens come from
> [[UI - Design Language]]. Screens that compose these live in [[UI - Home Page]],
> [[UI - Results Page]], [[UI - Filters and Facets]].

---

## 1. Conventions

- Components are plain HTML with `data-component="name"` and BEM-ish utility classes.
- Behaviour attaches by `data-` attribute, never by class name — styling and behaviour stay
  independent.
- Every component defines: default, hover, focus-visible, active, disabled, loading, error, and
  (where relevant) empty.
- All components are logical-property based and therefore RTL-correct without a second stylesheet
  ([[UI - RTL and Localization]]).

---

## 2. `SearchBox`

```html
<form data-component="search-box" role="search" action="/search" method="get">
  <input type="search" name="q" autocomplete="off" spellcheck="false"
         enterkeyhint="search" dir="auto"
         aria-label="Search" aria-expanded="false" aria-controls="suggestions"
         aria-autocomplete="list" role="combobox">
  <button type="button" data-action="voice" aria-label="Search by voice">…</button>
  <button type="button" data-action="image" aria-label="Search by image">…</button>
  <button type="submit" aria-label="Search">…</button>
</form>
```

| Aspect | Spec |
|:---|:---|
| Height | 56 px (`sm`), 60 px (`lg`) |
| Radius | `--radius-full` |
| `dir="auto"` | **critical** — the box flips to RTL as soon as the first Arabic character is typed ([[UI - RTL and Localization]] §4) |
| Focus | `--shadow-md` + accent border, no layout shift |
| Clear button | appears when non-empty, 44 px target |
| Voice/image buttons | hidden when the API is unsupported or JS is off |
| Max length | 512 (matches [[API Contract]] §2) |
| Debounce | 120 ms before calling `/suggest` |

Keyboard: `↓`/`↑` move through suggestions, `Enter` submits the selected one or the raw text, `Esc`
closes suggestions and restores the typed text, `/` from anywhere focuses the box.

## 3. `SuggestionList`

`<ul id="suggestions" role="listbox">` with `role="option"` items and `aria-selected`. Rendered
below the box, `--z-dropdown`, max 8 items.

Each item: icon by `kind` (query / entity / transliteration / curated), the suggestion text with the
typed prefix in `font-weight: 500`. **The prefix is highlighted by index, not by injecting markup**
into the response string ([[Autocomplete Service]] §10).

States: hidden (default), open, empty (hidden entirely — never "no suggestions"), loading (previous
list stays, no spinner; a 40 ms spinner is worse than nothing).

## 4. `SummaryBlock`

The AI summary above results. See [[UI - Results Page]] §3 for behaviour.

| State | Rendering |
|:---|:---|
| `reserved` | fixed min-height 96 px placeholder, prevents CLS |
| `generating` | shimmer lines (static bar if `prefers-reduced-motion`) + "Summarising…" |
| `streaming` | text appears as it arrives; no per-token animation |
| `done` | full text + citation chips `[1] [2]` linking to result cards |
| `unavailable` | **block removed entirely**, results move up; no error text |

The summary is inserted with `textContent`, never `innerHTML` — it is model output derived from
untrusted crawled text ([[Security and Privacy]] §5).

Includes a persistent, non-dismissable label: *"AI summary — check the sources below."*

## 5. `ResultCard`

```html
<article data-component="result-card" dir="auto">
  <header>
    <span data-slot="badge">…</span>
    <a data-slot="url" href="…">elkhabar.com › economie</a>
    <time datetime="2026-08-04">4 August 2026</time>
    <span data-slot="sentiment">…</span>
  </header>
  <h3><a href="…" rel="noopener">Title</a></h3>
  <p data-slot="excerpt">…<em>term</em>…</p>
  <footer data-slot="engagement">…</footer>
</article>
```

| Slot | Rules |
|:---|:---|
| `badge` | platform: web / Facebook / Instagram / TikTok, icon + text |
| `url` | breadcrumb-style display URL, truncated with `text-overflow` |
| `time` | absolute date; relative ("2 days ago") in `title`. If `precision = "unknown"`, render "date unknown" — **never a fabricated date** |
| `sentiment` | icon + label; **omitted entirely when confidence is low** ([[Sentiment Engine]] §4.3) |
| `title` | 2 lines max, `-webkit-line-clamp` |
| `excerpt` | 3 lines max; only `<em>` from the server is preserved, everything else escaped |
| `engagement` | shown for social results only; omitted when all counts are 0 |
| `thumbnail` | 96 × 96, `loading="lazy"`, `referrerpolicy="no-referrer"`, `decoding="async"`; fixed box so a failed load does not shift layout |
| `matched_comments` | up to 2, indented, with their own sentiment; collapsible |
| `+N similar` | when the card represents a dedup cluster ([[Deduplication Service]] §4.5) |

The whole card is **not** one big link — the title is the link, so text is selectable and the URL is
copyable. Hover raises the title colour only.

## 6. `FilterChip` / `FilterBar`

Toggle chips for source and sentiment; a date control for range. Full behaviour in
[[UI - Filters and Facets]].

```html
<button data-component="filter-chip" role="switch" aria-checked="false" aria-label="Facebook, 700 results">
  <svg aria-hidden="true">…</svg> Facebook <span data-slot="count">700</span>
</button>
```

States: default, selected (`--color-accent-weak` background, accent border), disabled (count 0),
count-unavailable (count hidden when facets were dropped under load,
[[Error Handling and Resilience]] §6).

## 7. `Pagination`

Prev / page numbers / Next. 20 per page, max 100 pages. Current page has `aria-current="page"`.
Arrow direction is **logical** — in RTL, "next" points left ([[UI - RTL and Localization]] §5).
Result count renders as "about 1,800 results" when `estimated` is true.

No infinite scroll: it breaks back-button behaviour, breaks "share this page of results", and is an
engagement pattern rather than a utility one ([[UI Specification]] §2).

## 8. `Sheet` (mobile) and `Rail` (desktop)

Filters live in a bottom sheet below `lg` and a sticky rail at `lg`. The sheet is a `<dialog>` with
focus trap, `Esc` to close, backdrop click to close, and scroll lock on the body. Same content in
both — one component, two layouts.

## 9. `Toast`

Transient, `--z-toast`, `role="status"` (`aria-live="polite"`). Auto-dismiss at 5 s, manual close
always available. Used for "Link copied", "Filters cleared", "Microphone unavailable". **Never** for
errors that need action — those are inline ([[UI - States and Errors]]).

## 10. `Skeleton`

Placeholder for the summary block and result cards during load. Matches the real component's
dimensions exactly so nothing shifts. Shimmer via `background-position` animation; static under
`prefers-reduced-motion`. Marked `aria-hidden="true"` with a single `aria-live` region announcing
"Loading results".

## 11. `EmptyState`

Icon, one-line explanation, and **actionable** suggestions — not a shrug. For zero results:
"No results for X" plus: remove a filter (with the filters listed), try the transliterated form
(offered when the query looks Arabizi), broaden the date range. See [[UI - States and Errors]] §3.

## 12. `LanguageToggle`

Four options: العربية / Français / English / Darija (Arabizi input hint). Changes UI chrome language
and sets `dir`. Persisted in `localStorage` as a plain enum — the only thing we store, and it is
disclosed ([[UI Specification]] §9).

## 13. `Badge`

Platform and metadata badges. Text + icon, `--text-xs`, `--radius-sm`. Platform colours are muted
(not brand colours at full saturation) so a page of Facebook results does not turn blue.

---

## 14. Component Checklist

Every component ships with: all states styled, keyboard operation, an accessible name, RTL
verification, a `prefers-reduced-motion` variant, a dark-mode check, and an entry in the visual
regression suite ([[Testing Strategy]]).

## 15. Open Questions

- [ ] Should `ResultCard` show the domain favicon? It is a third-party request per card — likely
      proxied or dropped ([[Security and Privacy]] §9).
- [ ] Do citation chips in `SummaryBlock` scroll to the card, or highlight it in place?
- [ ] Is `+N similar` an expand-in-place or a new search?

## Related

[[UI - Design Language]] · [[UI - Results Page]] · [[UI - Home Page]] · [[UI - Accessibility]] ·
[[UI - RTL and Localization]] · [[UI - States and Errors]]
