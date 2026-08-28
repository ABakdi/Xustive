---
tags:
  - solution
  - admin
  - observability
problem: PROB-003
date: 2026-08-26
status: solved
---
# PROB-003 Solution — Admin Console Coverage

> Problem: [[Problems#PROB-003 — The admin console exposes a fraction of what tunes, measures, and controls the system|Problems register → PROB-003]] ([../Problems.md](../Problems.md))
> Related: [[PROB-001 - Bounded Frontier and Queue]] — the capacity alarm here is the admin half
> of its never-again guarantee; [[PROB-002 - Crawl and Index Throughput]] — the pause control and
> registry policy editor operate the levers it documented.
> Outcome: **the six ranked recommendations are closed** — the fifteen dropped fields render, the
> effective configuration is one page, the capacity alarm is on Overview and Queue, the evaluation
> trail has a page, and the control gaps are closed in operator-pain order (pause/resume, registry
> lifecycle + policy, weak-term forget, per-item DLQ). What is deliberately *not* built is listed
> at the end with reasons, so the next audit reads decisions, not omissions.

## What was built, recommendation by recommendation

### 1. Render what already arrives *(commit `31477b5`)*
Every field an admin endpoint returned and a page silently dropped now renders: the `index` block
(alias, resolved index, Meilisearch URL) on Compute; GPU name/VRAM on healthy boxes and models'
`actual_mib`; the three missing ranking knobs (`unknown_date_factor`, `per_domain_cap`,
`simhash_collapse_distance`); source health's `approved`/`crawlable` and failed/thin/duplicate
counts; Live's `parsed`/`revisited` and per-row host/time; dead letters' `failed_at`; media
endpoints; interaction's `hot_floor`; the weak-coverage resolution warning ("no source configured —
these terms are collected but nothing chases them"). The enqueue/add/remove client types were
fixed to the real API shapes, so the UI now reports "already known" and "not listed" honestly
instead of claiming success unconditionally.

### 2. The Configuration page *(commit `71f9a36`)*
`GET /admin/config` serialises the **effective** `Config` — after file, env overrides, and
validation — with every secret redacted before it leaves the process (`admin_key`, `meili_key`,
Brave key, SERP proxy, Qdrant key, interaction salt render as `«set — redacted»`). The page
(SYSTEM → Configuration) is a filterable per-section table. Read-only on purpose: recommendation 6
(config *editing* through `Config::validate()`) remains the documented longer-term path, so the
k-floors and politeness guards can never be bypassed from a browser.

### 3. The capacity alarm on Overview *(commit `aca8b7f`, extends PROB-001)*
Overview carries a Capacity chip (amber from 80%: "Redis N% — drain or raise maxmemory"); the
Queue page carries the full tiles (used/max bytes, frontier waiting/deferred) and the ≥80% banner
explaining that the crawler self-pauses at 85%. This is the signal whose absence let the PROB-001
OOM arrive silently.

### 4. The Evaluation page *(commit `f9cfacd`)*
`GET /admin/eval` reads `eval/reports/*.json` and summarises each by kind (dated eval runs,
`baseline.json`, `ab-*`, `serp-*`, `calibration-*`): headline scores, per-language nDCG, A/B
variants. It computes the **regression-gate verdict with the same relative tolerance as
`xustive eval --baseline`** (1%), so the console shows the verdict CI would give, never a second
opinion; absent scores stay absent rather than defaulting to zero (the BUG-019 lesson). Miner
sheets (`data/expansion/candidates-*.tsv`) are listed with row counts; unparseable report files
are named loudly. The page (SEARCH → Evaluation) renders the trend table, the gate banner, each
A/B run, and the sheets awaiting review. Read-only: re-baselining and applying calibrations change
what "regression" means, so they stay deliberate CLI acts.

### 5. Controls, in operator-pain order
- **Crawler pause/resume** *(commit `7e8e53a`)* — `POST /admin/crawler/pause` sets a flag in Redis
  beside the crawl state; every worker polls it in its throttled guard probe (effect within
  seconds, survives restarts of console and crawler alike). Pausing holds **new claims only** —
  in-flight fetches finish politely, the frontier is untouched. The Live page has the button, a
  paused banner, and a `paused` state distinct from idle-or-broken.
- **Registry lifecycle + crawl policy** — `POST /admin/crawler/registry` performs the same
  transitions as `xustive registry approve|activate|disable` with the same guards (an archived
  source must be re-proposed, never resurrected from a button), records the disable reason in the
  registry, and edits the policy fields the CLI never had a verb for: frequency
  (realtime/hourly/daily/weekly), per-run doc cap, crawl delay, depth. Floors are server-side —
  the delay can be raised freely but never set below 500 ms, and `respect_robots` is not editable
  at all. The Source health page grew a per-row editor next to the numbers that motivate using it;
  the registry is written atomically (tmp + rename), and the response says honestly that changes
  take effect on the crawler's next seeding pass.
- **Weak-term forget** *(commit `7e8e53a`)* — the `WeakCoverage::forget` that existed only in code
  is now `POST /admin/crawler/weak-coverage/forget` plus a per-row button. Forgetting is
  *dismissal, not suppression*: a real gap re-accumulates past the k-anonymity floor on its own.
- **Per-item dead letters** — dead-letter rows now carry their stream entry id;
  `POST /admin/queue/dead/replay` re-enqueues one letter (re-enqueue first, delete second — the
  same crash-ordering argument as dead-lettering itself, so a crash can duplicate but never lose),
  and `POST /admin/queue/dead/drop` is the queue's only deliberate discard, double-click-confirmed
  in the UI and logged. Both answer `found: false` for an id already gone rather than erroring,
  because the page may be a poll behind reality. Covered by a live-Redis integration test.

## Knobs and validation floors introduced

| Control | Where | Guard |
|---|---|---|
| pause flag | Redis `crawl:paused`, worker guard probe | new claims only; ≤2 s to take effect |
| registry action | `POST /admin/crawler/registry` | archived sources refuse approve/activate |
| `crawl_delay_ms` | same endpoint | ≥ 500 ms, server-side |
| `depth_limit` | same endpoint | 1–10 |
| `max_docs_per_run` | same endpoint | 1–100 000 |
| DLQ drop | `POST /admin/queue/dead/drop` | confirm-in-UI + warn-level log |

## Deliberately not built (decisions, not omissions)

- **Ranking-weights editor.** Editing means writing `config/ranking.toml` (a file the repo does
  not ship) and restarting; doing that safely is exactly recommendation 6 — config editing routed
  through `Config::validate()` so the relevance-dominance bound is enforced server-side. Building
  a one-off editor beside that path would create a second, weaker validation surface. The weights
  are *visible* (all ten now render), and calibration verdicts arrive on the Evaluation page; the
  apply step stays a reviewed file change.
- **Blocklist manager.** The three-tier exclusion set is in-memory with no persistent file wired —
  a UI over it would silently lose entries on every restart, which is worse than no UI. Wiring
  blocklist persistence is the prerequisite line item the register already named; the manager
  belongs after it.
- **Config editing in general** (recommendation 6). Longer-term as ranked: the safe subset routed
  through the same `Config::validate()` as file changes, so k-floors, politeness, and salt rules
  apply identically. Nothing in this solution blocks it; the Configuration page is its natural
  read side.
- **Prometheus mirror pages.** The console mirrors the families an operator acts on (capacity,
  queue, sources, evaluation, interaction). Charting all ~27 families would duplicate Grafana,
  which already exists in dev for exactly that; the console's job is decisions, not dashboards.

## Verification

- `cargo test -p xustive-api -p xustive-queue` green (112 + 11 unit tests, including the eval
  summariser and gate-classification tests); the per-item DLQ path has a live-Redis integration
  test (`one_dead_letter_can_be_replayed_or_dropped_without_touching_the_rest`).
- `npx tsc --noEmit` clean; telemetry/compose/docs lints green.
- Operational note: the host API process must be restarted to serve the new endpoints — the same
  restart already owed for the PROB-001/002 binaries.

---

> **2026-08-28 — [[Milestone 12 - The Operator's Console]] revisits the "deliberately not built"
> list.** The ranking-weights editor is built (runtime settings with `Weights::check`, persisted
> to `runtime.toml`); config editing exists for the whitelist an operator tunes (budgets,
> switches) and stays off for the rest; charts exist without Grafana, from a 24-hour ring the
> API keeps itself. The blocklist manager stays unbuilt for the reason given.
