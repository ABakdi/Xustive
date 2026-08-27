---
tags:
  - planning
  - milestone
status: done
updated: 2026-08-27
closed: 2026-08-16
---

# Milestone 1B — Frontend and Instant Answers

> Sits between [[Milestone 1 - Text Search MVP]] and [[Milestone 3 - Multimodal Input]]. Numbered
> 1B rather than appended to M1 because M1's remaining work — the task queue, the indexer worker —
> is backend and can proceed in parallel with none of this.

> **Closed — recorded by the 2026-08-27 audit.** The header said `planned`; the last 1B commit
> is `7616086` (2026-08-15, contrast audit) and the offline/degraded states landed 2026-08-16.
> Against the exit gate: the Rust renderer is deleted (T03.7); `no-js-check.sh`, the bundle
> budget and the contrast audit run in `make ui-gates`; the matcher precision corpus exists
> (T04.6, ten queries); the calculator golden set passes (re-asserted under `fend-core` in
> M8-T07.5); `test-egress.sh` is green with `toold` in the topology. The tools this note listed
> as *not built* — currency, the weather forecast, the translator — were finished later:
> currency and weather in [[Milestone 8 - The Answer Layer]] (M8-T05/T06), the translator
> registered on 2026-08-20 (`3afd301`). Still open: the typed OpenAPI client (T01.3), the
> native-speaker items (T02.7, T07.3, T08.4 — B7), visual regression (T08.5), sports (T07.5),
> and the remaining utilities (T07.7).

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
- [ ] M1B-T01.3 Typed API client generated from an OpenAPI description of [[API Contract]] —
      *audit 2026-08-27: `web/lib/api.ts` is typed but hand-written; there is no OpenAPI
      description and nothing is generated. BUG-028 (a TS field the API never sends) is the
      cost of that*
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
- [x] M1B-T02.2 IBM Plex Sans + Sans Arabic, self-hosted, subset per script, preloaded per
      direction — 4 faces, 172 KB committed; 86 KB fetched on an RTL page, 44 KB on an LTR one.
      Latin is one variable file for 400–600; the Arabic family is not variable on Google Fonts,
      which a range request revealed only by returning an HTML error page and silently producing
      no Arabic at all. `fetch-fonts.sh` now fails if a family is missing or two faces collide on
      one filename, both of which it did on the way here
- [x] M1B-T02.3 shadcn primitives copied in and **rewritten** — not themed. Button, Select and
      Toggle, taking shadcn's structure (variants, `focus-visible`, state never on colour alone)
      and none of its code. The reason is not styling: shadcn's primitives are Radix components,
      Radix components are client components, and adopting them would push `'use client'` into the
      result page. Zero dependencies, zero client components, and `no-js-check.sh` now fails if a
      primitive ever declares one. Bundle unchanged at 175/176 KB
- [x] M1B-T02.4 The qalam rule as a shared primitive, used only by summary and tool cards
- [x] M1B-T02.5 Density tokens are cookie-driven **and the toggle is built**. Mirrors
      `ThemeToggle` exactly — same shape, same optimistic write, same `aria-label` naming the
      current state rather than the action, because two adjacent controls that behave differently
      is worse than either behaviour alone. Compact is not cosmetic here: Arabic sets taller than
      Latin at the same point size, so a list that fits one screen in French runs onto two in
      Arabic, and much of the traffic is on small phones
- [x] M1B-T02.6 Contrast audit of both themes at AA. `scripts/contrast-audit.mjs` reads the oklch
      tokens from globals.css, converts to linear sRGB and checks every text/control pair against
      4.5:1 (body) or 3:1 (large/control edges) in both themes; in `make ui-gates`. Caught two real
      failures — `--fg-faint` at 2.78:1 on rendered hint text and `--line-strong` at 1.8:1 on the
      search-box border — both fixed
- [ ] M1B-T02.7 Native-speaker read on the Arabic face and numeral system ← *B7*

## M1B-T03 — Port the existing UI

Ported, then the Rust renderer **deleted**. Two renderers is the problem being solved.

- [x] M1B-T03.1 Home
- [x] M1B-T03.2 Results — Server Component, HTML in the first response
- [x] M1B-T03.3 Filters as server-rendered links
- [x] M1B-T03.4 Suggestions as an ARIA combobox
- [x] M1B-T03.5 Summary, fetched after paint
- [x] M1B-T03.6 Error, zero-result, **offline and degraded** states all built. Offline is a client
      component (the server that would render an error is the unreachable thing) — a banner over the
      page that keeps the query and confirms recovery. Degraded: the API drops facets under load, and
      a `facets_degraded` flag now lets the page say filtering is temporarily unavailable instead of
      the filter row vanishing silently
- [x] M1B-T03.7 **Deleted `web.rs` and the legacy assets.** The API is JSON and operations only.
      `/admin` stays on it with its CSS and JS embedded in the binary — an operator tool has to
      work when the frontend is the thing that is down. *Settled differently on 2026-08-19
      (`f561482`, `1d097db`):* the admin console and `/bot` moved into the Next.js app under an
      `(operator)` route group and the API became JSON-only under `/api/v1/admin`. The
      "works when the frontend is down" argument lost to having one renderer

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
- [x] M1B-T04.7 Per-tool opt-out, persisted — dismissable from the card and reversible from a
      settings page, both real forms posting Server Actions so they work without JavaScript. The
      preference is a cookie the **Next layer** applies; it is deliberately never sent to the API,
      because the set of tools someone has switched off is stable and unusual enough to identify
      them, and a privacy control that makes you more identifiable is the wrong shape. `/tools`
      serves the inventory so the settings page cannot drift from what the engine runs

## M1B-T05 — Tier 1 tools

- [~] M1B-T05.1 Calculator — decimal, not binary float; precedence, percentages, functions,
      Arabic-Indic digits, depth and length guards. **~40 golden expressions, not several
      hundred**
- [x] M1B-T05.2 Unit converter incl. qintar and sa'a; 7 dimensions, ar/fr/en phrasings,
      temperature by offset. Unit names are localised — every entry carries `ar` and `fr` names and
      Darija falls back to Arabic, never English. Asserted across the whole table, since the table
      is the kind of list people append to and an entry added with only its English name degrades
      silently. The Arabic check tests for Arabic *script*: an English word in the `ar` slot passes
      a non-empty check while still answering the wrong language
- [x] M1B-T05.3 Currency — official **and** parallel, side by side, each with `as_of`.
      *Settled 2026-08-26 (M8-T06):* `currency.rs` + a `rates` dataset in `xustive-toold`;
      the **official** rate only, dated from the publisher, withheld when stale. The parallel
      rate ships disabled for want of a nameable source (T06.7)
- [~] M1B-T05.4 Translator on the existing local Qwen; streaming, cancellable, nothing leaves —
      **built but not enabled.** The engine now streams token by token and a dropped connection
      stops the model worker on its next token (verified: the slot is freed and counted). The
      endpoint, the detector, the language plumbing and the streaming card all work. What does not
      is the model's output *into* Arabic: `أين closest الصيدلية؟`, `أين تقع الم下车؟` — a semantic
      equivalent from another language substituted mid-sentence. Arabic as a **source** is correct.
      Sampling, the repetition penalty, an Arabic-language instruction, a worked example and the
      prompt delimiter were each ruled out by measurement; the summariser produces fluent Arabic
      through the same engine, because its context is full of Arabic passages and a translation
      prompt cannot supply that. **Blocked on a better model for this task**, not on more code
      the machine. *Update 2026-08-20 (`3afd301`, `3378905`):* the tool is now **registered** —
      a bare `translate` / `traduire` / `ترجم` verb opens its editable card — and stray CJK/kana
      characters are stripped from quantised-model output. The Arabic-target quality caveat
      stands (`xustive-tools/src/lib.rs` says so at the registration site)
- [x] M1B-T05.5 Weather — current conditions and five days for all 58 wilayas, served from
      cache. **Custom line icons are not drawn**; the card carries the WMO code and the client
      maps it. *Settled 2026-08-26 (M8-T05):* the forecast is drawn (today, day strip, week
      toggle, SVG graphs), the fetcher covers 48 h hourly and seven days, `طقس` with no place
      resolves through a local database, and the per-WMO-code line icons exist (M8-T05.7)
- [x] M1B-T05.6 Prayer times — computed from coordinates and date, no network, nothing that can
      go stale. Three methods and both Asr rules; the method is on the card because Algerian
      mosques do not all follow one authority and an unnamed reckoning makes an ordinary
      disagreement look like a defect
- [~] M1B-T05.7 Hijri ↔ Gregorian and days-between are built, computed locally so they cannot go
      stale. Uses the **tabular** calendar; Algeria announces Eid by sighting and can differ by a
      day, which the card discloses rather than hides. World clock is not built

> **Where this milestone stands** *(written 2026-08-08; superseded — see the audit note at the
> top: currency, weather and translation have since shipped, prayer times were already ticked
> below)*. The framework, calculator, converter and date tools are built
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
- [x] M1B-T06.7 **Resolve parallel-rate sourcing.** If no honest source exists, the parallel rate
      ships disabled rather than invented. *Resolved 2026-08-26 (M8-T06.4):* none exists — the
      square-market rate is quoted by no publisher we can verify — so it ships **disabled**, and
      the card says so. Reopens only if a nameable source appears

## M1B-T07 — Tier 2 and 3 tools

- [x] M1B-T07.1 Wilaya reference — all 58, compiled in, with seat coordinates that prayer times
      and later weather use to turn a named place into a location without ever asking the browser
- [x] M1B-T07.2 Fuel prices — compiled in rather than fetched. These are **administered** prices
      set by the ARH, uniform nationally, unchanged since 2020 until 1 January 2026; neither the
      ARH nor Naftal publishes a feed, so there is nothing to poll. The card shows the authority
      and effective date and carries no `as_of`, since an administered price is not measured. The
      January 2026 change was applied at midnight with no announcement, so the table will
      eventually be wrong silently — a test fails once its review date passes, turning that into a
      broken build rather than a wrong answer
- [ ] M1B-T07.3 Dictionary — ar / fr / en, Darija marked as incomplete ← *B7*
- [x] M1B-T07.4 Transliterator, surfacing [[Query Expander]] — offered on an explicit request only,
      never applied silently: Arabizi is ambiguous, so the card shows a second reading alongside
      the first rather than presenting one guess as settled
- [ ] M1B-T07.5 Sports fixtures and results
- [x] M1B-T07.6 Exam results — links to official ONEC portals only, never mirrored. Recognises
      BAC / BEM / cinquième results queries in ar/fr/en; requires a results word so the bare exam
      name does not trigger; renders the portal as a link (`rel=noopener`, no referrer) gated on an
      `official` flag. It never fetches, stores or shows a result — the restraint is the feature
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
- [~] M1B-T08.4 Darija falls back to Arabic, never English, **and now has its own catalogue** —
      32 keys overridden. Only the strings a person would say differently: Darija has no settled
      written standard, so invented spellings for institutional vocabulary (`الإعدادات`,
      `الولاية`) read more slowly than the MSA every Algerian knows from forms and bulletins, and
      those rows keep the Arabic wording deliberately. What changes is the conversational register.
      A test asserts it is not an alias, by counting overrides rather than pinning strings — pinning
      would break whenever a reviewer improves one, which is the edit most worth encouraging.
      ← *machine-generated, spelling wants a native speaker*; B7
- [ ] M1B-T08.5 Visual regression: 4 languages × 2 directions × 2 themes — *not built (audit
      2026-08-27; same gap as M1-T14.6)*
- [x] M1B-T08.6 `<bdi>` on numbers, expressions and URLs inside RTL text — verified in a browser
      against the Arabic locale, and enforced by `scripts/lint-bidi.sh`. Brackets were the real
      hazard: they are neutral on both sides, so an unisolated `(104 ms)` renders with the pair
      swapped and reads as a typo rather than a rendering bug

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
