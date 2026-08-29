---
tags:
  - ui
  - responsive
  - accessibility
status: specified
date: 2026-08-29
---
# UI - Responsive

> How every surface behaves from a 320-pixel phone to a wide desktop. Companion to
> [[UI - Design Language]] (tokens, type, colour), [[UI - RTL and Localization]] (direction) and
> [[UI - Accessibility]] (targets, focus, motion). Audited and implemented 2026-08-29.

## 0. Why this exists

Algeria reaches the web on phones. The engine was built at a laptop and, measured at 360 and
390 CSS pixels on 2026-08-29, showed it:

| Surface | Measured at 360–390 px | What that means |
|:---|:---|:---|
| Home | no overflow | fine |
| Results | header search field squeezed to a sliver; vertical tabs overflowed the page by **29 px** | the most important control on the page was unusable, and the page scrolled sideways |
| Admin overview | page overflowed by **171 px** | a 190 px fixed sidebar left ~170 px for content — one word per line |
| Admin documents | page overflowed by **131 px** | filter inputs with `minWidth: 220…300` pushed the page wide |
| Everywhere | 33–87 controls under 40 px tall | fine for a mouse, wrong for a thumb |

The stylesheet contained **no width-based media query at all**, and the whole app used three
`sm:` classes and five `lg:` ones.

**After** (same harness, same widths, 2026-08-29): every one of those pages overflows by **0 px**
at 360 and 390; the results header shows the query on both; the admin console's navigation is a
scrolling strip above full-width content and its sidebar returns at `md`.

## 1. Breakpoints

Mobile-first: the base rules are the phone's, and each breakpoint adds. Tailwind's defaults,
used as follows — no new ones, because a breakpoint nobody can name is a breakpoint nobody
maintains.

| Token | Width | Who | What changes |
|:---|---:|:---|:---|
| base | ≥ 320 px | phones | one column; header wraps to two rows; admin nav is a scrolling strip |
| `sm` | 640 px | large phones, small tablets | header is one row; filter fields regain their natural width |
| `md` | 768 px | tablets | admin sidebar returns as a column; two-up admin grids |
| `lg` | 1024 px | laptops | the knowledge rail appears beside the results (already the rule) |
| `xl` | 1280 px | wide | nothing new; content stays capped for readability |

**320 px is the floor.** Below it we do not promise a layout; at it, nothing may scroll
sideways.

## 2. The rules

1. **No horizontal page scroll, ever, at ≥ 320 px.** Wide things — tables, charts, keypads, tab
   strips — scroll *inside their own container*, never by widening the page. `.scroll-x` is that
   container: `overflow-x: auto`, momentum, hidden scrollbar, `overscroll-behavior-x: contain`.
2. **The page gutter is a token**, `--pad` (1 rem on phones, 1.5 rem from `sm`). Nothing
   hard-codes `px-6` at the top level; a strip that should bleed to the screen edge uses
   `-mx-[var(--pad)] px-[var(--pad)]` so it starts flush and ends flush.
3. **Nothing has a fixed width in a flow layout.** A minimum width is written
   `w-full sm:w-auto sm:min-w-[N]`, so it is full-width on a phone and comfortable on a desktop.
   Grids use `minmax(0, …)` or `auto-fit`, never a bare `px` column below `md`.
4. **Touch targets are at least 40 px on coarse pointers** (`@media (pointer: coarse)`), applied
   to buttons, selects, chips and toolbar links — not to links inside prose, where a 40 px line
   box would be worse than a small target.
5. **The search field is the page.** It never shrinks below full width on a phone: the header
   wraps to two rows (wordmark and toggles above, the field across the bottom) and returns to one
   row at `sm`.
6. **Tab and chip strips scroll, they do not wrap into stacks** that push the results below the
   fold. The active tab scrolls into view.
7. **Tables scroll inside a wrapper**, and every admin table goes through the shared `Table`
   component that provides it. A table is never restyled into cards: an operator reading a table
   on a phone wants the same columns, and a card layout hides the comparison that is the point of
   a table.
8. **Dialogs are sheets on phones** — full width, bottom-anchored, `max-block-size: 90dvh`,
   scrolling inside — and centred panels from `sm`.
9. **Viewport units are dynamic** (`dvh`, not `vh`), because a mobile URL bar changes the
   viewport, and safe areas are respected (`viewport-fit=cover` plus `env(safe-area-inset-*)` on
   the sticky header and the footer).
10. **Direction-agnostic**: logical properties (`margin-inline`, `padding-block`, `inset-inline`)
    so a wrapped header and a scrolling strip behave identically in Arabic
    ([[UI - RTL and Localization]]).

## 3. Surface by surface

| Surface | Phone (base) | `sm` | `md` | `lg` |
|:---|:---|:---|:---|:---|
| Home | wordmark, field, one line of prose; nothing else competes | — | — | — |
| Results header | two rows: wordmark + toggles, then the field | one row | — | — |
| Verticals | scrolling strip, edge-to-edge | — | — | — |
| Filters / language chips | wrap, tappable | — | — | — |
| Results list | one column, `--result-gap` | — | — | rail appears beside |
| Knowledge rail | under the results | — | — | sticky column, 360 px |
| Images / videos grid | `auto-fill, minmax(150px, 1fr)` | 180 px | 200 px | — |
| Instant answers (calculator, converter) | keypad 5 across, fields stack | converter goes one row | — | — |
| Image search dialog | sheet | centred panel | — | — |
| Admin shell | nav is a scrolling strip on top | — | sidebar column returns | — |
| Admin tables | scroll inside their wrapper | — | — | — |
| Admin filter rows | fields full width, stacked | natural widths, one row | — | — |
| Admin charts | full width, height 120–160 | — | two-up | — |

## 4. Testing

- **The harness**: `/responsive?path=/fr/search%3Fq%3Dtest&w=360,390,768` (development only —
  it 404s in production) renders the given page in same-origin iframes at each width and
  `window.audit()` returns, per width, the page's overflow in pixels, the offending elements and
  a count of sub-40 px controls. This is how the table in §0 was measured and how a change is
  checked.
- **The static gate**: `scripts/lint-responsive.sh` fails a build on the mistakes that caused
  every row of that table — a fixed `minWidth`/`min-w-[…]` of 200 px or more without a
  breakpoint, a raw `100vh`, and a `<table>` outside the shared component.
- **Manual pass** before a release: home, results, images, an instant answer, the image dialog and
  three admin pages at 360 and 768, in French and Arabic.

## 5. Open questions

- A phone-specific results density (the `compact` toggle already exists; should coarse pointers
  default to it?).
- Whether the admin console deserves a genuinely mobile-first rethink rather than a shell that
  survives a phone — an operator on a phone is usually *checking*, not *steering*.
- Offline and slow-network behaviour on phones is [[UI - Frontend Architecture]]'s question, not
  this document's, but they meet here.

## Related

[[UI - Design Language]] · [[UI - RTL and Localization]] · [[UI - Accessibility]] ·
[[UI - Results Page]] · [[UI - Admin Console]] · [[UI - Frontend Architecture]]
