---
tags:
  - ui
  - design
type: ui
status: built
updated: 2026-08-27
---

# UI - Component Library

> Every reusable UI component: file, props, states, and behaviour. Tokens come from
> [[UI - Design Language]]. Screens that compose these live in [[UI - Home Page]],
> [[UI - Results Page]], [[UI - Filters and Facets]], [[UI - Knowledge Panel]], [[UI - Tool Cards]].

> **Audited 2026-08-27 against `web/components/**`.** The 2026-08-06 version described HTML
> contracts with `data-component` attributes for a vanilla renderer. The components are React now
> (Server Components wherever nothing is interactive) and this note names the actual files and
> props. Superseded items are kept with the date so the reasoning survives.

---

## 1. Conventions

- Server Components by default. `'use client'` appears only where there is state or an effect:
  `SearchBox`, `VoiceButton`, `Summary`, `EntityPanel`, `ListPanel`, `InteractionBeacon`,
  `OfflineBanner`, `LangSwitcher`, `ThemeToggle`, `DensityToggle`, and the interactive tool
  pieces (`TranslateCard`, `ImageOcr`, `WeatherDetail`, `CopyButton`).
- **No Radix / shadcn runtime.** shadcn is the reference for *structure* (variant prop,
  `focus-visible` ring, state not carried by colour alone) but its primitives are client
  components, and the result list shipping as markup is the most valuable property this frontend
  has. `Button`, `Select`, `Toggle` are plain elements with a class.
- Styling: Tailwind v4 utilities for layout plus a handful of semantic classes in `globals.css`
  (`.field`, `.chip`, `.chip-active`, `.chip-clear`, `.chip-count`, `.card`, `.assert`, `.rise`,
  `.ghost`, `.btn-quiet`, `.numeric`, `.rtl-flip`, `.list-row`, `.summary-answer`,
  `.voice-button`, `.voice-meter`). Colours come from tokens (`var(--fg-muted)` etc.), never
  literals — except two deliberate brand-dot maps in the knowledge components.
- All components use logical properties (`ms-`, `start-`, `insetInlineStart`) and are therefore
  RTL-correct without a second stylesheet ([[UI - RTL and Localization]]). Directional SVGs carry
  `.rtl-flip`; `scripts/rtl-icons.sh` fails the build otherwise.
- Every user-visible string is a key on `Messages` (`lib/i18n/messages.ts`), typed against the
  Arabic catalogue so a missing key in another locale is a compile error. Components take `t`.

*Superseded 2026-08-27*: `data-component="…"` / behaviour-by-`data-` attribute — there is no
separate behaviour layer to attach.

## 2. `SearchBox` — `components/search/SearchBox.tsx` (client)

Props: `lang`, `t`, `initialQuery?`, `compact?` (header variant: 38 px min height, 16 px icons;
home: 48 px, 18 px).

A real `<form role="search" action="/[lang]/search" method="get">` around a real
`<input type="search" name="q">`, so it submits without JavaScript. The wrapper is `.field` — a
**pill** (`--radius-pill`), lifted, with an inset accent-wash underline on hover/focus.

| Aspect | Spec |
|:---|:---|
| `dir="auto"` | the box flips to RTL on the first Arabic character ([[UI - RTL and Localization]] §4) |
| `maxLength` | 512 ([[API Contract]] §2) |
| ARIA | `role="combobox"`, `aria-expanded`, `aria-controls`, `aria-autocomplete="list"`, `aria-activedescendant` |
| Debounce | **90 ms**, prefix ≥ 2 chars, `/api/v1/suggest?limit=8`, aborts the previous request |
| Clear | one `✕` button (`aria-label="Clear"` — untranslated, see open questions) shown when non-empty; the browser's own `::-webkit-search-cancel-button` is hidden in CSS so there is never two |
| Voice | `VoiceButton` inline — renders nothing where the browser cannot record; while recording the placeholder becomes `t.voiceListening`, a level meter (`.voice-meter`, four bars) and elapsed seconds appear, interim words show dimmed in the box, and stop **submits**. Errors appear as a `role="status"` line under the field ([[UI - Voice Search]]) |
| Image | a `Camera` **link** to `/[lang]/tools/ocr` (`aria-label={t.ocrByImage}`) — a link, not a button, so it works without JS and opens in a new tab ([[UI - Image Search]]) |
| Focus | `.field:focus-within` — border-strong + shadow, no layout shift |
| Storage | nothing. No local history: "the most private thing a search engine holds is what you started to type and then deleted" |

Keyboard: `↓`/`↑` move through suggestions and wrap to the typed text; `Enter` submits the
highlighted suggestion or the raw text; `Esc` restores the typed text and closes; `Tab` closes.
Arrow semantics do not flip in RTL — the list is vertical. The `/` global shortcut is **not built**.

## 3. Suggestion list (inside `SearchBox`)

`<ul role="listbox">` with `role="option"` items and `aria-selected`; absolute, `--z-dropdown`,
`max-block-size: 60vh`, opaque `--bg-sunk` surface with a `--line-strong` border (a floating panel
must occlude what it covers). Items select on `mousedown` — `blur` fires before `click` and would
close the list first.

States: hidden (default), open, empty (hidden entirely — never "no suggestions"), loading (the
previous list stays; no spinner).

*Not built*: per-kind icons and prefix highlighting. Items render `item.text` only.

## 4. `Summary` — `components/search/Summary.tsx` (client)

Props: `token`, `note`, `loadingLabel`, `sourcesLabel`, `badge`, `prominent?`.

| State | Rendering |
|:---|:---|
| loading | `.assert.rise` section, `aria-busy`, sparkle + three pulsing dots + `loadingLabel` |
| resolved, empty | **removed entirely**; no error text |
| resolved | badge chip (`badge`, sparkle, accent wash) · text with `[n]` superscript links · sources row of `[n]` pill chips → `#result-<id>` · `note` in `text-xs` |

`prominent` (the query was a question) adds `.summary-answer` and `text-lg`; nothing else changes.
Text is React children — escaped by construction — and only recognised `[n]` markers become
markup. Full behaviour in [[UI - Results Page]] §3.

*Superseded 2026-08-27*: the `reserved` / `generating` / `streaming` states of the SSE design.
There is no stream and no reserved height.

## 5. `ResultCard` — `components/search/ResultCard.tsx` (server)

Props: `result`, `t`, `locale`.

```html
<li id="result-<id>" class="card min-w-0 overflow-hidden scroll-mt-24" dir="auto">
  <div class="text-xs muted">
    <span class="border radius-sm">web</span>       <!-- t[source_type] -->
    <span class="accent">From the web</span>         <!-- only when from_web -->
    <bdi class="truncate">elkhabar.com › economie</bdi>
    <bdi><time datetime="…">4 August 2026</time></bdi>   <!-- or t.dateUnknown -->
    <span><span aria-hidden>▲</span> positive</span>     <!-- only when sentiment != null -->
  </div>
  <h2><a href="…" rel="noopener nofollow" data-doc="<id>">Title with <em>term</em></a></h2>
  <p class="text-sm muted">excerpt with <em>term</em></p>
</li>
```

- `min-w-0 overflow-hidden` are load-bearing: one unbreakable percent-encoded Arabic slug once
  widened the document to 4 487 px.
- `data-doc` is read by `InteractionBeacon`'s delegated listener; the `href` stays the real
  destination.
- Never carries `.assert` — that mark means the engine is asserting something; a result is what
  somebody else published.
- The title is the link, not the whole card, so text is selectable and the URL copyable.

*Not built*: thumbnail, engagement footer, `matched_comments`, `+N similar`, line clamps,
relative-date `title`.

## 6. `Filters` — `components/search/Filters.tsx` (server)

Props: `lang`, `t`, `facets`, `active`, `q`. Three groups (`language`→`lang`, `source_type`→
`source`, `sentiment.label`→`sentiment`), each `role="group" aria-label`, each value a `LinkButton`
(`emphasis` + `aria-current="true"` when selected) with the count in `.numeric`. A group with fewer
than two values and nothing selected is hidden; a "clear" `LinkButton.chip-clear` appears when
anything is selected. Real links — every toggle is a navigation. Full behaviour in
[[UI - Filters and Facets]].

*Superseded 2026-08-27*: `role="switch"` / `aria-checked` chips. These navigate, so they are links.

## 7. `Pagination` — `components/search/Pagination.tsx` (server)

Words (`t.previous` / `t.next`), a five-page window around the current page, current page as
`<span class="chip chip-active numeric" aria-current="page">`, others as `LinkButton`. Hidden
when `total_pages ≤ 1`. Words rather than `‹ ›` because in RTL a literal chevron points the wrong
way and mirroring it with a transform is worse than writing the word.

No infinite scroll: it breaks the back button and "share this page of results", and is an
engagement pattern rather than a utility one ([[UI Specification]] §2).

## 8. `Verticals` — `components/search/Verticals.tsx` (server)

A `<nav>` of links: All, News, Files, Images, Videos; `?v=` in the URL; active tab has
`aria-current="page"` and an accent bottom border. [[UI - Search Verticals]].

## 9. `MediaGrid` — `ImageGrid` / `VideoList` (server)

Tile layouts for the Images and Videos tabs, `<img>` through the signed `/api/thumb` proxy.
[[UI - Search Verticals]] and [[UI - Image Search]].

## 10. `EntityPanel` and `ListPanel` (client)

The knowledge rail and the relation row. Documented in [[UI - Knowledge Panel]]. (`KnowledgePanel.tsx`,
the earlier Wikipedia-only rail, is still in the tree but no longer mounted — `EntityPanel` folds
the Wikipedia extract in.)

## 11. `InteractionBeacon` — `components/search/InteractionBeacon.tsx` (client)

Props: `token`, `children`. A `<div>` with one capture-phase click listener that reads
`a[data-doc]` and `navigator.sendBeacon('/api/v1/interaction', {t, d})`. Absent when the API sent
no token. [[Interaction Signals]].

## 12. `Button` / `LinkButton` — `components/ui/Button.tsx` (server)

`variant: 'default' | 'emphasis' | 'quiet'` → `.chip` / `.chip.chip-active` / `.btn-quiet`.
`type` is **not defaulted** — HTML's default of `submit` is right in the Server Action forms and
wrong elsewhere, so the caller says. `LinkButton` is a separate `next/link` wrapper rather than an
`asChild` polymorph: a link must be a real `<a>`.

## 13. `Select` — `components/ui/Select.tsx` (server)

A **native** `<select>` with a visible label, styled `.chip`. The option list cannot be styled;
that is the price of a control every mobile browser already renders well and that works without
JavaScript.

## 14. `Toggle` — `components/ui/Toggle.tsx` (server)

A submit `Button` with `aria-pressed`, not `role="switch"`: it posts a Server Action and the page
re-renders, which is a button. Used on `/[lang]/settings` for per-tool on/off (cookie
`xustive-tools-off`).

## 15. `Icon` — `components/ui/Icon.tsx` (server)

Hand-picked 24-unit paths, `stroke="currentColor"`, `strokeWidth 1.75`, `aria-hidden`, drawn
inline — an icon package would cost more JS than the whole entity panel. Names: `sparkle user film
tv pin building box book music calendar leaf bulb cake cross flag shirt briefcase star clock globe
tag people ruler link quote camera play check chevron-start chevron-end users`. The chevrons are
logical (start/end) and flip under `dir="rtl"`.

`lucide-react` is used only in client components that already ship JS (`SearchBox`, the header
toggles, the tool cards).

## 16. Header controls — `components/layout/*`

| Component | Behaviour |
|:---|:---|
| `Wordmark` | `XUSTIVE`, always Latin, `dir="ltr"`, links to `/[lang]`; `size: 'lg' \| 'sm'` |
| `LangSwitcher` | `.ghost` button (`aria-haspopup="menu"`, `aria-expanded`) opening a `role="menu"` of four **links** — العربية / الدارجة / Français / English, each named in its own language, `lang`+`dir` per item, `aria-current` on the active one — to the same path and query under the other locale. Only the disclosure needs JS. Switching changes chrome language, `dir`, the `ui=` ranking signal and the summary language ([[UI - RTL and Localization]]) |
| `ThemeToggle` | cycles system → light → dark; writes `data-theme` on `<html>` immediately, then the `xustive-theme` cookie via a Server Action and `router.refresh()`. `aria-label` names the **current** state |
| `DensityToggle` | comfortable ↔ compact, identical shape (`data-density`, `xustive-density` cookie); compact tightens `--result-gap` from 12 px to 6 px. Matters here because Arabic sets taller than Latin |
| `OfflineBanner` | fixed top `role="status"`, assertive while offline, polite 3 s "back online" note on recovery; mounted in the `[lang]` layout |

*Superseded 2026-08-27*: the `LanguageToggle` in `localStorage`. Language is the URL segment;
theme, density and disabled tools are cookies, so the server can render the right thing on the
first byte ([[UI - Frontend Architecture]] §6).

## 17. Tool cards — `components/tools/*`

`ToolCard` (the generic frame), `TranslateCard`, `WeatherDetail`, `ImageOcr`, `CopyButton`,
`DismissTool`. [[UI - Tool Cards]].

## 18. Not built

`Sheet`/`Rail` (filters live inline), `Toast` (nothing transient to show yet; voice errors are an
inline status line), `Skeleton` for the page (results arrive server-rendered; the entity panel has
its own skeleton), `Badge` as a component (badges are spans with `.chip`-like classes), `EmptyState`
as a component (the empty copy is inline in the search page).

---

## 19. Component Checklist

Every component ships with: all states styled, keyboard operation, an accessible name, RTL
verification (`scripts/rtl-icons.sh`, `scripts/lint-bidi.sh`), a `prefers-reduced-motion` variant
(`.rise` is gated on `no-preference`), a dark-mode check (`data-theme` tokens), and a contrast pass
(`scripts/contrast-audit.mjs`). There is no visual-regression suite yet.

## 20. Open Questions

- [ ] Should `ResultCard` show the domain favicon? A third-party request per card — would have to
      go through `/api/thumb` like everything else ([[ADR-0021 - Proxied Thumbnails with Signed URLs]]).
- [x] Citation chips: scroll to the card (`#result-<id>`). (2026-08-27)
- [ ] `+N similar`: expand in place or a new search? (`similar_count` is in the contract, unused)
- [ ] The clear button's `aria-label="Clear"` is the one untranslated string in the search box.
- [ ] Delete `KnowledgePanel.tsx` and `/api/knowledge` now that `EntityPanel` covers them?

## Related

[[UI - Design Language]] · [[UI - Results Page]] · [[UI - Home Page]] · [[UI - Knowledge Panel]] ·
[[UI - Accessibility]] · [[UI - RTL and Localization]] · [[UI - States and Errors]]
