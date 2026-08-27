---
tags:
  - ui
  - design
status: implemented
updated: 2026-08-27
---

# UI — Design Language

> Replaces the former `UI - Design System` note, which is deleted rather than kept alongside —
> two token lists is how the values drift. Contrast obligations live in [[UI - Accessibility]].
>
> **Source of truth is `web/app/globals.css`.** This note was re-read against it on 2026-08-27:
> the tokens in §3–§5 and §7 are now copied from the stylesheet, and the sections that described
> the earlier square, green, hairline system are kept as history and marked superseded rather than
> deleted, because the reversal was deliberate and someone reading later should see it.

## 0. The soft pass

The system was rebuilt around **rounding, elevation and space** after the previous look — hairline
rules and 2px corners — was judged old and clunky twice.

That earlier version was coherent, and it read as a *printed table*. Correct for a document, wrong
for something you act on. Three things separate the two, and of the three the space matters most:
the corners are what you notice, the padding is what makes it feel calm.

| | Was | Now |
|:---|:---|:---|
| Radius | 2px everywhere | 8 / 14 / 20px, plus a pill for anything you press |
| Separation | hairline rules | elevation — surfaces sit *on* the page rather than being cut from it |
| Search field | rectangle | pill (`.field`), lifted, focus raises rather than outlines |
| Current chip | inverted to solid black | accent-tinted (`.chip[aria-current]`) |
| Result | no container | a card (`.card`) with hover lift |

**This reverses the earlier "do not use rounded elements a lot" rule**, deliberately and on
instruction. Recorded rather than quietly changed, because the old rule was also given
deliberately and someone reading the history should see the reversal rather than a contradiction.

One accent still, for what is interactive — plus, since 2026-08-27, **one second hue with exactly
one job** (§6). A second colour used anywhere else is how a tool starts looking like a dashboard.

The admin console mirrors these tokens by hand rather than importing them — it must work when the
frontend is down, so it depends on nothing outside its own binary. The cost is that the two are
kept in step deliberately ([[UI - Admin Console]]).

## 1. The problem with looking like everything else

shadcn/ui out of the box is a known quantity: neutral greys, `oklch` slate, 0.5 rem radii, Inter.
It is excellent and it is *everywhere*. An Algerian search engine that looks like a Vercel
template says nothing about what it is.

The primitives are still the right foundation — accessible, unstyled-by-default, source-copied so
they can be rewritten rather than fought. In practice that meant *writing* `Button`, `Select`,
`Toggle` as plain elements with a class (`components/ui/`), because the shadcn originals are
Radix client components and the results page ships no client runtime for its list. What follows
replaces the skin entirely.

## 2. The idea: editorial, structural — and, once, square

> **Superseded by §0 (2026-08-09).** Kept because the reasoning about *tone* still holds; the
> geometry does not.

A first attempt used warm manuscript paper and blue-hour indigo. It was rejected as insufficiently
polished, and the criticism was right: warmth reads as nostalgia, and a product that indexes the
live web should not look like it is about the past.

What replaced it is **editorial**. Near-neutral surfaces with a faint cool cast, one accent, and —
in that version — geometry that was essentially square: a 2 px radius that existed only so 1 px
borders would not fray, a rectangular search field, chips as rectangles, result cards with no
border at all, structure from hairlines and 30 px of space. That is the part §0 reversed. The
search field is a pill now, the cards are surfaces with a shadow, and the radii are 8/14/20.

What still holds: no glassmorphism, no gradient meshes, no neon, nothing that animates for longer
than 100 ms. A search engine is a reading surface. The design's job is to disappear.

## 3. Colour

Perceptual (`oklch`) so a hue shift does not change apparent brightness — which matters when the
same accent has to hold against near-black text and against a dark ground.

The values below are `globals.css` as of 2026-08-27. Two things changed since the first version:
the cast moved from cool grey (hue 250) to a **faint violet (hue 275)** so the ground and the
accent belong to the same family, and the accent is no longer green.

```css
/* Light — :root. Off-white with a faint violet cast. Flat white glares under long reading;
   warm cream reads as nostalgia. This should look like a well-set page. */
--bg:            oklch(98.8% 0.004 275);
--bg-sunk:       oklch(96%   0.006 275);
--surface:       oklch(100%  0 0);        /* cards sit on the page rather than being cut from it */
--surface-hover: oklch(98.5% 0.002 250);
--fg:            oklch(17%   0.008 250);
--fg-muted:      oklch(52%   0.008 250);
--fg-faint:      oklch(65%   0.006 250);
--line:          oklch(90.5% 0.004 250);
--line-strong:   oklch(65%   0.006 250);  /* borders controls; 3:1 against the page, was 80% = 1.8:1 */

/* Dark — :root[data-theme='dark']. Genuinely dark and desaturated, not mid-grey. */
--bg:            oklch(15%   0.012 275);
--bg-sunk:       oklch(18.5% 0.014 275);
--surface:       oklch(19.5% 0.013 275);  /* above the page = lighter, on dark */
--surface-hover: oklch(23%   0.015 275);
--fg:            oklch(93%   0.004 250);
--fg-muted:      oklch(66%   0.007 250);
--fg-faint:      oklch(50%   0.007 250);
--line:          oklch(26%   0.008 250);
--line-strong:   oklch(48%   0.01  250);

/* The accent: a vibrant indigo-violet (2026-08-27; the muted green read as a government portal). */
--accent:        oklch(55% 0.23  275);   /* light */   oklch(72% 0.19  278);   /* dark */
--accent-hover:  oklch(48% 0.24  275);                 oklch(80% 0.17  278);
--accent-wash:   oklch(96% 0.035 275);                 oklch(28% 0.07  278);
--accent-fg:     oklch(99% 0 0);                       oklch(15% 0.02  278);

/* The second hue — the assert rule only (§6). */
--assert-accent: oklch(62% 0.19 25);     /* light */   oklch(72% 0.16 30);     /* dark */

--warn:   oklch(58% 0.13 65);
--danger: oklch(53% 0.18 27);
```

Why the green went: it was chosen to be restrained and it read as institutional. The violet is
saturated enough to carry a page and still legible as a link colour — anything past ~0.2 chroma at
that lightness fails contrast on white. Chroma is what makes it feel modern, not hue; the pale wash
does the tinting so the page stays calm while the accent itself stays strong.

**Rules.**

1. **One accent**, for what is interactive or current. The assert hue is the one exception and it
   marks exactly one thing.
2. **No pure black or white** for text or the page. (`--surface` is pure white in light mode —
   a card on an off-white page — which is the one place it is allowed.)
3. **Colour is never the only signal.** The active filter is filled *and* carries `aria-current`.
4. **Dark is not inverted light.** Different lightness *and* different chroma for the accent, and
   elevation is carried by a lighter surface because a shadow is invisible on a dark ground.

`scripts/contrast-audit.mjs` reads these tokens from the stylesheet and checks the text and
control pairs in both themes ([[UI - Accessibility]] §4).

## 4. Typography

The hard requirement: **Arabic, Darija in Arabic script, Arabizi in Latin, and French — often in
the same line.** Most pairings fail this. A Latin face chosen for elegance next to a default
Arabic fallback looks like a bug.

| Role | Face | Why |
|:---|:---|:---|
| Arabic | **IBM Plex Sans Arabic**, self-hosted, 400 and 600 static | Drawn alongside the Latin family, so a URL or a Latin name inside an Arabic line matches rather than clashing — which is the common case on every result page. Cairo, then Noto, as fallbacks. |
| Latin | **IBM Plex Sans**, self-hosted, variable 400–600 | One variable file covers the range. Requesting three static weights instead cost 468 KB against 172 KB. |
| Mono | `ui-monospace`, DejaVu Sans Mono | admin and the odd identifier |
| Numerals | `font-variant-numeric: tabular-nums` on `bdi` and `.numeric` | Counts, prices and rates must align in columns. |

**Never list a family that is not actually available.** Leading the stack with an uninstalled
`Inter` was not harmless: the browser fell back per glyph and **broke Arabic shaping outright** —
letters rendered unjoined and in reverse order. A font stack has to be honest about what exists:
`'IBM Plex Sans', system-ui, -apple-system, 'Segoe UI', Roboto, 'Noto Sans', sans-serif`.

Self-hosting is forced rather than chosen: the CSP is `default-src 'self'`, so a font from a CDN is
blocked, and relaxing it would hand a third party the IP of everyone who loads a results page.
(The CSP header is set by the Rust API's `security_headers` middleware in
`crates/xustive-api/src/lib.rs`, not by `next.config.ts` — which sets Referrer-Policy, nosniff,
COOP and Permissions-Policy only. Whether the Next-served HTML carries the CSP depends on what sits
in front of both; see [[Deployment Topology|Deployment]].) The files are fetched by `scripts/fetch-fonts.sh` into
`web/public/fonts/` and **committed** — a build that needs the network to succeed is a build that
fails when the network does.

Split per script with `unicode-range` in the generated `app/fonts.css`, `font-display: swap`, and
one file preloaded per direction in `app/[lang]/layout.tsx`: an RTL page preloads Arabic 400
(42 KB), an LTR page the Latin variable file (45 KB); the Arabic 600 (45 KB) and Latin-extended
(30 KB) files load only if the page contains that script. (Sizes measured 2026-08-27; the first
version's 86 / 44 KB predates the re-subsetting.)

```css
/* @theme in globals.css */
--text-xs: 0.75rem;   --text-sm: 0.8125rem;  --text-base: 0.9375rem;
--text-lg: 1.0625rem; --text-xl: 1.375rem;   --text-2xl: 1.875rem;
/* no leading tokens: body { line-height: 1.6 }, h1–h3 1.3 at weight 550 */
```

**Arabic runs 1.85 line-height and 1.04em** at the same nominal size (`:lang(ar), :lang(ary)`).
Arabic glyphs carry more vertical detail; typeset at Latin metrics they are cramped and slow to
read. This is set per-element from the content language, not per-page — a French interface
showing an Arabic result needs it on the result alone.

## 5. Shape, space, motion

```css
--radius-sm: 8px;  --radius: 14px;  --radius-lg: 20px;  --radius-pill: 999px;
--shadow:      0 1px 2px oklch(0% 0 0 / .05), 0 1px 3px oklch(0% 0 0 / .04);   /* resting */
--shadow-lift: 0 2px 4px oklch(0% 0 0 / .06), 0 4px 12px oklch(0% 0 0 / .07);  /* hover */
--shadow-pop:  0 4px 8px oklch(0% 0 0 / .07), 0 12px 28px oklch(0% 0 0 / .1);  /* focused field, menus */
--pad-card: 1.15rem 1.35rem;  --gap: 0.75rem;  --measure: 680px;
```

Three radii, used consistently — a scale with more values than that produces pages where nothing
quite lines up. A 16 px corner reads as a surface you can act on, a 2 px corner reads as a printed
table. (Superseded 2026-08-09: the first version's "radii are small, 4/8 px" and "elevation is a
hairline, not a shadow" are the rules §0 reversed.)

**Elevation instead of borders.** Surfaces are separated by light rather than by lines. Two levels
in use: resting, and lifted on hover; `--shadow-pop` for the one thing that must occlude — the
focused search field, the suggestion list, the language menu. All soft and low-contrast: a hard
shadow reads as 2010.

**Motion budget: 100 ms, ease-out, opacity and 3 px translate only** (`.rise`). No spring, no
scale, no stagger. Inside `@media (prefers-reduced-motion: no-preference)`. A result list that
animates in is a result list you cannot read for 300 ms. The one longer animation is the recording
button's 1.4 s breathe, off under `reduce` ([[UI - Voice Search]]).

## 6. The signature: the assert rule

One distinctive element, used sparingly, so the product is recognisable without being decorated.

A **3 px rule on the inline-start edge** of anything the engine is asserting rather than merely
listing: the AI summary (`Summary.tsx`) and every tool card (`ToolCard.tsx`, `TranslateCard.tsx`).
The class is `.assert`; it mirrors automatically in RTL because it is `border-inline-start`.

Two things changed on 2026-08-27 from the "qalam rule" the first version described:

- It is a **tinted, elevated surface** (`--surface`, `--shadow`, `--radius`, `--pad-card`) with the
  rule on its edge, not a bare rule — on a page of soft cards a lone hairline looked like a
  leftover. A question's answer (`.summary-answer`) is lifted one step more and set larger,
  because its placement already says it is primary and the surface should agree.
- It uses **its own hue**, `--assert-accent` (a red-orange), rather than the accent. With the
  accent on every link and chip, an accent-coloured edge no longer stood out; a second hue with
  exactly one job means an instant answer is distinguishable from a result at a glance without a
  legend.

It carries meaning, which is the only reason it earns its place: **a coloured edge means Xustive is
telling you something, not showing you what someone else published.** Result cards never have it.
The entity panel and the relation row do not either — they are facts the engine *found*, marked
by the accent chip and the plain surface instead.

## 7. Focus, layering, icons

```css
:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
  border-radius: 0;          /* the ring is square even where the element is not */
}
```

Focus is **never removed, only restyled**. `:focus-visible` rather than `:focus`, so mouse users
see no rings and every keyboard user always does. The search field is the one exception: focus
*lifts* it (`--shadow-pop`) and colours its border with the accent instead of drawing a ring,
because a ring on a pill reads as an error state — the border keeps it visible in forced-colours
mode, where shadows are dropped. Hit targets: 44 px is the target, not yet the practice
([[UI - Accessibility]] A5).

| Token | Value | Layer |
|:---|:---|:---|
| `--z-sticky` | 10 | sticky results header |
| `--z-dropdown` | 20 | suggestion list, language menu |
| `--z-sheet` | 30 | (reserved — no mobile sheet exists yet) |
| `--z-modal` | 40 | (reserved — no dialog exists) |
| `--z-toast` | 50 | offline banner |

There is no `--z-base`; content simply has no `z-index`. **Any `z-index` not from this table is a
bug.** The suggestion panel losing to the sticky header is what happens without a scale, and it
happened.

Icons, two sources by design:

- **`lucide-react`** in client components only (search box, voice, toggles, language menu, OCR),
  at 16–18 px, `aria-hidden`. Directional ones must carry `rtl-flip`; `scripts/rtl-icons.sh`
  enforces it ([[UI - RTL and Localization]] §6).
- **`components/ui/Icon.tsx`**, a hand-drawn set of ~30 paths (24-unit viewBox, 1.75 stroke,
  `currentColor`, `aria-hidden`), for the server-rendered entity panel, relation row and summary
  badge — an icon package would cost more JavaScript than the whole panel, and the results page
  ships none for its content. Its chevrons are named `chevron-start`/`chevron-end`, logically.

A control whose only content is an icon gets an `aria-label` ([[UI - Accessibility]] §3). Weather
glyphs are drawn to the same stroke weight ([[UI - Tool Cards]]).

Named classes worth knowing, all in `globals.css`: `.field` (the pill search box), `.chip` /
`.chip-active` / `.chip-clear` / `.chip-count`, `.card`, `.ghost` (header icon buttons),
`.btn-quiet` (dismiss), `.assert` / `.summary-answer`, `.list-row` (the hidden-scrollbar relation
row), `.voice-button` / `.is-recording` / `.voice-meter`, `.rise`, `.numeric`, `.rtl-flip`.
Matched terms are a bare `<em>`: weight 550 with an inset `--accent-wash` underline, not italic —
a page of coloured rectangles is harder to scan than the text it is meant to help you scan.

---

## 8. Density

Two modes, remembered per device.

| | Comfortable (default) | Compact |
|:---|:---|:---|
| `--result-gap` | 12 px | 6 px |
| Attribute | `data-density="comfortable"` on `<html>` | `data-density="compact"` |

The gap between result cards is the whole difference today (the first version's 24/14 px predate
the card layout). Comfortable by default because the first-run impression should be readable, not
dense. Compact matters more here than on most sites: Arabic sets taller than Latin at the same
point size, so a list that fits one screen in French runs onto two in Arabic, and much of the
audience is on small phones.

Mechanism (mirrors the theme exactly, §9): `DensityToggle` is a ghost button whose `aria-label`
names the **current** state; a click sets `document.documentElement.dataset.density` at once and
writes the `xustive-density` cookie through a Server Action (`lib/prefs.ts`), then
`router.refresh()` so the next server render agrees. `readDensity()` puts the attribute on
`<html>` before the first byte.

## 9. Themes

Three settings — **system** (default), light, dark — stored in the `xustive-theme` cookie (one
year, `lax`, not `httpOnly`, `lib/prefs.ts`). Cookie rather than `localStorage` because the theme
has to be known before the first byte; anything the server cannot read produces a flash of the
wrong theme, which is the single most common failure of a dark-mode implementation and looks like
a bug every time.

How the three states become two attributes:

1. `readTheme()` in `app/[lang]/layout.tsx` renders `<html data-theme="system|light|dark">`.
2. `THEME_SCRIPT` (`lib/theme.ts`), the only inline script on the page, runs in `<head>` **before
   paint** — not in an effect, an effect is what causes the flash — and rewrites `system` to
   `dark` or `light` from `prefers-color-scheme`. `suppressHydrationWarning` on `<html>` covers the
   deliberate attribute difference.
3. The stylesheet only knows two: `:root` is light, `:root[data-theme='dark']` overrides. There is
   no `[data-theme='light']` block (superseded 2026-08-27 — the first version showed one).
   `color-scheme` is set on each so native controls follow.

`ThemeToggle` cycles system → light → dark, sets the resolved attribute immediately, writes the
cookie in the background and refreshes. Its `aria-label` names the state it is *in*
(`themeSystem` / `themeLight` / `themeDark`), so a screen-reader user pressing it repeatedly knows
where they landed. No `matchMedia` listener: a system-theme change mid-session applies on the next
navigation.

## 10. What this is not

- Not a dashboard. No stat tiles, no second colour beyond the one assert hue.
- Not brutalist. Soft surfaces and space, not raw borders and system fonts.
- Not "AI-styled". No purple gradients, no glow. The summary carries a small sparkle-and-badge
  ("AI summary") because a reader is entitled to know which paragraph a model wrote — but it is
  marked by the same assert rule as a unit conversion, because both are the engine speaking.

## 11. Open questions

- [ ] Should a Maghrebi geometric motif appear at all — in the empty state, the 404, the footer?
      Risk: decoration that dates fast and reads as costume. Leaning towards **one** use, in the
      zero-results illustration only.
- [x] Is IBM Plex Sans Arabic the right face, or is Cairo more familiar to Algerian readers?
      **Plex Arabic**, with Cairo as the first fallback. Familiarity was the argument for Cairo,
      but every result page mixes scripts — a French headline above an Arabic snippet, a Latin
      domain in an Arabic breadcrumb — and the two Plex families are drawn together, so the mixed
      case looks intentional instead of accidental. Verified rendering in a browser.
      Needs a native-speaker read ← *B7*
- [x] Tabular numerals in Arabic contexts: Eastern (٤٥) or Western (45)? **Western**, set
      explicitly as `numberingSystem: 'latn'` in `lib/i18n/format.ts` — Algerian print leans
      Western. Kept in one place so reversing it is a one-line edit. Native-speaker confirmation
      still wanted ← *B7*. (Two components bypass the helper; [[UI - RTL and Localization]] §7.)
- [ ] `Button.tsx`'s header comment still says "radius is 2px, no shadow" — a stale comment from
      the square era, harmless but misleading to the next reader.

## Related

[[UI - Accessibility]] · [[UI - RTL and Localization]] ·
[[UI - Component Library]] · [[Instant Answers]] · [[ADR-0010 - Next.js for the Frontend]]
