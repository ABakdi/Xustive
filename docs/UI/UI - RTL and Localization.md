---
tags:
  - ui
  - i18n
type: ui
status: specified
updated: 2026-08-06
---

# UI - RTL and Localization

> Four UI languages, two writing directions, and text that mixes both in the same paragraph.
> Parent: [[UI Specification]] · Related backend: [[Language Detector]], [[Query Expander]]

---

## 1. The Actual Problem

Algerian text is not neatly one language. A single Facebook post routinely contains Arabic script,
French words, Latin-script Darija, and ASCII digits — sometimes in one sentence. The UI has to render
that correctly *without knowing in advance* what any given string contains.

So the rule is: **direction is determined per-string at render time, not per-page**.

---

## 2. UI Languages

| Code | Language | UI direction | Notes |
|:---|:---|:---|:---|
| `ar` | العربية (MSA) | RTL | default for Arabic-locale browsers |
| `fr` | Français | LTR | |
| `en` | English | LTR | |
| `ary` | Darija | RTL chrome, Arabizi-friendly input | UI strings in Darija written in Arabic script |

Darija as a *UI* language is a real choice, not a token one: administrative Arabic reads as formal
and distant to many users, and Darija chrome ("شوف النتائج", "ما لقيناش والو") is warmer and clearer.
It also signals what the product is for.

Selection: `navigator.language` → override via [[UI - Home Page]] §4 → persisted in `localStorage`
as `xustive.lang`.

---

## 3. Layout Direction

`<html lang="ar" dir="rtl">` set server-side from the chosen UI language, so the first paint is
already correct — no flash of mirrored layout.

**Every layout property is logical**, no exceptions:

| Never use | Always use |
|:---|:---|
| `margin-left` / `right` | `margin-inline-start` / `-end` |
| `padding-left` / `right` | `padding-inline-start` / `-end` |
| `text-align: left` | `text-align: start` |
| `left` / `right` | `inset-inline-start` / `-end` |
| `border-left` | `border-inline-start` |
| `float: left` | `float: inline-start` |

Tailwind's logical utilities (`ms-*`, `me-*`, `ps-*`, `pe-*`, `text-start`, `text-end`) are used
throughout, and a CI lint rejects physical-direction utilities in component CSS. There is **no
separate RTL stylesheet** — that approach always drifts.

Flexbox and grid handle direction automatically; `flex-direction: row` reverses with `dir` for free.

---

## 4. Per-String Direction (`dir="auto"`)

Every element rendering user or corpus content gets `dir="auto"`, which sets direction from the
string's first strong directional character:

| Element | Why |
|:---|:---|
| Search input | flips to RTL the moment the first Arabic character is typed |
| Result title | an Arabic title in a French UI still reads correctly |
| Excerpt | same |
| Author name | same |
| Suggestion item | same |
| AI summary | same |

Without this, an Arabic title in an LTR page renders with its punctuation on the wrong side — the
classic "why does the question mark appear at the start" bug.

---

## 5. Bidi Hazards

The cases that break in practice, and their fixes:

| Case | Problem | Fix |
|:---|:---|:---|
| URL inside RTL text | `elkhabar.com/economie` reorders and becomes unreadable | wrap in `<bdi>` |
| Number + Arabic unit | `20 نتيجة` may reorder | `<bdi>` around the number |
| Mixed excerpt with `<em>` | highlight spans can straddle a direction run | keep `<em>` inside a single run; never split a word |
| Parentheses/brackets | mirror automatically, but neutral-run rules surprise people | test, do not assume |
| Punctuation at a string boundary | attaches to the wrong side | `dir="auto"` on the container handles most of it |
| Truncation with ellipsis | ellipsis lands on the wrong side | `text-overflow: ellipsis` is direction-aware only if `dir` is correct on that element |
| User content in `aria-label` | screen readers read a flattened string | build labels from separated fields, not concatenated ones |

Rule of thumb: **anything that concatenates a number, a URL, or a Latin token into an Arabic string
needs `<bdi>`.**

---

## 6. Icons and Direction

| Icon | RTL behaviour |
|:---|:---|
| Chevrons, arrows, next/prev | **mirror** |
| Back / forward | mirror |
| Search, mic, camera, filter, calendar | do **not** mirror |
| Platform logos | never mirror |
| Sentiment ▲ ▼ | do not mirror (vertical semantics) |

Implemented via `[dir="rtl"] .icon-directional { transform: scaleX(-1) }`, applied only to icons
explicitly marked directional — mirroring everything is the common mistake and it produces
backwards logos.

---

## 7. Numbers, Dates, Currency

| Item | Rule |
|:---|:---|
| Digits | **Western Arabic numerals (0–9) everywhere**, including in Arabic UI — this is the Algerian convention, unlike the Gulf |
| Number formatting | `Intl.NumberFormat(locale)` — `1,800` / `1 800` |
| Dates | `Intl.DateTimeFormat(locale)` with `Africa/Algiers` |
| Arabic month names | Algerian/Maghrebi forms — **أوت** not أغسطس, **جويلية** not يوليو |
| Relative time | `Intl.RelativeTimeFormat` |
| Calendar | Gregorian (Hijri is an open question — [[UI - Filters and Facets]] §9) |

The month-name detail is small and it matters: أغسطس reads as foreign to an Algerian reader. The same
applies to [[Content Parser]] §4.3, which must *parse* both forms.

---

## 8. String Management

- Strings live in `web/i18n/{ar,ary,fr,en}.json`, flat keys, no nesting deeper than one level.
- Interpolation uses named placeholders: `"results_count": "حوالي {count} نتيجة"`.
- **Pluralisation uses `Intl.PluralRules`.** Arabic has six plural categories (`zero`, `one`, `two`,
  `few`, `many`, `other`) — naive `count === 1 ? x : y` is wrong in Arabic for most numbers, and it
  is the single most common i18n bug in Arabic UIs.
- No string concatenation in code. A sentence assembled from fragments cannot be translated or
  ordered correctly.
- Missing key → fall back to `en` and log a build warning; a missing key fails CI, it does not ship.

---

## 9. Fonts

Per [[UI - Design System]] §3: `--font-arabic` applied via `:lang(ar)`, with `font-size: 1.08em` and
`line-height: 1.75` because Arabic at nominally equal size reads smaller and needs more leading.

Arabic must never be rendered in a font lacking proper shaping — broken letter joining is instantly
recognisable as amateur work and is a real risk with default Android fallbacks.

---

## 10. Testing

- Every screen has a visual regression snapshot in **all four languages**, both directions.
- A pseudo-localisation mode (`?pseudo=1`) wraps strings in markers and expands them 40 % to catch
  hard-coded text and truncation.
- Bidi fixture set: mixed Arabic/French sentences, URLs in Arabic text, numbers with Arabic units,
  Arabizi queries, emoji in RTL text.
- A CI lint fails on physical-direction CSS utilities and on `dir="auto"` missing from content slots.
- Native-speaker review of `ar` and `ary` strings before each milestone ships — machine-translated
  Darija is worse than English.

---

## 11. Open Questions

- [ ] Who writes and reviews the Darija UI strings? This needs a named person, not a hope.
- [ ] Should the UI language default follow the *query* language rather than the browser locale?
- [ ] Hijri dates as a display option?
- [ ] Do we support Tamazight (Latin or Tifinagh) chrome? It is an official national language, so the
      honest answer is "eventually" — and the scope should be decided rather than drifted into.

## Related

[[UI - Design System]] · [[UI - Component Library]] · [[UI - Accessibility]] · [[Language Detector]] ·
[[Query Expander]] · [[Content Parser]] · [[UI Specification]]
