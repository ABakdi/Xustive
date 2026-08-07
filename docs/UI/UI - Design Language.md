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

## 2. The idea: editorial, structural, square

A first attempt used warm manuscript paper and blue-hour indigo. It was rejected as insufficiently
polished, and the criticism was right: warmth reads as nostalgia, and a product that indexes the
live web should not look like it is about the past.

What replaced it is **editorial**. Near-neutral surfaces with a faint cool cast, one accent, and
geometry that is essentially square.

**Rounded corners are the tell.** A pill-shaped search field is the single most recognisable
"generic web app" signal there is. The only radius in this system is **2 px**, and it exists so
1 px borders do not fray at the corner — not to make anything look friendly. The search field is a
rectangle. Chips are rectangles. Result cards have no border at all.

**Structure comes from hairlines and space**, never from shadows or fills. If two things need
separating, a 1 px rule or 30 px of space does it.

What this rules out: glassmorphism, gradient meshes, neon accents, drop shadows, pills, cards with
elevation, and anything that animates for longer than 100 ms. A search engine is a reading
surface. The design's job is to disappear.

## 3. Colour

Perceptual (`oklch`) so a hue shift does not change apparent brightness — which matters when the
same accent has to hold against near-black text and against a dark ground.

```css
/* Light — off-white with a faint cool cast. Flat white glares under long reading; warm cream
   reads as nostalgia. This should look like a well-set page. */
--bg:          oklch(99%   0.002 250);
--bg-sunk:     oklch(96.5% 0.003 250);
--fg:          oklch(17%   0.008 250);
--fg-muted:    oklch(52%   0.008 250);
--fg-faint:    oklch(68%   0.006 250);
--line:        oklch(90.5% 0.004 250);
--line-strong: oklch(80%   0.006 250);

/* Dark — genuinely dark and desaturated, not mid-grey. Same cool cast, so the two themes are
   one product rather than two. */
--bg:          oklch(15.5% 0.006 250);
--bg-sunk:     oklch(19%   0.007 250);
--fg:          oklch(93%   0.004 250);
--line:        oklch(26%   0.008 250);

/* One accent. Lifted and *desaturated* in dark: saturated colour on a dark ground vibrates. */
--accent:      oklch(52% 0.135 162);   /* light */
--accent:      oklch(72% 0.125 162);   /* dark  */
--accent-wash: oklch(95.5% 0.03 162);
```

**Rules.**

1. **One accent**, for what is interactive or current. Nothing else is coloured.
2. **No pure black or white.** The highest-strain pairing there is, on pages read for minutes.
3. **Colour is never the only signal.** The active filter is filled *and* carries `aria-current`.
4. **Dark is not inverted light.** Different lightness *and* different chroma for the accent.

## 4. Typography

The hard requirement: **Arabic, Darija in Arabic script, Arabizi in Latin, and French — often in
the same line.** Most pairings fail this. A Latin face chosen for elegance next to a default
Arabic fallback looks like a bug.

| Role | Face | Why |
|:---|:---|:---|
| Arabic | **Cairo** | A modern UI face designed for screen text. Settles §11's open question in favour of the option Algerian readers see most often. |
| Latin | **system-ui** stack | Until faces are self-hosted (M1B-T02.2). |
| Numerals | `font-variant-numeric: tabular-nums` | Counts, prices and rates must align in columns. |

**Never list a family that is not actually available.** Leading the stack with an uninstalled
`Inter` was not harmless: the browser fell back per glyph and **broke Arabic shaping outright** —
letters rendered unjoined and in reverse order. A font stack has to be honest about what exists.

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
