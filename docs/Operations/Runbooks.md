---
tags:
  - operations
  - ops
type: operations
status: living
updated: 2026-08-21
---

# Runbooks

> One section per configured alert ([[Observability]] §6, M4-T09). The rule is strict: **an alert
> without a runbook here gets deleted**, because an alert nobody can act on is noise that trains
> people to ignore the dashboard. Every rule in `deploy/prometheus/alerts.yml` must appear below.

Each section is: what fired, what it means, how to confirm it is real, and the concrete steps to
resolve — in the order to try them. Commands assume the `make` targets and `xustive-cli`.

> **Scope note.** [[Observability]] §6 also specifies alerts for the crawler's proxy/session/signer
> machinery (`ProxyPoolDegraded`, `SignerFailure`, `EgressMismatch`, …). Those features are not
> built yet, so their alerts are **not configured** and have no runbook here — they arrive with the
> metrics they watch. This file covers exactly what is armed today.

---

## SearchDown  · page

**Fires:** `up{job="xustive-api"} == 0` for 2 minutes — Prometheus cannot scrape the API at all.

**Means:** the search API is not answering. This is the outage everything else is secondary to.

**Confirm:** `curl -fsS http://<api-host>:8080/readyz` — a connection refused or a hang confirms it.

**Resolve, in order:**
1. Is the process alive? (`systemctl status xustive-api` / `docker ps` / the orchestrator's view.)
   If it exited, check its last logs for a panic or a failed startup dependency, then restart it.
2. If it is alive but not answering, check whether it is wedged on a dependency: `curl
   http://<meili-host>:7700/health`. A search engine will not become ready if Meilisearch is down.
3. If Meilisearch is the cause, jump to that engine's health (CPU, disk, memory) — the API recovers
   on its own once its dependency does.
4. Restart the API as the blunt fix once the dependency is healthy; graceful shutdown drains in
   ≤ 25 s (M4-T02.7), so a rolling restart does not drop in-flight searches.

---

## SearchLatencyHigh  · page

**Fires:** p95 of `/search` request duration > 400 ms for 10 minutes.

**Means:** one search in twenty is over budget; the perceived-time budget ([[Performance Budgets]]
§2) is blown.

**Confirm:** watch `histogram_quantile(0.95, …xustive_http_duration_seconds_bucket{route="/api/v1/search"})`
on the dashboard, and check whether `xustive_search_duration_seconds{stage=…}` points at one stage.

**Resolve, in order:**
1. **Which stage grew?** `xustive_search_duration_seconds{stage}` says whether it is Meilisearch,
   the merge/re-rank, or expansion. Measure before changing anything.
2. If it is the Meilisearch stage: check that engine's CPU and the index size. Single-node latency
   at scale is the *expected* first failure ([[Performance Budgets]] §10); if this is at 10M docs,
   this is the read-replica decision (M4-T03.8), not an incident.
3. If it is the merge/re-rank stage: a candidate pool that grew (a settings change) or a slow
   re-rank input (authority map reload). Check the last deploy.
4. If nothing changed and load is high, this is capacity — shed is already automatic (the API sheds
   with 503 under overload); scale the API and/or Meilisearch.

---

## ZeroResultsSpike  · page

**Fires:** zero-result searches / total searches > 25 % over 15 minutes.

**Means:** a quarter of searches return nothing. The corpus did not vanish — the query path stopped
matching it. **Almost always a bad index or settings deploy.**

**Confirm:** run a known-good query (`xustive-cli search "الجزائر"`). If *it* returns nothing, the
index or its settings are wrong, not the queries.

**Resolve, in order:**
1. **Check the last `migrate`/reindex.** A filterable/searchable-attribute change that did not apply
   is the classic cause (this is exactly the News-vertical bug: a filter on an undeclared attribute
   made the whole query 500). `make migrate-check` reports drift between declared and live settings.
2. If settings drifted, re-apply them: `make migrate` (idempotent).
3. If a reindex swapped in a bad index, **roll it back**: `xustive-cli reindex --rollback` restores
   the previous contents instantly (M4-T04.8).
4. If the index itself is empty/short, the indexer stalled — see **QueueBacklog** / **SearchDown**.

---

## SummaryDropHigh  · ticket

**Fires:** withheld summaries / attempted summaries > 20 % over 15 minutes.

**Means:** the summariser is dropping more than a fifth of generations (validator rejections or
timeouts). **Results are unaffected** — the summary never blocks the result list — so this is a
ticket, not a page.

**Confirm:** `xustive_summary_withheld_total` rate vs `xustive_summary_duration_seconds_count`; the
API logs record the withholding reason (uncited / wrong-language / timeout).

**Resolve, in order:**
1. If the reason is **timeouts**: `xustive-ml` is starved. It shares the machine with search on the
   reference hardware; check CPU/GPU contention. Scale or move it to its own node.
2. If the reason is **validator rejections** (uncited, wrong language): the model is producing weak
   output — usually the 3B CPU model under load. This is a known quality ceiling; the fix is the 7B
   model on a GPU, not an incident response.
3. Confirm the model actually loaded: `curl http://<api>:8080/api/v1/admin/status` shows the model
   status. A model that never loaded withholds 100 %.

---

## QueueBacklog  · ticket

**Fires:** `xustive_queue_depth` > 50 000 for 30 minutes.

**Means:** documents are waiting to be indexed faster than the single Meilisearch writer drains
them. Search still serves the *already-indexed* corpus, so this is a freshness problem, not an
outage.

**Confirm:** `xustive-cli stats` (indexed count vs expected) and the `xustive_queue_depth` trend —
is it climbing, flat, or draining?

**Resolve, in order:**
1. **Are workers running?** The backlog only drains if `xustive-cli worker` processes exist. Start
   or scale them (`make worker`, or more replicas).
2. If workers are running but the depth is flat, they are blocked on Meilisearch — check its write
   throughput and `MEILI_MAX_INDEXING_THREADS` (M4-T05.4). The shared indexer breaker (M4-T02.2)
   means a truly-down Meili makes workers back off; check its state in the logs.
3. If the crawl is simply outrunning the indexer, throttle the crawler — the backpressure check
   already pauses it above a threshold; verify that is firing, and lower the crawl rate if needed.

---

## DLQGrowth  · ticket

**Fires:** `increase(xustive_queue_dead_letters[15m]) > 10`.

**Means:** the indexer is *giving up* on documents — more than a trickle in 15 minutes. The
dead-letter gauge only rises (replay is manual), so every one is a document the index will not get
until someone acts. Sustained growth is data loss.

**Confirm:** `make dlq A=peek` shows the dead-lettered payloads and their rejection reasons.

**Resolve, in order:**
1. **Look for a common cause.** A run of the same rejection reason points at one class of bug — a
   parser regression producing malformed documents, or a schema/settings mismatch (a field the
   index rejects). Fix the cause first; replaying into the same broken state just re-dead-letters.
2. Once fixed, **replay**: `make dlq A=replay`. Replay is deliberately manual so a poison payload
   cannot loop automatically.
3. If the payloads are genuinely bad and unfixable (corrupt source), they stay dead-lettered by
   design — record why, and they age out of the DLQ per its retention.

---

## ToolDataAgeing  · ticket

**Fires:** `xustive_data_age_seconds{dataset="weather"}` > 90 min for 10 minutes.

**Means:** the tool-data fetcher (`xustive-toold`) has not written a fresh value in over 90 minutes.
Cards **still render** (the serving plane withholds only at 3 h), so this is the window to fix it
before users see the feature disappear.

**Confirm:** `xustive_data_age_seconds` trend; check `xustive-toold` is running and can reach the
publisher.

**Resolve, in order:**
1. Is `xustive-toold` running? (`make toold` runs one pass; the service runs on a timer.) Restart it
   if it died.
2. If it is running, it cannot reach or parse the publisher — check its logs for fetch errors or
   validation rejections.
3. This shares a cause with **ToolDataStale** below; fixing it here prevents that page.

---

## ToolDataStale  · page

**Fires:** `xustive_data_age_seconds{dataset="weather"}` > 3 h for 5 minutes.

**Means:** past the staleness limit — the serving plane now **withholds** the cards, so the feature
is already gone for users. This is the page that **ToolDataAgeing** was the warning for.

**Confirm / resolve:** same cause and steps as **ToolDataAgeing**, but now user-visible, so treat it
as an active incident: restart `xustive-toold`, confirm it completes a pass (the age gauge drops),
and verify a weather card reappears.

---

## ToolDataMissing  · page

**Fires:** `absent(xustive_data_age_seconds{dataset="weather"})` for 15 minutes.

**Means:** there is **no** weather data cached at all — distinct from stale (which means old data
exists). Either Redis was flushed, or `xustive-toold` has never completed a pass. A threshold rule
cannot catch this: a flushed cache publishes *no series*, so absence must be alerted on directly.

**Confirm:** `redis-cli KEYS 'toold:*'` (or the equivalent) — nothing there confirms it.

**Resolve, in order:**
1. If Redis was flushed (e.g. after an OOM recovery — see [[qwen-3b-noncommercial-licence]]'s
   sibling finding on Redis memory), the fetcher just needs to run: `make toold`, or wait for its
   next timer pass.
2. If `xustive-toold` has never run in this deployment, start it.
3. Confirm coverage recovers via **ToolDataCoverageDropped**'s gauge.

---

## ToolDataCoverageDropped  · ticket

**Fires:** `xustive_data_entries{dataset="weather"} < 55` for 30 minutes.

**Means:** a *partial* fetch failure — some wilayas are cached, others are not. The age gauge cannot
see this (the wilayas that do refresh keep it healthy), so coverage is watched separately.

**Confirm:** the `xustive-toold` logs show per-wilaya rejection reasons.

**Resolve, in order:**
1. A run of `out_of_bounds` rejections points at the **publisher** sending bad values — nothing to
   fix on our side but worth noting.
2. A run of `moved_too_far` points at a **real event** we are refusing as implausible — check
   whether the validation threshold is too tight for a genuine weather swing.
3. If a subset of wilayas simply fails to fetch, check whether the publisher dropped those
   endpoints.

---

## Common operations

The routine actions an operator performs, with the exact commands (M4-T09.3). Where a one-shot
command does not exist yet, that is stated rather than papered over.

### Scale indexer workers

The index queue drains only as fast as the running workers. To add capacity, start more worker
processes — they share a Redis consumer group, so work is split automatically and never duplicated:

```
make worker            # one worker, runs until SIGTERM
```

Run several (more replicas / more `make worker` invocations / a higher replica count in the
orchestrator). Graceful shutdown means scaling *down* is safe: a worker stops taking new batches and
leaves any in-flight batch unacked for another worker to pick up (M4-T02.7).

### Drain the queue once and stop

For a controlled catch-up (e.g. after a backlog), process what is queued and exit rather than running
continuously:

```
cargo run -p xustive-cli -- worker --once
```

### Inspect and replay the dead-letter queue

```
make dlq A=stats       # how many, and the rejection-reason breakdown
make dlq A=peek        # the actual dead-lettered payloads
make dlq A=replay      # re-enqueue them — do this ONLY after fixing the cause (see DLQGrowth)
```

Replay is deliberately manual: a poison payload that auto-replayed would loop.

### Disable a source (opt-out, takedown, persistent failure)

```
cargo run -p xustive-cli -- registry disable <source-id> --reason "operator: <why>"
```

This flips the source in the registry and starts its 90-day archival clock. It stops *future*
crawling of the source; it does not by itself remove already-indexed documents — for that, see the
takedown flow below.

### Force a recrawl

The crawler resumes from the shared frontier by default. To restart the frontier from the seed list
(a full re-discovery, not a targeted recrawl):

```
cargo run -p xustive-cli -- crawld --reset
```

> ❌ **Not built: targeted single-URL / single-source recrawl.** There is no CLI command today to
> force *one* URL or source to be re-fetched ahead of schedule — the recrawl scheduler
> ([[Adaptive Recrawl over Static Crawling]]) decides cadence. Forcing one is a follow-up; the
> primitives (`revisit::Visits::forget`) exist in the ingest crate but are not exposed on the CLI.

### Execute a takedown (remove already-indexed content)

A takedown is a **composite**, not one command — the content lives in several stores and each must be
cleared:

1. **Stop future crawling:** `registry disable <source-id>` (above), or add the host to the takedown
   exclusion tier so it is refused even if reachable another way.
2. **Remove the image vectors** whose documents are being taken down, then reconcile orphans:
   `cargo run -p xustive-cli -- reconcile-vectors` deletes vectors whose parent document no longer
   exists (M4-T04.8 / [[Security and Privacy]] §8).
3. **Remove the raw stored bodies** so the page's bytes do not linger (`raw_store` TTL, or an
   explicit purge).

> ❌ **Not built: a single `xustive-cli takedown <url|domain>` command** that performs all of the
> above atomically. The store-level primitives exist (`MeiliClient::delete_document`,
> `Store::delete_by_document`, `raw_store` drop, the exclusion tier), but wiring them into one
> audited command is a follow-up. Until then, a takedown is the operator running the steps above and
> recording what was removed.

---

## Incident procedure

**Severity.** Each alert above is tagged **page** or **ticket**:

- **page** — user-visible or imminent: the search API is down or slow, results have gone empty, a
  user-facing tool card has been withheld. Respond now.
- **ticket** — degraded but not user-fatal: a growing backlog, dropped summaries (results are
  unaffected), partial tool coverage. Respond in hours, not minutes.

**On a page, in order:**
1. **Confirm it is real** — every runbook above has a one-line confirm step. A false alert is itself
   a bug: fix the rule (`deploy/prometheus/alerts_test.yml`) so it does not page again.
2. **Stop the bleeding** before finding root cause — roll back the last deploy (`reindex --rollback`
   for an index/settings regression), restart the wedged process (graceful, ≤ 25 s drain), or shed
   load. The system already sheds automatically under overload (503) and fails fast when a
   dependency is down (circuit breakers, M4-T02.2), so "stop the bleeding" is often "let the
   automatic protection work and remove the trigger".
3. **Then** diagnose using `xustive_search_duration_seconds{stage}` / the DLQ / the logs.

**Communication.** Note what fired, when, the user-facing impact (or "none — results unaffected"),
and the action taken. The privacy posture holds during an incident too: **never** paste query text
into an incident channel — the nightly log scan (`make scan-logs`) exists precisely so query text
never leaves the box ([[ADR-0008 - No Query Logging]]).

> ❌ **Not built: a formal escalation/on-call policy** (who is paged, hand-off, comms templates) —
> that is an organisational decision for when there is a team to escalate *to*, not code
> (M4-T09.2). This section is the technical incident flow; the people-process wraps it later.

---

## Related

[[Observability]] · [[Error Handling and Resilience]] · [[Performance Budgets]] ·
[[Milestone 4 - Quality and Operations]] · [[Security and Privacy]]
