---
tags:
  - planning
  - milestone
milestone: 12
status: in-progress
updated: 2026-08-28
---
# Milestone 12 - The Operator's Console

> **Goal:** the admin console reads the system *and steers it*, with time on the x-axis. Every
> page that shows a thing offers the control that changes it where one exists; every number that
> moves has a line behind it; the operator gets from anywhere to anything in two keystrokes.
> **Exit gate:** the overview answers "is anything wrong, and since when" with charts, not
> tiles; ranking weights, federation budgets, collection and interaction switches are editable
> from the console with validation and persist across a restart; no admin page is display-only
> where the backend has a control; a command palette reaches every page and action; the console
> stays inside a measured bundle budget; the docs describe the console that exists.
> Parent: [[TODO]] · Previous: [[Milestone 11 - Learning from Readers]] · Components:
> [[UI - Admin Console]], [[Search Events]], [[Observability]]

## Why This Milestone Exists

An inventory of the console on 2026-08-28: sixteen pages, six of them display-only (Overview,
Discovery, Evaluation, Interaction, Media, Configuration); no page has a time axis — the
metrics registry keeps only current values, Grafana's provisioning directory is empty, and the
one dated series in the repo (the eval reports) is rendered as a table; ranking weights, every
budget and every `[section]` flag are read once at startup and shown read-only; three copies of
a `Tile` component, two of a status dot, and a `--ok` token that does not exist; and
[[UI - Admin Console]] still says the console is server-rendered by the Rust API. An operator can
see that the frontier is at 12,000 but not whether that is up or down, and can see the ranking
weights but not touch them.

## The shape of the thing

**Time, sampled by the API itself.** A 30-second ring of the vitals — searches, zero results,
p95 latency for the interval, summaries, crawl fetches and indexings, queue depth, events —
kept for 24 hours in the process and served as `GET /admin/timeseries`. No dependency on
Prometheus for the page to draw a line; Prometheus keeps the long memory for those who run it.

**Charts by the method, not by taste.** A small SVG kit, no library: stat tiles with deltas
and sparklines, line charts with a crosshair and one tooltip for every series, thin horizontal
bars with the value at the tip, a meter for a ratio against a limit; a table twin behind a
toggle on every chart; a validated palette for both themes (the brand indigo first). One axis,
ever.

**Controls where the state is.** A `[runtime]` overlay the API can write: ranking weights,
federation budgets, the collection and interaction switches, summaries on/off — validated
through the same `Config::validate` a restart would run, applied at once, and persisted to
`config/runtime.toml` so a restart keeps them. Existing switches (pause, politeness, device, log
level, integrations) gather where their subject is.

**Navigation that costs nothing.** A command palette (⌘K / Ctrl-K) over pages and actions; the
sidebar carries each page's status dot; the overview links every page (today it omits four).

---

## M12-T01 — Vitals and the chart kit

- [x] M12-T01.1 `timeseries.rs`: a 30 s sampler over the metrics registry, the crawl snapshot and
      the events sink; interval p95 from bucket differences; `GET /admin/timeseries?hours=`
- [x] M12-T01.2 `components/admin/charts.tsx`: `StatTile`, `Sparkline`, `LineChart`, `Bars`,
      `Meter`, `compact()`; `--viz-*` roles in both themes, validated (adjacent CVD ΔE ≥ 8;
      light aqua/yellow under 3:1 → always direct-labelled or tabled)
- [ ] M12-T01.3 The overview rebuilt: a hero (documents in the index), tiles with sparklines,
      three lines — searches and zero results, p95 latency, crawl and queue — a status strip with
      icon + colour, quick actions, and every page linked
- [ ] M12-T01.4 The kit consolidated: one `Tile`, one `Status` dot, one `Section`, `Toggle`,
      `NumberField`; the three inline copies removed; `--ok` replaced by `--viz-good`

## M12-T02 — Runtime settings that persist

- [ ] M12-T02.1 `RuntimeSettings` on the state: ranking weights (`Arc<RwLock<Weights>>`),
      federation `budget_ms`/`fetch_budget_ms`/`max_hits`/`eager_index`, `collection.enabled`,
      `interaction.enabled`, `ml.summaries_enabled` — read per request
- [ ] M12-T02.2 `PATCH /admin/settings` with a whitelist and bounds; validation through
      `Config::validate`; the change logged with the operator's peer and the old and new values
- [ ] M12-T02.3 `config/runtime.toml` written on every accepted change and merged at startup
      after the environment, so the console's changes survive a restart and are visible in
      `/admin/config` as "runtime override"
- [ ] M12-T02.4 The ranking editor: ten sliders with the relevance gap rule enforced (side
      weights must stay under the gap), a preview query, Apply and Revert
- [ ] M12-T02.5 Federation budgets, collection, interaction and summaries on their pages

## M12-T03 — Every page gets its time and its control

- [ ] M12-T03.1 Searches: per-day series from the events (searches, CTR, zero-result rate,
      reports) behind the tiles; the lists become bars where a bar reads faster
- [ ] M12-T03.2 Evaluation: nDCG@10 / MRR / recall over the dated reports as a line; a *Run
      eval* action that starts `xustive eval` and streams its result
- [ ] M12-T03.3 Queue: depth over time from the ring; Live: a fetch-rate sparkline from the SSE
      frames the page already receives
- [ ] M12-T03.4 Discovery and Interaction: bars; Media: the switches it says are config-only
      become the runtime ones of T02 where they can (summaries) and say "restart" where they
      cannot (sidecars); Compute: a VRAM meter
- [ ] M12-T03.5 Documents: `hits` columns and sort (M11-T02.3), composition as bars

## M12-T04 — Navigation

- [ ] M12-T04.1 A command palette: pages, actions (pause, toggle federation, raise logs, replay
      DLQ…), and a document/query jump; ⌘K, `/`, Esc
- [ ] M12-T04.2 Sidebar status dots from the overview's subsystem states; the active page's
      section expanded
- [ ] M12-T04.3 `scripts/bundle-budget.sh` measures `/admin` too, with its own budget

## M12-T05 — Docs

- [ ] M12-T05.1 [[UI - Admin Console]] rewritten for the console that exists (Next.js, polling,
      the kit, the charts, the runtime settings); PROB-003 "not built" list updated

## Deliberately not in this milestone

- Replacing Grafana. The ring is 24 hours; alerting and long retention stay in Prometheus.
- Editing arbitrary config. The whitelist is what an operator tunes; the rest is a file and a
  restart, on purpose.
- Multi-user admin, roles, audit beyond the log line.
