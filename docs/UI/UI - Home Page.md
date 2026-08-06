---
tags:
  - ui
type: ui
status: specified
updated: 2026-08-06
---

# UI - Home Page

> Route `/`. The entry point: one search box and nothing competing with it.
> Parent: [[UI Specification]] · Components: [[UI - Component Library]]

---

## 1. Purpose

Get the user into a query as fast as possible, and communicate two things in passing: this searches
Algerian content, and it does not record what you search.

## 2. Layout

```
┌──────────────────────────────────────────────────────┐
│                                          [ عربية ▾ ] │  ← language toggle, top-inline-end
│                                                      │
│                    X U S T I V E                     │  ← wordmark, --text-3xl
│              محرك البحث الجزائري                      │  ← tagline, --text-sm, muted
│                                                      │
│   ┌────────────────────────────────────────────┐     │
│   │ 🔍  ابحث…                        🎤  📷    │     │  ← SearchBox, 60px
│   └────────────────────────────────────────────┘     │
│   ┌────────────────────────────────────────────┐     │
│   │ suggestions…                               │     │  ← SuggestionList (conditional)
│   └────────────────────────────────────────────┘     │
│                                                      │
│            🔒 ما نسجلوش عمليات البحث تاعك              │  ← privacy line, always visible
│                                                      │
│                                                      │
│   عن الموقع · الخصوصية · الروبوت · أضف مصدر            │  ← footer links
└──────────────────────────────────────────────────────┘
```

Vertical position: the search box sits at **38 % of viewport height** on `lg` and **28 %** on `sm`,
not centred — optical centre reads better than mathematical centre, and on mobile it keeps the box
clear of the keyboard.

## 3. Behaviour

| Event | Behaviour |
|:---|:---|
| Page load | search input receives focus on `lg`; **not** on `sm` (auto-opening the keyboard on mobile hides the page and is widely disliked) |
| Typing | debounce 120 ms → `GET /suggest` → render [[UI - Component Library]] §3 |
| `Enter` | navigate to `/search?q=…` |
| Empty submit | no-op, input shakes 120 ms (skipped under `prefers-reduced-motion`) |
| Mic button | → [[UI - Voice Search]] |
| Camera button | → [[UI - Image Search]] |
| `/` key | focus the search box |
| First Arabic character typed | box flips to RTL via `dir="auto"` |

Suggestion requests are aborted (`AbortController`) when a newer keystroke arrives — otherwise a slow
response overwrites a newer one, which reads as the list "jumping backwards".

## 4. Language Toggle

Four options; changing it sets the chrome language, the `lang` and `dir` attributes on `<html>`, and
a `lang` hint on subsequent searches ([[API Contract]] §2). It is **a UI language choice, not a
content filter** — a French UI still returns Arabic results. The toggle's label makes that clear.

Default: `navigator.language` → `ar` if Arabic, `fr` if French, `en` otherwise. Persisted in
`localStorage` under `xustive.lang` ([[UI Specification]] §9).

## 5. The Privacy Line

> 🔒 **ما نسجلوش عمليات البحث تاعك** — *We don't store your searches*

Always visible, never dismissable, not a modal. Links to `/privacy`, which explains *how* the claim
is structurally enforced rather than merely promised ([[Security and Privacy]] §1).

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

The wordmark is inline SVG. Critical CSS is inlined; the rest loads `media="print" onload` style. JS
is `defer` — the form works without it.

## 7. States

| State | Rendering |
|:---|:---|
| Default | as §2 |
| Typing, suggestions loading | previous suggestions remain; no spinner |
| Suggestions empty | list hidden entirely |
| Suggest endpoint failing | silent; the box keeps working ([[Autocomplete Service]] §7) |
| Offline | banner: "You appear to be offline" — the form still submits and fails visibly |
| Search unavailable (`/readyz` red) | banner above the box; the box remains usable so a retry is one keystroke away |

## 8. Accessibility

- The search form is a landmark (`role="search"`) and the first tab stop after skip-to-content.
- Combobox pattern per WAI-ARIA: `aria-expanded`, `aria-controls`, `aria-activedescendant`,
  `role="listbox"`/`option`.
- Suggestion count is announced via a polite live region ("8 suggestions available").
- The wordmark has an accessible name; the tagline is not a heading.
- Full commitments in [[UI - Accessibility]].

## 9. Not on This Page

Deliberately absent: trending searches (a query-log surface we do not want —
[[Autocomplete Service]] §12), news feed, ads, login, cookie banner (there are no cookies), app
install prompt, newsletter modal.

The home page has one job.

## 10. Open Questions

- [ ] Does the tagline work better in Darija than MSA? Worth testing with actual users.
- [ ] Should `/submit` be linked from the footer before [[Milestone 5 - Beta Launch]]?
- [ ] Is a "what is Xustive?" one-liner needed for first-time visitors, or does it add noise?

## Related

[[UI - Component Library]] · [[UI - Voice Search]] · [[UI - Image Search]] · [[UI - Results Page]] ·
[[Autocomplete Service]] · [[Security and Privacy]] · [[UI - Accessibility]]
