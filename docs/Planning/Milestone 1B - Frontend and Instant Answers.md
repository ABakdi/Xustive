---
tags:
  - planning
  - milestone
status: planned
updated: 2026-08-07
---

# Milestone 1B — Frontend and Instant Answers

> Sits between [[Milestone 1 - Text Search MVP]] and [[Milestone 2 - Multimodal Input]]. Numbered
> 1B rather than appended to M1 because M1's remaining work — the task queue, the indexer worker —
> is backend and can proceed in parallel with none of this.

## Why this milestone exists

Two things forced it.

**The UI hit its ceiling.** Every component existed twice, once in Rust and once in JavaScript,
and the language filter shipped broken on the server path because the two drifted. That is not a
bug to fix; it is a structure to replace ([[ADR-0010 - Next.js for the Frontend]]).

**Search alone is not a reason to switch engines.** Someone using Google will not move for
slightly better Algerian results. They will move for a product that answers `20 eur dzd` with
*both* rates, translates without sending the text anywhere, and knows what a qintar is. The tools
are the reason to exist; the search is the foundation under them.

---

## M1B-T01 — Frontend foundation

- [x] M1B-T01.1 Next.js 16.3 + React 19.2 + TypeScript + Tailwind v4 in `web/`. **npm, not
      pnpm** — enabling corepack needs root on this machine and the package manager is not worth
      a privileged install
- [x] M1B-T01.2 `[lang]` routing for ar / ary / fr / en; direction and theme resolved server-side
- [ ] M1B-T01.3 Typed API client generated from an OpenAPI description of [[API Contract]]
- [x] M1B-T01.4 Theme: light / dark / system, cookie-resolved, resolved before the first byte
- [x] M1B-T01.5 Bundle budgets enforced by `scripts/bundle-budget.sh`, measured as a browser
      fetches them. Exceeding fails. It caught the framework cost immediately
- [x] M1B-T01.6 `scripts/no-js-check.sh` — five assertions over the HTML `curl` receives, which
      is exactly what a reader without JavaScript gets. Search, filtering and pagination all pass

## M1B-T02 — Design language

> The first palette — warm manuscript paper and blue-hour indigo — was rejected as insufficiently
> polished. Replaced by an editorial, near-neutral, essentially square language; see
> [[UI - Design Language]] §2.


- [x] M1B-T02.1 `oklch` token set, both themes ([[UI - Design Language]] §3)
- [ ] M1B-T02.2 IBM Plex Sans + Sans Arabic, self-hosted, subset per script, preloaded per
      direction
- [ ] M1B-T02.3 shadcn primitives copied in and **rewritten** — not themed
- [x] M1B-T02.4 The qalam rule as a shared primitive, used only by summary and tool cards
- [~] M1B-T02.5 Density tokens exist and are cookie-driven; the toggle control is not built
- [ ] M1B-T02.6 Contrast audit of both themes at AA ([[UI - Accessibility]])
- [ ] M1B-T02.7 Native-speaker read on the Arabic face and numeral system ← *B7*

## M1B-T03 — Port the existing UI

Ported, then the Rust renderer **deleted**. Two renderers is the problem being solved.

- [x] M1B-T03.1 Home
- [x] M1B-T03.2 Results — Server Component, HTML in the first response
- [x] M1B-T03.3 Filters as server-rendered links
- [x] M1B-T03.4 Suggestions as an ARIA combobox
- [x] M1B-T03.5 Summary, fetched after paint
- [~] M1B-T03.6 Error and zero-result states are built; offline and degraded are not
- [x] M1B-T03.7 **Deleted `web.rs` and the legacy assets.** The API is JSON and operations only.
      `/admin` stays on it with its CSS and JS embedded in the binary — an operator tool has to
      work when the frontend is the thing that is down

## M1B-T04 — Tool framework

- [x] M1B-T04.1 `Tool` trait: pure, total; a panicking matcher is caught and skipped
- [x] M1B-T04.2 Arbitration by confidence; below 0.5 renders nothing. Confidence reflects how
      much of the query a tool explains, which is what makes the converter beat the calculator on
      `5 km to miles`
- [x] M1B-T04.3 `instant` in the search response ([[Instant Answers]] §3)
- [x] M1B-T04.4 Explicit invocation (`!calc`, `!convert`)
- [x] M1B-T04.5 Shared card frame incl. interpretation line and copy. Server-rendered; only the
      copy button hydrates
- [x] M1B-T04.6 **Matcher precision corpus**: ten ordinary queries that must match no tool, plus
      per-tool prose cases. Ten is a start, not a corpus
- [ ] M1B-T04.7 Per-tool opt-out, persisted

## M1B-T05 — Tier 1 tools

- [~] M1B-T05.1 Calculator — decimal, not binary float; precedence, percentages, functions,
      Arabic-Indic digits, depth and length guards. **~40 golden expressions, not several
      hundred**
- [~] M1B-T05.2 Unit converter incl. qintar and sa'a; 7 dimensions, ar/fr/en phrasings,
      temperature by offset. **Unit names render in English in every locale** — the table has one
      canonical name per unit and no translations
- [ ] M1B-T05.3 Currency — official **and** parallel, side by side, each with `as_of`
- [ ] M1B-T05.4 Translator on the existing local Qwen; streaming, cancellable, nothing leaves
      the machine
- [~] M1B-T05.5 Weather — current conditions and five days for all 58 wilayas, served from
      cache. **Custom line icons are not drawn**; the card carries the WMO code and the client
      maps it
- [x] M1B-T05.6 Prayer times — computed from coordinates and date, no network, nothing that can
      go stale. Three methods and both Asr rules; the method is on the card because Algerian
      mosques do not all follow one authority and an unnamed reckoning makes an ordinary
      disagreement look like a defect
- [~] M1B-T05.7 Hijri ↔ Gregorian and days-between are built, computed locally so they cannot go
      stale. Uses the **tabular** calendar; Algeria announces Eid by sighting and can differ by a
      day, which the card discloses rather than hides. World clock is not built

> **Where this milestone stands.** The framework, calculator, converter and date tools are built
> and localised. Currency, weather, prayer times and translation are **not**, and each is blocked
> on something real rather than on effort: currency and weather need [[Tool Data Plane]] plus the
> unresolved parallel-rate sourcing question (§6 below), and translation needs the summariser's
> model wired to a second call path. None of them is worth half-building — a currency card that
> invents a rate is exactly the failure [[Instant Answers]] §2 exists to prevent.

## M1B-T06 — Tool data plane

- [x] M1B-T06.1 `xustive-toold` on `ingest` and `core`. Joining `core` grants no egress —
      `core` is internal — it grants reachability to Redis. This process is the bridge and is the
      only thing that should be, which is why it takes no user input at all
- [x] M1B-T06.2 Fixed cadence, never per-request. 58 wilayas every 30 minutes is 116 requests
      an hour, identical whether one person searched or a million did — which is what makes a
      weather search reveal nothing
- [x] M1B-T06.3 Validation: bounds, movement guard, timestamp sanity, misaligned series. A
      rejection keeps the previous value rather than clearing it
- [x] M1B-T06.4 `observed_at` distinct from `fetched_at` throughout, and the card's age uses
      the publisher's measurement
- [x] M1B-T06.5 `xustive_data_age_seconds{dataset}` gauge, sampled on a timer rather than on
      request — a fetcher that stops silently leaves the last values in place, so traffic-driven
      sampling would report a healthy number for a dead fetcher. Reports the **oldest** entry, not
      the mean: one stuck wilaya out of 58 would move an average by two per cent and fire nothing.
      Four alerts, unit-tested with `promtool test rules` so a threshold typo is caught here rather
      than during the incident
- [x] M1B-T06.6 Egress test re-run with `toold` in the topology: the serving plane still cannot
      reach the internet
- [ ] M1B-T06.7 **Resolve parallel-rate sourcing.** If no honest source exists, the parallel rate
      ships disabled rather than invented

## M1B-T07 — Tier 2 and 3 tools

- [x] M1B-T07.1 Wilaya reference — all 58, compiled in, with seat coordinates that prayer times
      and later weather use to turn a named place into a location without ever asking the browser
- [ ] M1B-T07.2 Fuel prices
- [ ] M1B-T07.3 Dictionary — ar / fr / en, Darija marked as incomplete ← *B7*
- [x] M1B-T07.4 Transliterator, surfacing [[Query Expander]] — offered on an explicit request only,
      never applied silently: Arabizi is ambiguous, so the card shows a second reading alongside
      the first rather than presenting one guess as settled
- [ ] M1B-T07.5 Sports fixtures and results
- [ ] M1B-T07.6 Exam results — links to official portals only, never mirrored
- [~] M1B-T07.7 Utility tools ([[Instant Answers]] §5.3) — 13 built: Base64, URL encoding, TVA at
      19 %/9 %, percentage change, Roman numerals, hex→RGB, case conversion, word and character
      counts, SHA-256, JSON formatter, tip split, loan repayment, BMI. All pure, offline and
      deterministic. **Still open:** QR code (needs an encoder and an SVG renderer), timer and
      stopwatch (a client component, not a matcher), hsl/oklch conversion. Random numbers, dice and
      coin flips are now *excluded* rather than pending — see [[Instant Answers]] §5.3

## M1B-T08 — Localisation

- [x] M1B-T08.1 Catalogues for ar / fr / en; a missing key is a compile error
- [x] M1B-T08.2 `Intl.PluralRules` — Arabic has six categories and a ternary is wrong for four
- [x] M1B-T08.3 `Intl.NumberFormat` / `DateTimeFormat` in one module, numeral system an explicit
      constant. `Intl` defaults Arabic to Eastern digits; Algerian print uses Western, so the
      locale default would be wrong for this audience
- [~] M1B-T08.4 Darija falls back to Arabic, never English. A distinct catalogue still needs a
      native speaker ← *B7*
- [ ] M1B-T08.5 Visual regression: 4 languages × 2 directions × 2 themes
- [ ] M1B-T08.6 `<bdi>` on numbers, expressions and URLs inside RTL text

---

## Exit gate

1. The Rust HTML renderer is deleted and nothing regressed.
2. JavaScript disabled: search, filter and paginate all work.
3. Bundle budgets pass in CI.
4. Calculator golden set passes 100 %.
5. Matcher precision corpus: **zero** false tool activations.
6. No stale value is ever rendered without its age.
7. `xustive-api` still has no internet egress.
8. Both themes pass AA contrast in all four languages.

## Risks

| Risk | Mitigation |
|:---|:---|
| A tool is confidently wrong | Golden sets, plausibility bounds, and the rule that a tool renders nothing rather than guessing |
| Parallel rate cannot be sourced honestly | Ships disabled. The feature is not worth inventing a number for |
| Next.js bundle creep | Budgets fail the build rather than warn |
| Tools push results down and annoy people | Precision corpus, 0.5 confidence floor, per-tool opt-out |
| Translation is too slow to be useful on CPU | Already true of the summariser. GPU (blocker B6) is the fix; the card streams and can be cancelled meanwhile |
| Two renderers coexist "temporarily" | T03.7 deletes the old one in the same milestone, not later |

## Related

[[ADR-0010 - Next.js for the Frontend]] · [[UI - Frontend Architecture]] · [[UI - Design Language]] ·
[[UI - Tool Cards]] · [[Instant Answers]] · [[Tool Data Plane]] · [[Milestone 1 - Text Search MVP]] ·
[[TODO]]
