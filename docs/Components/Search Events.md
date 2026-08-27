---
tags:
  - component
  - data
type: component
status: built
updated: 2026-08-27
---

# Search Events

> First-party search data ([[ADR-0030 - First-Party Search Data, Kept to Learn From]],
> [[Milestone 11 - Learning from Readers]]). Code: `crates/xustive-api/src/events.rs` (the
> sink, the report beacon, the admin endpoints), `crates/xustive-search/src/events.rs` (sweep and
> forget), `crates/xustive-search/src/settings.rs` (`events_settings`), `web/components/search/
> Visitor.tsx`, `InteractionBeacon.tsx`, `ResultCard.tsx`, `web/app/(operator)/admin/searches`.
> Related: [[Interaction Signals]] (the anonymous, derived counters beside this),
> [[Search Index]], [[UI - Admin Console]], [[Security and Privacy]], [[Legal and Compliance]].

## What it keeps

One document per event in the `events` index, three kinds:

| kind | fields |
|:---|:---|
| `search` | `at`, `visitor`, `session`, `query` (as typed), `normalized`, `ui`, `lang`, `vertical`, `page`, `total_hits`, `shown[]` (ids in rank order), `latency_ms` |
| `click` | `at`, `visitor`, `session`, `query`, `doc`, `rank` |
| `report` | the click's fields plus `reason` (`irrelevant`) |

And on each document, `hits { opens, reports, last_opened_at }`, filterable and sortable.

Never in an event: the IP address, the user agent, the device, a precise location. The query
text is in the *store* — that is the point — and still never in a log line
(`scripts/lint-telemetry.sh`).

## How it flows

- **Search.** The handler builds the response, then hands the sink a `search` event with the
  ids it showed; the sink's bounded channel (4096) and one writer task batch it into
  Meilisearch (200 or 2 s). A full channel drops the event and counts it (`dropped` on the admin
  page). The query and the shown ids are also kept in memory beside the search's token
  (`AppState::token_context`, ≤ 4096 entries).
- **Click.** The existing beacon (`{t, d}`) hits `POST /interaction`; the server resolves the
  token to the query and the rank and writes a `click` event, then the anonymous store records
  its k-anonymous click as before. The browser never sends the query.
- **Report.** *Not relevant* on a result card is a `<button data-report=id>`; the same delegated
  listener beacons `{t, d, r: "irrelevant"}` to `POST /report`, the button says thanks and takes
  no second press. Same token, same resolution.
- **Counters.** The writer reads the batch's documents once (`id IN [...]`), adds, and writes
  `hits` back as a partial update; the index merges it. A re-crawl that rewrites the document
  resets the counters — the events are the truth, and `xustive events rebuild-hits` recomputes
  every document's counters from them.
- **Visitor.** `web/components/search/Visitor.tsx` sets `xv` (a ULID, one year) and `xs`
  (session) on the results page when a token exists; the API reads them from the request. The
  first search of a new browser has no visitor id; every one after does.

## Operating it

- `[collection] enabled` (default false; dev true), `retention_days` (365).
- `xustive events sweep` — delete events older than the retention;
  `xustive events forget <visitor>` — delete one visitor's events and print the count;
  `xustive events rebuild-hits` — recompute the documents' counters.
- `GET /api/v1/admin/events/overview?days=` — totals, zero-result queries, searched-never-opened,
  reported results, most opened, top queries, the last hundred events;
  `GET /admin/events/visitor?visitor=`; `POST /admin/events/forget {visitor}`. The page is
  `/admin/searches`.
- Retention is the operator's duty until a scheduler runs the sweep; one search event is about
  1 KB.

## What it must never do

Carry an address. Log a query. Be sent anywhere. Feed a ranking decision by itself — a learner
reads it, and that learner gets its own ADR.
