---
tags:
  - ui
type: ui
status: implemented
updated: 2026-08-27
---

# UI - Filters and Facets

> Narrowing results by language, source and tone.
> Data: [[API Contract]] §2 · Backend: [[Search Index]] faceting · Parent: [[UI Specification]]
>
> **Audited against the code 2026-08-27.** The shipped filter is `web/components/search/Filters.tsx`
> — a row of link chips, server-rendered, no rail, no bottom sheet, no date or sort filter. The
> 2026-08-06 spec is kept below where it is still the target and marked superseded where the
> shipped design went a different way on purpose.

---

## 1. Principles

1. **Filters are visible, not hidden.** A user who cannot see that a filter is active will conclude
   the search engine is broken.
2. **Every filter shows its cost.** Chips carry result counts so the user knows what narrowing will
   do before doing it.
3. **Zero-result filters are disabled, not offered.** *Superseded in practice (2026-08-27): the chip
   row drops values with a count of 0 rather than showing them disabled. See §7.*
4. **Filters live in the URL.** Shareable, back-button-correct, no hidden state
   ([[UI Specification]] §7).
5. **One tap to clear.** "Clear all" is always reachable when any filter is active.
6. **Links, not script.** (Added with the implementation.) Every chip is an `<a>` to a URL, so
   narrowing works with JavaScript disabled and every filtered view is a URL you can share. A filter
   that needs script disappears on exactly the connection where narrowing matters most.

---

## 2. Filter Inventory

What the row renders, from the `GROUPS` table in `Filters.tsx`:

| Filter | Facet field | Values (from the response) | Param | Group label |
|:---|:---|:---|:---|:---|
| Language | `language` | `ar`, `ary`, `fr`, `en`, `mixed` — labelled via `lang_*` keys | `lang` | `language` |
| Source | `source_type` | whatever the index holds (e.g. `web`) — labelled via a same-named key, else the raw value | `source` | `source` |
| Tone | `sentiment.label` | `positive`, `neutral`, `negative` | `sentiment` | `tone` |

**Single-select within a group.** Each param holds one value; clicking a different value replaces
it, clicking the active value clears it. The multi-select "OR within a filter" spec (2026-08-06) is
superseded — the API accepts one value per param today. Across groups it is AND, as before:
`lang=fr` + `sentiment=negative` means "French AND negative".

**Not built:** the Date filter (`from`/`to`, presets, custom range) and Sort. §5 keeps the date spec
as the target. Verticals (`v=news|images|videos|files`) are a separate control
([[UI - Search Verticals]]), not a facet.

---

## 3. Layout

One horizontal chip row, `mb-5 flex flex-wrap gap-x-3 gap-y-2`, rendered above the summary (for a
topic) and the result list, only when there is at least one result. Each group is
`<div role="group" aria-label={label}>` with a muted `text-xs` label followed by its chips
(`LinkButton` → `.chip`); the active chip is `variant="emphasis"` with `aria-current="true"`.
"Clear all" (`clearFilters`) is a dashed `.chip-clear` link at the end of the row, present only
when something is active.

```
language  [العربية 820] [Français 520] [English 90]   source [web 1.2k]   tone [▲ 401] [● 1100] [▼ 333]   [clear filters]
```

**Superseded (2026-08-27):** the sticky `lg` rail with checkboxes and radios, and the `sm`
chip-row-plus-`<dialog>`-bottom-sheet, were not built. With three small groups and no date filter
the single row covers every breakpoint, and it is the same markup at every size — nothing to keep
in sync. The original argument for a live bottom sheet ("an Apply button hides the effect of a
choice") still applies if a sheet is ever added.

The results column has a right-hand knowledge rail at `lg` ([[UI - Results Page]]); the filter row
stays in the results column, not the rail.

---

## 4. Behaviour

| Event | Behaviour (today) |
|:---|:---|
| Click a chip | full navigation to `/{lang}/search?q=…&{other active params}&{param}={value}` — a new server render |
| Click the active chip | same, with that param removed |
| Other active filters | **preserved** on every chip link (a bug in an earlier version silently dropped the language when narrowing by tone) |
| Page number | dropped — a new filter starts at page 1; `Pagination` in turn carries `lang`/`source`/`sentiment` on every page link |
| Vertical (`v`) | **not carried** by chip links today — filtering from the Images tab returns to `all` |
| Scroll position | browser default for a navigation (top); the "preserved" spec is superseded with the link design |
| Focus | browser default after navigation |
| Counts | from the new response's `facets` |
| Clear all | `/{lang}/search?q=…` — removes every filter param, keeps `q` |
| Zero results after filtering | the plain empty state; the chip row is **not** rendered on an empty page, so the filter that emptied it cannot be undone in place ([[UI - States and Errors]] §3) |

There is no debounce, no `replaceState`, no aborting of in-flight requests: each click is one
request and one page.

---

## 5. Date Filter

**Not built (2026-08-27).** Kept as the target design.

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

Labelled **tone** in the interface (`tone` key: "Ton" / "الانطباع"). Chips are the translated label
(`positive` / `neutral` / `negative` keys) plus the count — text, never colour alone
([[UI - Accessibility]] §4). The result cards themselves show a glyph + word per result.

**Not built:** the explanatory line under the group —

> Sentiment is estimated automatically and is often wrong for sarcasm and dialect.

— is still the right thing to say. [[Sentiment Engine]] §12 documents the sarcasm limitation
explicitly, and the UI should not pretend otherwise. Low-confidence documents are labelled `neutral`
by the backend, so a "neutral" filter includes "we're not sure" — worth keeping in mind when
interpreting the counts.

---

## 7. Facet Counts

- Come from `facets` in the search response ([[API Contract]] §2), typed
  `Record<string, Record<string, number>> | null` — nullable defensively, because an older backend
  sent `null` when the facet stage was deadline-dropped and that crashed the Server Component
  (BUG-001).
- Counts reflect **the query plus all other active filters**, not the unfiltered corpus — so the
  numbers always answer "what happens if I add this filter".
- Values are sorted by count, descending, and formatted with `formatNumber(lang, n)` (locale
  digits; no "1.2k" abbreviation is applied).
- **Zero-count values are dropped**, not disabled. A group with fewer than two values is hidden
  altogether — **unless** one of them is the active filter, in which case that single value *is*
  the filter and hiding it would strand the reader with no way back.
- If no group survives, the row is not rendered at all.
- When facets were dropped under load (`facets_degraded`), the row is absent and a faint note
  `filtersUnavailable` says filters are resting ([[UI - States and Errors]] §5). The spec's "chips
  without counts" is superseded: with no facet values there is nothing to draw chips from.

---

## 8. Accessibility

- Chips are links (`<a class="chip">`), not `role="switch"`: a link that navigates is what they
  are, and a switch that reloads the page would lie. The active chip carries `aria-current="true"`
  and the `.chip[aria-current]` style (accent wash + border + weight 550) — state by attribute
  *and* fill, never colour alone.
- The count is inside the link text (`label` + `<span class="numeric">count</span>`), so the
  accessible name reads "Français 520".
- Each group is `role="group"` with `aria-label` = the group label; the row itself has no landmark.
  The spec's `<fieldset>`/`<legend>` and `role="region" aria-label="Filters"` were the rail design
  and went with it.
- No live-region announcement after a filter change — the page reloads and the result count line
  is the first thing in the main column.
- Everything is reachable and operable by keyboard in visual order: label, chips, clear.

---

## 9. Open Questions

- [ ] Should a Hijri calendar option be offered alongside Gregorian for the date filter? It is
      culturally expected in some contexts and adds real complexity.
- [ ] Is a "wilaya" geo filter worth building, given `geo.wilaya` is populated by a gazetteer with
      unknown coverage? ([[Enrichment Pipeline]] §4.1)
- [ ] Should filters persist across searches within a session, or reset with each new query?
      (Built: reset — the search box submits `q` only.)
- [ ] Do we expose a "verified sources only" filter based on `trust_tier`?
- [ ] Carry `v` on chip links so a filter applied inside the Images tab stays in it.
- [ ] Render the chip row on an empty page so a filter can be removed in place.
- [ ] The date filter — the one from the inventory most often asked for and still absent.

## Related

[[UI - Results Page]] · [[UI - Component Library]] · [[UI - Search Verticals]] · [[API Contract]] ·
[[Search Index]] · [[Sentiment Engine]] · [[Data Model]] · [[UI - Accessibility]] ·
[[UI - States and Errors]]
