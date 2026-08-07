---
tags:
  - ui
  - design
status: specified
updated: 2026-08-07
---

# UI — Design Language

> Replaces the former `UI - Design System` note, which is deleted rather than kept alongside —
> two token lists is how the values drift. Contrast obligations live in [[UI - Accessibility]].

## 1. The problem with looking like everything else

shadcn/ui out of the box is a known quantity: neutral greys, `oklch` slate, 0.5 rem radii, Inter.
It is excellent and it is *everywhere*. An Algerian search engine that looks like a Vercel
template says nothing about what it is.

The primitives are still the right foundation — accessible, unstyled-by-default, source-copied so
they can be rewritten rather than fought. What follows replaces the skin entirely.

## 2. The idea: paper and ink, under a Saharan sky

Two references, both Algerian, neither literal:

**Maghrebi manuscript pages.** Warm off-white paper, dense black ink, and a single saturated
accent used only for what matters — a chapter mark, a correction. Not decorative. The restraint
is the point: a page of text with one red mark tells you where to look.

**The blue hour over the Hauts Plateaux.** Deep indigo that is not black, with warm light at the
horizon. This is the dark theme, and it is why dark mode here is not the light theme inverted.

What this rules out: glassmorphism, gradient meshes, neon accents, drop shadows on everything.
A search engine is a reading surface. The design's job is to disappear.

## 3. Colour

Perceptual (`oklch`) so that a hue shift does not change apparent brightness — which matters when
the same accent sits on parchment and on indigo.

```css
/* Light — "manuscript" */
--paper:      oklch(98.4% 0.006  85);   /* warm off-white, not #fff */
--paper-sunk: oklch(96.2% 0.010  85);   /* cards, wells */
--ink:        oklch(21%   0.018  75);   /* warm near-black, never #000 */
--ink-muted:  oklch(48%   0.014  75);
--rule:       oklch(89%   0.010  85);   /* hairlines */

/* Dark — "blue hour" */
--night:      oklch(19%   0.028 255);   /* deep indigo, not grey */
--night-lift: oklch(24%   0.030 255);
--dusk:       oklch(94%   0.010  85);   /* text: warm, so it reads as lit not glowing */
--dusk-muted: oklch(68%   0.016 255);
--rule-night: oklch(32%   0.028 255);

/* Accent — one, in both themes */
--accent:      oklch(56% 0.148 158);    /* deep Algerian green */
--accent-lift: oklch(64% 0.152 158);    /* dark-theme variant, lifted to hold contrast */
--accent-wash: oklch(94% 0.036 158);

/* Semantics — never carried by colour alone */
--warn:   oklch(62% 0.130  70);
--danger: oklch(56% 0.170  27);
```

**Rules.**

1. **One accent.** Green marks the interactive and the current — links, focus, the active filter,
   the tool-card rule. Nothing else is coloured.
2. **No pure black or white.** `#000` on `#fff` is the highest-strain combination there is, and
   these pages are read for minutes at a time.
3. **Colour is never the only signal.** Every state also carries a glyph, a label, or a border
   ([[UI - Accessibility]]).
4. **Dark is not inverted light.** Different hue family, different accent lightness.

## 4. Typography

The hard requirement: **Arabic, Darija in Arabic script, Arabizi in Latin, and French — often in
the same line.** Most pairings fail this. A Latin face chosen for elegance next to a default
Arabic fallback looks like a bug.

| Role | Face | Why |
|:---|:---|:---|
| Arabic | **IBM Plex Sans Arabic** | Drawn alongside the Latin, so mixed lines share weight and rhythm. Open licence, full Arabic coverage, readable at 14 px. |
| Latin | **IBM Plex Sans** | Same family. Mixed-script lines stop looking accidental. |
| Numerals | **IBM Plex Sans**, `font-variant-numeric: tabular-nums` | Counts, prices and rates must align in columns. |
| Code / tools | **IBM Plex Mono** | Calculator input, converters, dev tools. |

```css
--text-xs:   0.75rem;   --text-sm: 0.875rem;  --text-base: 1rem;
--text-lg:   1.125rem;  --text-xl: 1.375rem;  --text-2xl:  1.75rem;
--leading-tight: 1.3;   --leading-body: 1.7;  /* Arabic needs more than Latin */
```

**Arabic runs 1.7 line-height and one step larger** than the Latin at the same nominal size.
Arabic glyphs carry more vertical detail; typeset at Latin metrics they are cramped and slow to
read. This is set per-element from the content language, not per-page.

## 5. Shape, space, motion

```css
--radius-sm: 4px;   --radius-md: 8px;   --radius-pill: 999px;
--space: 4px;  /* everything is a multiple */
```

Radii are **small**. Large radii read as soft and app-like; this is a document surface.

**Elevation is a hairline, not a shadow.** Cards are separated by `--rule`, not by blur. Exactly
two things float — the suggestion panel and any modal — and they get one restrained shadow.

**Motion budget: 120 ms, ease-out, opacity and 4 px translate only.** No spring, no scale, no
stagger. Everything inside `@media (prefers-reduced-motion: no-preference)`. A result list that
animates in is a result list you cannot read for 300 ms.

## 6. The signature: the qalam rule

One distinctive element, used sparingly, so the product is recognisable without being decorated.

A **2 px accent rule on the inline-start edge** of anything the engine is asserting rather than
merely listing: the AI summary, and every tool card. It mirrors automatically in RTL because it
is `border-inline-start`.

It carries meaning, which is the only reason it earns its place: **a green edge means Xustive is
telling you something, not showing you what someone else published.** Result cards never have it.

## 7. Focus, layering, icons

Carried over from the note this replaces, unchanged because they were right.

```css
:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
  border-radius: var(--radius-sm);
}
```

Focus is **never removed, only restyled**. `:focus-visible` rather than `:focus`, so mouse users
see no rings and every keyboard user always does. Hit targets ≥ 44 px.

| Token | Value | Layer |
|:---|:---|:---|
| `--z-base` | 0 | content |
| `--z-sticky` | 10 | sticky search bar |
| `--z-dropdown` | 20 | autocomplete |
| `--z-sheet` | 30 | mobile filter sheet |
| `--z-modal` | 40 | dialogs |
| `--z-toast` | 50 | transient messages |

**Any `z-index` not from this table is a bug.** The suggestion panel losing to the sticky header
is what happens without a scale, and it happened.

Icons: `lucide-react`, 24 × 24, 1.5 px stroke, `currentColor`. Decorative by default
(`aria-hidden`); an icon that is a control's only content gets an `aria-label`
([[UI - Accessibility]] §3). Weather icons are drawn to the same stroke weight
([[UI - Tool Cards]] §3).

---

## 8. Density

Two modes, remembered per device.

| | Comfortable (default) | Compact |
|:---|:---|:---|
| Result spacing | 24 px | 14 px |
| Results above the fold, 1080p | ~5 | ~8 |

Comfortable by default because the first-run impression should be readable, not dense.

## 9. Themes

Three settings — light, dark, **system** (default). Resolved server-side from a cookie so there
is no flash of the wrong theme, which is the single most common failure of a dark-mode
implementation and looks like a bug every time.

```
:root[data-theme="light"] { … }
:root[data-theme="dark"]  { … }
```

The toggle cycles system → light → dark and announces the resulting state to assistive tech.

## 10. What this is not

- Not a dashboard. No cards-in-a-grid, no stat tiles.
- Not brutalist. Warmth and hairlines, not raw borders and system fonts.
- Not "AI-styled". No purple gradients, no sparkle icons, no glow. The summary is marked by the
  same green rule as a unit conversion, because both are the engine speaking.

## 11. Open questions

- [ ] Should a Maghrebi geometric motif appear at all — in the empty state, the 404, the footer?
      Risk: decoration that dates fast and reads as costume. Leaning towards **one** use, in the
      zero-results illustration only.
- [ ] Is IBM Plex Sans Arabic the right face, or is Cairo more familiar to Algerian readers?
      Needs a native-speaker read ← *B7*
- [ ] Tabular numerals in Arabic contexts: Eastern Arabic numerals (٤٥) or Western (45)? Algerian
      usage is mixed and leans Western in print. Needs a decision, not a guess ← *B7*

## Related

[[UI - Accessibility]] · [[UI - RTL and Localization]] ·
[[UI - Component Library]] · [[Instant Answers]] · [[ADR-0010 - Next.js for the Frontend]]
