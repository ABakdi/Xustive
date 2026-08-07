---
tags:
  - ui
  - moc
type: map-of-content
status: specified
updated: 2026-08-06
---

# UI Specification

> The hub for everything user-facing. Individual screens and systems have their own notes; this note
> holds the principles, the stack, and the constraints they all inherit.
> Data contract: [[API Contract]] · Parent: [[Xustive Search Engine – Technical Specification]]

---

## 1. Notes in this section

| Note | Covers |
|:---|:---|
| [[UI - Frontend Architecture]] | **framework, routing, rendering strategy, i18n, budgets** |
| [[UI - Design Language]] | **the visual identity: palette, type, shape, the qalam rule** |
| [[UI - Tool Cards]] | **instant-answer cards: one section per tool** |
| [[UI - Design System]] | tokens: colour, type, spacing, elevation, motion (contrast reference) |
| [[UI - Component Library]] | every reusable component, its states and markup |
| [[UI - Home Page]] | the search entry point + autocomplete |
| [[UI - Results Page]] | summary block, result cards, pagination |
| [[UI - Voice Search]] | mic permission, recording, transcript editing |
| [[UI - Image Search]] | upload/camera, OCR vs similarity results |
| [[UI - Filters and Facets]] | date, source, sentiment filtering |
| [[UI - RTL and Localization]] | Arabic RTL, bidi, four UI languages |
| [[UI - Accessibility]] | WCAG 2.2 AA commitments and how they're tested |
| [[UI - States and Errors]] | loading, empty, error, offline, degraded |

---

## 2. Product Principles

1. **The links are the product.** The AI summary is an accelerator, never a replacement. If the
   summary fails, the page is still complete and useful ([[Summarizer]] §7).
2. **Fast on a bad phone.** The target device is a mid-range Android on 3G, not a laptop on fibre.
   Every feature is judged against that.
3. **Arabic is not an afterthought.** RTL is a first-class layout, not a mirrored CSS hack. Darija
   input in Latin script works everywhere Arabic input does.
4. **Nothing is tracked.** No analytics, no cookies, no fingerprinting. The UI is *architecturally*
   incapable of it — there is no third-party script to add one
   ([[Security and Privacy]] P3, P7).
5. **Honest about uncertainty.** Approximate result counts say "about". Unknown dates are not
   rendered as certain. Low-confidence sentiment shows no badge ([[Sentiment Engine]] §4.3).
6. **No dark patterns.** No infinite scroll, no engagement bait, no interstitials. The user comes to
   find something and leave.

---

## 3. Technical Stack

| Concern | Choice | Rationale |
|:---|:---|:---|
| Framework | **Next.js 15, App Router, React Server Components** | results must be HTML in the first response; see [[ADR-0010 - Next.js for the Frontend]] |
| Language | TypeScript, strict | the API contract becomes a compile error rather than a runtime `undefined` |
| Styling | Tailwind CSS v4 | small final CSS, tokens from [[UI - Design Language]] |
| Components | **shadcn/ui, source-copied and rewritten** | accessible primitives we own outright, not a theme over someone else's product |
| State | URL query string is the state | shareable, back-button-correct, no store to desync |
| Summary | client fetch after paint | 20+ s on CPU; nothing may wait on it ([[Summarizer]] §3) |
| Icons | `lucide-react`, tree-shaken | no icon font, no extra request |
| Fonts | Self-hosted IBM Plex Sans + Sans Arabic, subset per script | zero third-party font requests ([[Security and Privacy]] P7) |
| Build | `next build` via `pnpm` | one toolchain rather than three |

**Nothing is loaded from a third-party origin at runtime.** Enforced by the CSP
(`default-src 'self'`) and by a CI check on the built asset manifest. React ships from our own
origin like everything else — the rule is about *origins*, not about having no dependencies.

The earlier no-framework position is superseded by [[ADR-0010 - Next.js for the Frontend]]. It
held while the UI was a search box and a list; it stopped holding when every component had to be
written twice, once in Rust and once in JavaScript, and the language filter shipped broken
because the two drifted.

---

## 4. Performance Budget (client)

| Metric | Budget | Why |
|:---|:---|:---|
| HTML (home, gzipped) | ≤ 12 KB | first paint on 3G |
| CSS (gzipped) | ≤ 18 KB | |
| JS (gzipped) | ≤ 25 KB | |
| Total first load | ≤ 60 KB | |
| LCP on mid-range Android / 3G | ≤ 2.0 s | |
| INP | ≤ 200 ms | |
| CLS | ≤ 0.05 | summary streaming must not shift the results below it |
| Requests on first load | ≤ 6 | |

CLS is the subtle one: the summary block streams in *above* the results, so it must reserve its
height before the first token arrives ([[UI - Results Page]] §3).

Budgets are enforced in CI (`bundlesize` + Lighthouse CI on a throttled profile) and mirror
[[Performance Budgets]].

---

## 5. Layout and Breakpoints

| Name | Width | Layout |
|:---|:---|:---|
| `sm` | < 640 px | single column, filters in a bottom sheet, 16 px gutters |
| `md` | 640–1023 px | single column, filters in a collapsible bar |
| `lg` | ≥ 1024 px | results column (max 680 px) + sticky filter rail |

Mobile-first, always. The desktop layout is the variant, not the baseline.

Container: results content is capped at **680 px** measure regardless of viewport — long lines are
harder to scan, and this matters more in Arabic where letterforms connect.

---

## 6. Page Inventory

| Route | Note | Notes |
|:---|:---|:---|
| `/` | [[UI - Home Page]] | search box, language toggle, privacy line |
| `/search?q=…` | [[UI - Results Page]] | the main surface |
| `/image?…` | [[UI - Image Search]] | image results |
| `/about`, `/privacy`, `/bot` | static | `/bot` documents the crawler ([[Politeness and Robots]] §4.5) |
| `/submit` | source submission form | gated until [[Milestone 5 - Beta Launch]] |
| error pages | [[UI - States and Errors]] | 404, 500, 503 |

---

## 7. URL as State

```
/search?q=سونلغاز&page=2&source=web,facebook&sentiment=negative&from=1754352000&sort=recency&lang=ar
```

Rules:
- Every filter change does `history.replaceState`; every search does `pushState`.
- Back/forward re-render from the URL — no re-fetch if the response is still in the in-memory
  session cache (cleared on unload; nothing persisted).
- The URL is shareable and contains no identifier of any kind.

---

## 8. Progressive Enhancement

| Without JS | Behaviour |
|:---|:---|
| Search | works — the form does a normal `GET /search` and the server renders results |
| Autocomplete | absent |
| AI summary | absent (SSE requires JS) |
| Voice / image | buttons hidden |
| Filters | render as links that reload the page |

A user on a locked-down browser, a text browser, or a flaky connection still gets working search.
This costs a server-rendered template we need anyway for first paint.

---

## 9. Privacy Surface in the UI

- No cookies. No `localStorage` beyond a UI language preference (a non-identifying enum) — and that
  is disclosed in [[UI - Home Page]].
- No third-party requests. Remote thumbnails carry `referrerpolicy="no-referrer"`, pending the
  decision on proxying them ([[Security and Privacy]] §9).
- A one-line, permanently visible statement on the home page: *"We don't store your searches."*
  Every word of that has to stay true — it is a claim the architecture must keep earning
  ([[Security and Privacy]] §1).

---

## 10. Open Questions

- [ ] Dark mode at launch, or after? (Leaning: at launch — it is cheap with tokens and expected.)
- [ ] Do we ship a self-hosted Arabic webfont (~80 KB) or rely on system Arabic faces, which vary
      wildly in quality across Android versions?
- [ ] Should the summary be collapsed by default on `sm` so links are above the fold?

## Related

[[API Contract]] · [[Performance Budgets]] · [[Security and Privacy]] · [[UI - Design System]] ·
[[UI - Results Page]] · [[TODO]]
