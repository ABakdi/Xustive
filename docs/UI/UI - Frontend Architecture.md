---
tags:
  - ui
  - architecture
status: specified
updated: 2026-08-07
---

# UI — Frontend Architecture

> Framework choice and its reasoning live in [[ADR-0010 - Next.js for the Frontend]].

## 1. Shape

```
web/
  app/
    [lang]/                       # ar | ary | fr | en — the only routing segment
      layout.tsx                  # theme + direction resolved server-side
      page.tsx                    # home
      search/page.tsx             # results — a Server Component
      settings/page.tsx
    api/                          # nothing. The Rust API is the API.
  components/
    ui/                           # shadcn primitives, rewritten per UI - Design Language
    search/                       # SearchBox, SuggestionList, ResultCard, Filters, Pagination
    tools/                        # one component per instant answer
    layout/
  lib/
    api.ts                        # typed client, generated from the OpenAPI contract
    i18n/                         # message catalogues
```

**No `app/api/` routes.** The temptation is to proxy the Rust API "just to add a little logic",
and the little logic becomes a second backend nobody specified. The frontend calls
`xustive-api` directly, server-side.

## 2. Rendering

| Route | Strategy | Why |
|:---|:---|:---|
| `/[lang]` | Static | The home page has no per-request state |
| `/[lang]/search` | **Server Component, dynamic** | Results must be HTML in the first response |
| Suggestions | Client | Per-keystroke; cannot be server-rendered |
| Filters | Server-rendered links, client-enhanced | Must work without JavaScript |
| Tool cards | Server where synchronous, client where interactive | See below |
| Summary | Client, after paint | 20+ seconds; nothing may wait on it ([[Summarizer]]) |

Tool cards split by nature: a calculator result is computed by the API and arrives with the
search response, so it is server-rendered. A unit converter with two dropdowns is a client
component that hydrates around the server-rendered initial answer — so the first answer is
visible before any JavaScript runs, and only *changing* it needs hydration.

## 3. The no-JavaScript path

Not a fallback — the same markup, minus enhancement.

- Search is a real `<form method="get">`.
- Filters are `<a>` elements with real hrefs.
- Pagination is links.
- Suggestions, the summary, and interactive tool controls are absent. Everything else works.

**Tested, not assumed**: CI runs the results page with JavaScript disabled and asserts a query
returns results, a filter narrows them, and page 2 loads.

## 4. Data fetching

One typed client generated from the API's OpenAPI description, so a contract change is a
TypeScript error rather than a runtime `undefined`. The duplication that made the language filter
ship broken is exactly what a generated client prevents.

- Server Components call `xustive-api` over the internal network.
- Client components call it through the browser, same origin, via the Next rewrite.
- **No client-side data library.** No SWR, no React Query. Search state lives in the URL, which is
  the correct store for it: shareable, bookmarkable, back-button-correct.

## 5. Internationalisation

Four locales — `ar`, `ary`, `fr`, `en` — as the first path segment. `/` redirects using
`Accept-Language`, then a cookie once chosen.

- Direction from locale: `ar`/`ary` → RTL, `fr`/`en` → LTR. Set on `<html>` server-side.
- **The interface language is independent of the query language.** Someone reading a French
  interface searching in Darija is the normal case, not an edge one.
- Message catalogues are flat JSON per locale, keyed by dot path. **A missing key is a build
  error**, not a fallback — a French page with English strings scattered through it looks broken
  in a way nobody reports.
- `Intl.PluralRules` for counts. Arabic has six plural forms; `n === 1 ? x : y` is wrong in a way
  that reads as illiteracy to a native speaker.
- `Intl.NumberFormat` and `Intl.DateTimeFormat` per locale, with the numeral system an explicit
  choice rather than a locale default ([[UI - Design Language]] §10).
- Darija catalogues fall back to Arabic, never to English.

## 6. Theming

Light / dark / system, resolved **server-side from a cookie** and stamped onto `<html>` before
the first byte. No flash of the wrong theme — the most common dark-mode defect and the one that
reads as a bug every time.

A tiny inline script reconciles `system` against `prefers-color-scheme` before paint. It is the
only inline script on the page and needs a CSP hash rather than `unsafe-inline`
([[Security and Privacy]]).

## 7. Performance budgets

Tighter than the current page, because a framework is only worth its cost if it stays fast.

| Metric | Budget |
|:---|:---|
| JS, home route | ≤ 185 KB gzipped — **~152 KB of that is the React and Next runtime**, before any of our code. The pre-React budget was 40 KB; see [[ADR-0010 - Next.js for the Frontend]] |
| JS, search route | ≤ 195 KB gzipped, same caveat |
| CSS | ≤ 20 KB gzipped |
| LCP, throttled 3G | ≤ 2.5 s |
| CLS | ≤ 0.05 |
| Hydration cost, mid-range phone | ≤ 150 ms |

Enforced in CI. **Exceeding a budget fails the build** — a budget that only warns is a number
nobody reads.

Fonts are self-hosted, subset per script, `font-display: swap`, preloaded for the active
direction only. Shipping Arabic glyphs to a French reader is most of a font budget wasted.

## 8. Deployment

A Node process beside `xustive-api`, both behind the reverse proxy in
[[Deployment Topology]]. Health at `/healthz`; **it must not proxy the API's health checks** —
a liveness probe that fails because a dependency is unwell gets the wrong process restarted.

`make up` gains a build step. Development runs `next dev` against the local API.

## 9. Migration

The existing `web.rs` renderer is **deleted** when the corresponding route lands, not kept in
parallel. Two renderers is the problem being solved.

Order: home → search results → filters → suggestions → summary → tool cards. Search results are
the risky one and go early, while there is appetite to fix what it breaks.

## 10. Open questions

- [ ] Self-host fonts or use a system stack for the Latin? Self-hosting costs ~35 KB per script
      and buys typographic consistency. Currently specified as self-hosted; worth revisiting
      against the 3G budget.
- [ ] Does `ary` need its own catalogue at launch, or does it start as an alias for `ar`? Writing
      Darija UI copy well is harder than translating it ← *B7*
- [ ] View Transitions for the search → results navigation: genuinely nice, or motion that gets
      in the way of reading?

## Related

[[ADR-0010 - Next.js for the Frontend]] · [[UI - Design Language]] · [[UI - Component Library]] ·
[[UI - RTL and Localization]] · [[UI - Accessibility]] · [[Instant Answers]] ·
[[Performance Budgets]] · [[Deployment Topology]] · [[API Contract]]
