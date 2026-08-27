---
tags:
  - planning
  - milestone
milestone: 11
status: in-progress
updated: 2026-08-28
progress: built 2026-08-27; open — T02.3 (counters on the documents page), a scheduler for the sweep
---
# Milestone 11 - Learning from Readers

> **Goal:** the product remembers what was searched, what was shown, what was opened and what
> was reported wrong — per event, durably, with a first-party visitor id — shows it to the
> operator in a form that says what to fix, and lets readers say *not relevant* from any result.
> **Exit gate:** a search, a click and a report each land as one event with the right fields;
> the opened document carries `hits.opens`/`hits.reports`; the admin page answers "what did
> people search, what did they open, what did they reject, what got nothing"; the privacy page
> says what is kept and how to be forgotten; `xustive events forget` removes a visitor's events;
> the telemetry lint still passes (no query in any log line).
> Parent: [[TODO]] · Decided by [[ADR-0030 - First-Party Search Data, Kept to Learn From]] ·
> Previous: [[Milestone 10 - Reverse Image Search]] · Components: [[Search Events]],
> [[Interaction Signals]], [[UI - Admin Console]], [[UI - Results Page]]

## Why This Milestone Exists

Read [[ADR-0030 - First-Party Search Data, Kept to Learn From]] §Context. In one line: every
improvement worth making next needs the raw events, and the product keeps none.

## The shape of the thing

**One index, three event kinds, one document counter.** `events` in Meilisearch: `search`,
`click`, `report`, each with visitor, session, time, query, and the fields its kind needs.
Documents gain `hits`. The token the search response already mints for the anonymous click
beacon is reused: the click and the report carry the token, and the server resolves it to the
query it kept in memory — the browser never sends query text on a click.

**The visitor is a cookie we set.** `xv` (a ULID, one year) and `xs` (session). The page's own
script sets them on first paint; the server reads them from the request. Nothing about them
reaches a third party.

**The admin page says what to do.** Not a dashboard of totals: the searches that got nothing,
the searches that got results and no click, the results readers reported, the documents most
opened — each a list an operator can act on, with the raw recent events underneath for the
cases that need a look.

---

## M11-T01 — The events store

- [x] M11-T01.1 `events` index: `id` (ULID), `kind`, `at` (unix s), `visitor`, `session`,
      `query`, `normalized`, `ui`, `lang`, `vertical`, `page`, `total_hits`, `shown[]`
      (document ids in rank order), `doc`, `rank`, `reason`, `latency_ms`. Filterable on
      `kind, at, visitor, session, doc, vertical, ui, total_hits, reason`; sortable on `at`;
      searchable on `query`. In `xustive migrate`
- [x] M11-T01.2 `[collection]`: `enabled` (default false; dev true), `retention_days` (365)
- [x] M11-T01.3 The search handler writes a `search` event after the response is built (never on
      the critical path: fire-and-forget into a bounded channel drained by one writer task, so a
      slow index cannot slow a search). Visitor/session from the `xv`/`xs` cookies
- [x] M11-T01.4 The token store keeps the query text beside the hash while collection is on, so
      `POST /interaction` writes a `click` event with the query, the doc and the rank
- [x] M11-T01.5 `POST /api/v1/report` — `{t, d, r}` — writes a `report` event; always 204
- [x] M11-T01.6 `xustive events sweep` deletes events older than `retention_days`;
      `xustive events forget <visitor>` deletes a visitor's events and answers with the count

## M11-T02 — Documents remember

- [x] M11-T02.1 `Document.hits { opens, reports, last_opened_at }`; filterable/sortable on
      `hits.opens` and `hits.reports`
- [x] M11-T02.2 A click increments `opens` and stamps `last_opened_at`; a report increments
      `reports`. Read-modify-write through the same writer task, batched. *Settled: the
      counters are partial updates the index merges; a re-crawl that rewrites the document
      resets them, and `xustive events rebuild-hits` recomputes them from the events. Noted on
      the dev box: single-document updates queue behind the crawler's batches in Meilisearch and
      can take minutes to apply — the events themselves do too. Correct, not instant*
- [ ] M11-T02.3 The documents admin page shows the counters and can sort by them — *open; the counters are in the index (`hits`, filterable and sortable), the page does not show them yet*

## M11-T03 — Readers can say no

- [x] M11-T03.1 A quiet *Not relevant* control on every result card (all four languages); one tap,
      then *Thanks* — no dialog, no reason list yet
- [x] M11-T03.2 Beacons `{t, d, r: "irrelevant"}` to `/api/v1/report`; works with the click
      beacon's token; absent when collection is off
- [x] M11-T03.3 The control is a `<button>` with an accessible name, not a link; keyboard-reachable

## M11-T04 — The admin page

- [x] M11-T04.1 `GET /api/v1/admin/events/overview?days=` — searches, clicks, reports, CTR,
      zero-result rate; **zero-result queries** (top, with counts); **searched, never opened**
      (queries with results and no click, top); **reported results** (doc, title, query, count);
      **most opened** (doc, title, opens, reports); recent events (last 100)
- [x] M11-T04.2 `/admin/searches`: the five lists above as tables with the action each implies
      (add a synonym, check the ranking, re-crawl, look at the page), a day-range picker, and
      the raw recent events; the sidebar entry
- [x] M11-T04.3 A visitor lookup: every event of one visitor id, and a *Forget* button that
      calls the deletion — the rights of ADR-0030 §6, operable

## M11-T05 — Policy and gates

- [x] M11-T05.1 The privacy page, four languages: what is kept (searches, results shown,
      results opened, reports, a visitor cookie), for what (improving and personalising search,
      our own AI), for how long, never shared, how to be forgotten; and the ADR-0029 sentence
      that queries may be processed by third parties without identity and may leak. The home
      page line *Searches are never linked to you* goes
- [x] M11-T05.2 README §Guarantees rewritten to what the build enforces now
- [x] M11-T05.3 Tests: an event per action with the right fields; the click beacon still carries
      no query text; `lint-telemetry` green; `events forget` removes everything for a visitor
- [x] M11-T05.4 [[Search Events]] component note; [[Interaction Signals]], [[Security and
      Privacy]], [[Legal and Compliance]] §5, [[API Contract]], [[UI - Admin Console]],
      [[UI - Results Page]] updated

## Deliberately not in this milestone

- Using the data: no ranking change, no autocomplete from history, no synonym mining, no
  personalisation. Each is a later task with this milestone's data as input.
- Accounts. The visitor id is a cookie; accounts bind to it when they come.
- Reasons for a report beyond *not relevant*.

---

> **Status 2026-08-28.** Built and verified end to end on the dev box: a search from a browser
> with the visitor cookie, its "not relevant" report from the card ("Thanks, noted"), and its
> click each landed as one event with the query, the rank (1), the 20 ids shown and the 118
> hits; `/admin/searches` lists the reported page under the query that produced it and shows
> the raw events; `xustive events forget <visitor>` deleted the visitor's six events. Open:
> the documents page does not yet show `hits`, nothing schedules the sweep, and a public
> deployment must not turn `[collection]` on before [[Legal and Compliance]] §5 is settled.
