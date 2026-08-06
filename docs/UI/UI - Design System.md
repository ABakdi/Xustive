---
tags:
  - ui
  - design
type: ui
status: specified
updated: 2026-08-06
---

# UI - Design System

> Tokens only. Components that consume them live in [[UI - Component Library]].
> Parent: [[UI Specification]]

---

## 1. Token Philosophy

Every value in the UI comes from a token. No component defines a raw hex colour, a raw pixel size, or
a raw duration. Tokens are CSS custom properties on `:root`, mapped into Tailwind's theme so both
utility classes and hand-written CSS read the same source.

Tokens are **semantic, not literal**: components use `--color-text-secondary`, never `--gray-500`.
This is what makes dark mode and future theming a token swap rather than a rewrite.

---

## 2. Colour

### Palette (literal, internal)

| Token | Light | Dark |
|:---|:---|:---|
| `--green-600` | `#0F7A4F` | `#2FA97A` |
| `--green-50` | `#E8F5EE` | `#0E2A20` |
| `--sand-500` | `#C9A227` | `#D9B84A` |
| `--slate-900` | `#0E1418` | `#F2F5F7` |
| `--slate-700` | `#33424C` | `#C3CDD4` |
| `--slate-500` | `#63757F` | `#8E9BA3` |
| `--slate-200` | `#DDE4E8` | `#2A3439` |
| `--slate-50` | `#F6F8F9` | `#151C21` |
| `--white` | `#FFFFFF` | `#0B1115` |
| `--red-600` | `#B3261E` | `#F2837C` |
| `--amber-600` | `#8A6116` | `#E0B15C` |
| `--blue-600` | `#1B5FA8` | `#7FB2E8` |

Green and sand are drawn from the Algerian flag without literally reproducing it — a search engine
should not look like a government portal.

### Semantic tokens (what components use)

| Token | Maps to | Use |
|:---|:---|:---|
| `--color-bg` | `--white` | page background |
| `--color-surface` | `--slate-50` | cards, sheets |
| `--color-border` | `--slate-200` | dividers, inputs |
| `--color-text` | `--slate-900` | body text |
| `--color-text-secondary` | `--slate-700` | excerpts, metadata |
| `--color-text-muted` | `--slate-500` | timestamps, counts |
| `--color-accent` | `--green-600` | links, focus, primary action |
| `--color-accent-weak` | `--green-50` | selected chips, highlight background |
| `--color-highlight-bg` | `#FFF3C4` / `#4A3B12` | search-term `<em>` background |
| `--color-danger` | `--red-600` | errors, negative sentiment |
| `--color-warning` | `--amber-600` | degraded states, neutral-negative |
| `--color-info` | `--blue-600` | informational banners |

### Sentiment colours

| Sentiment | Token | Never conveyed by colour alone |
|:---|:---|:---|
| positive | `--green-600` | ▲ icon + text label |
| neutral | `--slate-500` | ● icon + text label |
| negative | `--red-600` | ▼ icon + text label |

Colour-blind users and greyscale printouts must still read sentiment — see
[[UI - Accessibility]] §4.

### Contrast

All text/background pairs meet **WCAG AA (4.5:1 body, 3:1 large)**. `--color-text-muted` on
`--color-surface` is the tightest pair at 4.6:1 and is tested in CI, because it is the one that
breaks first when someone "lightens the timestamps a bit".

### Dark mode

`@media (prefers-color-scheme: dark)` plus a `:root[data-theme]` override for the manual toggle. Both
directions must work — the toggle wins over the media query.

---

## 3. Typography

### Families

```css
--font-latin:  ui-sans-serif, system-ui, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
--font-arabic: "Noto Naskh Arabic", "Segoe UI", Tahoma, "Arabic Typesetting", serif;
--font-mono:   ui-monospace, "Cascadia Code", Menlo, Consolas, monospace;
```

Arabic text gets `--font-arabic` via `:lang(ar)` — system Arabic faces vary enormously across Android
versions, and Naskh reads better than the default sans at small sizes. Whether we self-host it is an
open question in [[UI Specification]] §10.

### Scale

| Token | Size / line-height | Use |
|:---|:---|:---|
| `--text-xs` | 12 / 16 | timestamps, badges |
| `--text-sm` | 14 / 20 | metadata, chips |
| `--text-base` | 16 / 26 | body, excerpts |
| `--text-lg` | 18 / 28 | summary text |
| `--text-xl` | 20 / 28 | result titles |
| `--text-2xl` | 24 / 32 | page headings |
| `--text-3xl` | 32 / 40 | home wordmark |

**Arabic runs larger.** Arabic glyphs at the same nominal size read smaller and denser than Latin, so
`:lang(ar)` applies `font-size: 1.08em` and `line-height: 1.75` — otherwise Arabic result cards look
cramped next to French ones on the same page.

Weights: 400 body, 500 emphasis/metadata, 600 titles/headings. No 700 — with system fonts it renders
inconsistently.

---

## 4. Spacing

4 px base scale: `--space-1: 4px` … `--space-2: 8`, `--space-3: 12`, `--space-4: 16`,
`--space-5: 20`, `--space-6: 24`, `--space-8: 32`, `--space-12: 48`, `--space-16: 64`.

| Context | Value |
|:---|:---|
| Gutter (`sm`) | `--space-4` |
| Gutter (`lg`) | `--space-8` |
| Result card vertical rhythm | `--space-6` between cards, `--space-2` within |
| Section separation | `--space-12` |
| Touch target minimum | **44 × 44 px** ([[UI - Accessibility]]) |

Spacing tokens are **logical**: `padding-inline`, `margin-inline-start`, never `padding-left`. This
is what makes RTL work without a mirrored stylesheet ([[UI - RTL and Localization]] §3).

---

## 5. Radius, Border, Elevation

| Token | Value | Use |
|:---|:---|:---|
| `--radius-sm` | 6 px | chips, badges |
| `--radius-md` | 10 px | cards, inputs |
| `--radius-lg` | 16 px | sheets, modals |
| `--radius-full` | 9999 px | search box, pills |
| `--border-width` | 1 px | |
| `--shadow-sm` | `0 1px 2px rgb(0 0 0 / 0.06)` | resting card |
| `--shadow-md` | `0 4px 12px rgb(0 0 0 / 0.08)` | focused search box, dropdown |
| `--shadow-lg` | `0 12px 32px rgb(0 0 0 / 0.12)` | bottom sheet |

Elevation is used sparingly: this is a text-dense product, and shadows on every card produce visual
noise that slows scanning. Cards are separated by **space and a hairline border**, not shadow.

---

## 6. Motion

| Token | Value | Use |
|:---|:---|:---|
| `--ease-out` | `cubic-bezier(0.2, 0, 0, 1)` | entrances |
| `--ease-in-out` | `cubic-bezier(0.4, 0, 0.2, 1)` | movement |
| `--duration-fast` | 120 ms | hover, focus, chip toggle |
| `--duration-base` | 200 ms | dropdown, sheet |
| `--duration-slow` | 320 ms | page transitions |

Rules:
- Nothing animates longer than 320 ms.
- **Streaming summary text does not animate per token** — it renders as it arrives. Fading each token
  in is a fidget that costs INP and makes text harder to read.
- `@media (prefers-reduced-motion: reduce)` sets all durations to `1ms` and disables the shimmer in
  loading states, replacing it with a static placeholder ([[UI - States and Errors]] §2).

---

## 7. Focus and Interaction

```css
:focus-visible {
  outline: 2px solid var(--color-accent);
  outline-offset: 2px;
  border-radius: var(--radius-sm);
}
```

Focus is **never** removed, only restyled. `:focus-visible` (not `:focus`) so mouse users do not see
rings, but every keyboard user always does. Hit targets ≥ 44 px; interactive elements have a
`:hover`, `:focus-visible`, `:active`, and `:disabled` state defined in
[[UI - Component Library]].

---

## 8. Z-Index Scale

| Token | Value | Layer |
|:---|:---|:---|
| `--z-base` | 0 | content |
| `--z-sticky` | 10 | sticky search bar, filter rail |
| `--z-dropdown` | 20 | autocomplete |
| `--z-sheet` | 30 | mobile filter sheet |
| `--z-modal` | 40 | image upload dialog |
| `--z-toast` | 50 | transient messages |

Any `z-index` not from this table is a bug.

---

## 9. Iconography

Inline SVG sprite (`<use href="#icon-…">`), 24 × 24 grid, 1.5 px stroke, `currentColor` fill. Set:
search, mic, camera, image, filter, calendar, close, chevron, external-link, globe, check, alert,
sentiment-up, sentiment-neutral, sentiment-down, platform badges (web/facebook/instagram/tiktok).

Icons are decorative by default (`aria-hidden="true"`); when an icon is the only content of a control
the control gets an `aria-label` ([[UI - Accessibility]] §3).

---

## 10. Open Questions

- [ ] Is green-as-accent too close to a "government site" read for Algerian users? Worth testing with
      5 people before it becomes expensive to change.
- [ ] Self-host Noto Naskh Arabic (~80 KB subset) or accept inconsistent system rendering?
- [ ] Do we need a distinct visual treatment per source platform beyond the badge colour?

## Related

[[UI - Component Library]] · [[UI - RTL and Localization]] · [[UI - Accessibility]] ·
[[UI Specification]] · [[UI - States and Errors]]
