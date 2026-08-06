---
tags:
  - ui
type: ui
status: specified
updated: 2026-08-06
---

# UI - Filters and Facets

> Narrowing results by date, source, sentiment, and language.
> Data: [[API Contract]] §2 · Backend: [[Search Index]] faceting · Parent: [[UI Specification]]

---

## 1. Principles

1. **Filters are visible, not hidden.** A user who cannot see that a filter is active will conclude
   the search engine is broken.
2. **Every filter shows its cost.** Chips carry result counts so the user knows what narrowing will
   do before doing it.
3. **Zero-result filters are disabled, not offered.** A chip with count 0 is disabled with its count
   visible — that is information, not a dead end.
4. **Filters live in the URL.** Shareable, back-button-correct, no hidden state
   ([[UI Specification]] §7).
5. **One tap to clear.** "Clear all" is always reachable when any filter is active.

---

## 2. Filter Inventory

| Filter | Type | Values | Param |
|:---|:---|:---|:---|
| Source | multi-select chips | web, Facebook, Instagram, TikTok | `source` |
| Sentiment | multi-select chips | positive, neutral, negative | `sentiment` |
| Date | single-select + custom range | any time, 24 h, week, month, year, custom | `from`, `to` |
| Language | multi-select | Arabic, Darija, French, English | `lang` |
| Sort | single-select | relevance, most recent | `sort` |

Multi-select semantics are **OR within a filter, AND across filters**: `source=web,facebook` +
`sentiment=negative` means "(web OR facebook) AND negative". This is what users expect and it needs
no explanation in the UI.

---

## 3. Layout

### `lg` — sticky rail

```
┌─ Filters ───────────┐
│ Clear all           │   ← only when something is active
│                     │
│ SOURCE              │
│ ☑ Web          900  │
│ ☑ Facebook     700  │
│ ☐ Instagram    150  │
│ ☐ TikTok        84  │
│                     │
│ DATE                │
│ ◉ Any time          │
│ ○ Past 24 hours     │
│ ○ Past week         │
│ ○ Past month        │
│ ○ Custom…           │
│                     │
│ SENTIMENT           │
│ ☐ ▲ Positive   401  │
│ ☐ ● Neutral   1100  │
│ ☑ ▼ Negative   333  │
│                     │
│ LANGUAGE            │
│ ☐ العربية       820 │
│ ☐ Darija        410 │
│ ☐ Français      520 │
│ ☐ English        90 │
└─────────────────────┘
```

Plus a horizontal quick-chip row above the results for the two most-used filters (source, date), so
the common case never requires looking at the rail.

### `sm` — chip row + bottom sheet

A horizontally scrollable chip row shows active filters first, then the quick options, then a
`[⚙ All filters]` button opening a full-height `<dialog>` bottom sheet.

The sheet applies changes **live** (each toggle re-fetches) with a "Done" button that just closes it.
An Apply-button model means the user cannot see the effect of a choice while making it, which defeats
the point of showing counts.

---

## 4. Behaviour

| Event | Behaviour |
|:---|:---|
| Toggle a chip | update the URL (`replaceState`) → fetch → re-render results and counts |
| Scroll position | preserved on filter change; **not** reset to top |
| Focus | stays on the toggled chip after re-render |
| Counts | update from the new response's `facets` |
| Clear all | removes every filter param, keeps `q`, re-fetches |
| Zero results after filtering | empty state naming the filters to remove ([[UI - States and Errors]] §3) |
| Filter change while a search is in flight | abort the previous request; last write wins |

Rapid toggling is debounced 150 ms so tapping three chips fires one request, not three.

---

## 5. Date Filter

Presets cover almost every real use. "Custom…" opens two date inputs (`<input type="date">` — native
pickers are better than anything we would build, and they localise themselves).

| Detail | Spec |
|:---|:---|
| Timezone | `Africa/Algiers` for boundary computation |
| `from` | inclusive, 00:00 local |
| `to` | inclusive, 23:59:59 local |
| Validation | `from ≤ to`; a reversed range swaps rather than errors |
| Display | "4 Aug – 6 Aug 2026" in the active chip |
| Calendar | Gregorian; Hijri display is an open question in §9 |

Documents with `published_at_precision = "unknown"` are **excluded** from date-filtered results, and
the UI says so when a date filter is active: *"Results with unknown dates are hidden."* Silently
including guessed dates in a date filter would make the filter a lie ([[Data Model]] §2).

---

## 6. Sentiment Filter

Each option is icon + colour + text label, never colour alone ([[UI - Accessibility]] §4).

An explanatory line sits under the group, because sentiment is the least self-explanatory filter:

> Sentiment is estimated automatically and is often wrong for sarcasm and dialect.

This is honest rather than defensive. [[Sentiment Engine]] §12 documents the sarcasm limitation
explicitly, and the UI should not pretend otherwise. Low-confidence documents are labelled `neutral`
by the backend, so a "neutral" filter includes "we're not sure" — worth keeping in mind when
interpreting the counts.

---

## 7. Facet Counts

- Come from `facets` in the search response ([[API Contract]] §2).
- Counts reflect **the query plus all other active filters**, not the unfiltered corpus — so the
  numbers always answer "what happens if I add this filter".
- Counts are estimates when `pagination.estimated` is true; over 1 000 they render as "1.2k".
- When facets were dropped under load ([[Error Handling and Resilience]] §6), chips render **without**
  counts and remain fully usable. No error is shown — a missing count is a degraded detail, not a
  failure.

---

## 8. Accessibility

- Chips are `role="switch"` with `aria-checked`; their accessible name includes the count
  ("Facebook, 700 results").
- Filter groups are `<fieldset>` with a `<legend>`; the rail is `role="region"` with
  `aria-label="Filters"`.
- After a filter change, a polite live region announces the new count once ("About 333 results").
- The bottom sheet is a focus-trapped modal `<dialog>`; `Esc` closes and focus returns to the button
  that opened it.
- Every filter is reachable and operable by keyboard in a logical order; the rail follows the results
  in DOM order on `lg` but is reachable via a skip link.

---

## 9. Open Questions

- [ ] Should a Hijri calendar option be offered alongside Gregorian for the date filter? It is
      culturally expected in some contexts and adds real complexity.
- [ ] Is a "wilaya" geo filter worth building, given `geo.wilaya` is populated by a gazetteer with
      unknown coverage? ([[Enrichment Pipeline]] §4.1)
- [ ] Should filters persist across searches within a session, or reset with each new query?
      (Leaning: reset — persistent filters silently distort later searches.)
- [ ] Do we expose a "verified sources only" filter based on `trust_tier`?

## Related

[[UI - Results Page]] · [[UI - Component Library]] · [[API Contract]] · [[Search Index]] ·
[[Sentiment Engine]] · [[Data Model]] · [[UI - Accessibility]]
