---
tags:
  - ui
type: ui
status: built
updated: 2026-08-27
---

# UI - Home Page

> Route `/[lang]` (`web/app/[lang]/page.tsx`); the bare `/` negotiates a locale from
> `Accept-Language` and redirects (`web/app/page.tsx`). The entry point: one search box and nothing
> competing with it.
> Parent: [[UI Specification]] · Components: [[UI - Component Library]]

---

## 1. Purpose

Get the user into a query as fast as possible, and communicate two things in passing: this searches
Algerian content, and it does not record what you search.

## 2. Layout

As built (2026-08-27). The original mock had a footer row of links and a lock icon on the privacy
line; neither shipped — the page is shorter than planned.

```
┌──────────────────────────────────────────────────────┐
│                     [ 🌐 عربية ▾ ] [ ☀ ] [ ≡ ]        │  ← LangSwitcher · ThemeToggle · DensityToggle, inline-end
│                                                      │
│   X U S T I V E                                      │  ← Wordmark (lg), always Latin, dir="ltr"
│   محرك البحث الجزائري                                  │  ← tagline, text-sm, --fg-muted
│                                                      │
│   ┌────────────────────────────────────────────┐     │
│   │ 🔍  ابحث…                    ✕   🎤  📷    │     │  ← SearchBox (.field pill, 48px min)
│   └────────────────────────────────────────────┘     │
│   ┌────────────────────────────────────────────┐     │
│   │ suggestions…                               │     │  ← combobox listbox (conditional)
│   └────────────────────────────────────────────┘     │
│                                                      │
│   بحثك ما يترابط بيك أبداً                             │  ← privacy line → /[lang]/privacy, text-xs
└──────────────────────────────────────────────────────┘
```

The block is start-aligned inside `max-w-xl`, not centred. Vertical position: `<main>` is a flex
column with `justify-center` and `min-block-size: 76dvh`, which puts the box a little above the
mathematical centre — optical centre reads better, and on mobile it keeps the box clear of the
keyboard. (The original "38 % on `lg`, 28 % on `sm`" was the intent; one rule does it now.)

The top-right controls are three: [[UI - RTL and Localization|LangSwitcher]], `ThemeToggle`
(system / light / dark, `data-theme` on `<html>`) and `DensityToggle` (comfortable / compact,
`data-density`). Theme and density are cookies set by Server Actions (`web/lib/prefs.ts`), read
before the first byte so there is no flash.

## 3. Behaviour

| Event | Behaviour |
|:---|:---|
| Page load | **no autofocus** on any breakpoint — the original spec focused on `lg`; today the input is not focused until tapped |
| Typing | after 2 characters, debounce **90 ms** (`DEBOUNCE_MS`) → `GET /api/v1/suggest?q=…&limit=8` → combobox list |
| `Enter` | `router.push('/[lang]/search?q=…')`; without JS the `<form method="get" action="/[lang]/search">` submits the same URL |
| Empty submit | no-op (the original 120 ms shake was not built) |
| ✕ (clear) | our own button, shown only when the box has text; empties it and refocuses. The browser's native `type="search"` cancel is hidden by CSS (`::-webkit-search-cancel-button { display: none }`) so there is one clear button, not two |
| Mic button | → [[UI - Voice Search]] (inline; renders only where `getUserMedia` + `MediaRecorder` exist) |
| Camera | a real `<Link>` to `/[lang]/tools/ocr`, `aria-label`/`title` = `t.ocrByImage` → [[UI - Image Search]] |
| `/` key | not built |
| Arabic text | box flips via `dir="auto"` on the input; suggestion items are `dir="auto"` too |
| `↓`/`↑` | walk the list; past the end returns to what was typed (`typedRef`). Arrow semantics do not mirror in RTL — the list is vertical |
| `Esc` | restores the typed text and closes the list |

Suggestion requests are aborted (`AbortController`) when a newer keystroke arrives — otherwise a slow
response overwrites a newer one, which reads as the list "jumping backwards". Suggest failures
(including aborts) render as an empty list, never an error: the user is mid-keystroke.

Nothing about a prefix is stored client-side. No local history.

## 4. Language Toggle

Four options — `ar`, `ary` (Darija), `fr`, `en` — each named in its own language. The locale is
**the first path segment**, so each option is a real `<a>` to the same route under another locale
(query string preserved) and works without JavaScript; only the disclosure needs script.
`<html lang dir>` are set server-side from the segment (`dirOf`), never detected client-side.

It is **a UI language choice, not a content filter** — a French UI still returns Arabic results.
The results page does send it to the API as `ui=<lang>`, which the ranker uses as a soft signal
and the summariser uses as its output language ([[UI - Results Page]]).

Default for the bare `/`: `negotiate(Accept-Language)` → `ary` before `ar` (a naive prefix match
would swallow Darija), then `fr`, `en`, else `ar`. Nothing is persisted in `localStorage`; the
URL is the preference. (The original `xustive.lang` key was never built.)

## 5. The Privacy Line

> **بحثك ما يترابط بيك أبداً** · *Vos recherches ne sont jamais liées à vous* · *Searches are never
> linked to you* · Darija: *البحث تاعك عمرو ما يتربط بيك* (`t.privacyLine`)

Always visible, never dismissable, not a modal. The line itself is the link to `/[lang]/privacy`,
which explains *how* the claim is structurally enforced rather than merely promised
([[Security and Privacy]] §1). Since BUG-030 the same line is also the results page footer, so a
shared results URL is not a dead end.

This is the product's central claim. If any change to the system makes it untrue, this line has to
change with it — that dependency is deliberate and should be uncomfortable to break.

## 6. Performance

The home page is the strictest budget in the product ([[UI Specification]] §4):

| Asset | Budget |
|:---|:---|
| HTML | ≤ 12 KB gzipped |
| CSS (critical, inlined) | ≤ 8 KB |
| JS (deferred) | ≤ 25 KB |
| Requests | ≤ 4 |
| LCP (mid-range Android, 3G) | ≤ 1.5 s |

The wordmark is text (`Wordmark`, letter-spaced), not SVG. One font file is `<link rel="preload">`ed
per direction (Arabic or Latin, never both). The form works without JavaScript; the client
islands are `SearchBox`, `LangSwitcher`, the two toggles and `OfflineBanner`. The budgets above are
targets, not measured — not verified against a build on 2026-08-27.

## 7. States

| State | Rendering |
|:---|:---|
| Default | as §2 |
| Typing, suggestions loading | previous suggestions remain; no spinner |
| Suggestions empty | list hidden entirely |
| Suggest endpoint failing | silent; the box keeps working ([[Autocomplete Service]] §7) |
| Offline | `OfflineBanner` (fixed, top, `role="status"`, assertive): `t.offline` + `t.offlineHint`; a 3 s `t.backOnline` note on recovery. The form still submits and fails visibly |
| Search unavailable | no `/readyz` banner on the home page — not built; the failure shows on the results page ([[UI - States and Errors]]) |
| Voice error | red `role="status"` line under the field ([[UI - Voice Search]]) |

## 8. Accessibility

- The search form is a landmark (`role="search"`). No skip-link exists, so it is the first
  meaningful stop after the three header controls.
- Combobox pattern per WAI-ARIA: `role="combobox"`, `aria-expanded`, `aria-controls`,
  `aria-autocomplete="list"`, `aria-activedescendant`, `role="listbox"`/`option` with
  `aria-selected`.
- The input has `aria-label` = `t.searchLabel`; the icons are `aria-hidden`.
- Suggestion count is **not** announced (no live region for it) — not built.
- The wordmark is a link home; the tagline is a `<p>`, not a heading.
- The clear button's `aria-label` is the hard-coded English "Clear" — not localised. Gap.
- Full commitments in [[UI - Accessibility]].

## 9. Not on This Page

Deliberately absent: trending searches (a query-log surface we do not want —
[[Autocomplete Service]] §12), news feed, ads, login, cookie banner (the only cookies are theme,
density and disabled-tools preferences, none of them tracking), app install prompt, newsletter
modal. Also absent, though the original mock had them: footer links (about / privacy / bot / submit
a source) — the privacy link is the line itself.

The home page has one job.

## 10. Open Questions

- [ ] Does the tagline work better in Darija than MSA? (Darija now has its own: "محرك البحث تاع
      الجزائر".) Worth testing with actual users.
- [ ] Should `/submit` be linked from the footer before [[Milestone 5 - Beta Launch]]?
- [ ] Is a "what is Xustive?" one-liner needed for first-time visitors, or does it add noise?
- [ ] Localise the clear button's label.

## Related

[[UI - Component Library]] · [[UI - Voice Search]] · [[UI - Image Search]] · [[UI - Results Page]] ·
[[Autocomplete Service]] · [[Security and Privacy]] · [[UI - Accessibility]]
