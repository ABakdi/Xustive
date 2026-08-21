---
tags:
  - planning
  - milestone
milestone: 4
status: in-progress
updated: 2026-08-21
---

# Milestone 4 - Quality and Operations

> **Goal:** make it survivable. Scale to 10M documents, prove the [[Performance Budgets]] numbers on
> real data, and build the operational apparatus that lets a small team run this without heroics.
> **Exit gate:** load test passes at 10M documents; restore drill green; every alert has a runbook;
> security review clear.
> Parent: [[TODO]] · Previous: [[Milestone 2 - Ingestion at Scale]] · Next: [[Milestone 5 - Beta Launch]]

---

## Why This Milestone Exists

Everything before this was built and measured at fixture scale. This milestone is where the numbers
in [[Performance Budgets]] stop being targets and become either facts or corrections.

The most likely outcome is that **something does not hold at 10M documents** — most plausibly single-
node Meilisearch latency ([[Performance Budgets]] §10). Discovering that here, with time to react, is
the entire point. Discovering it during beta is not.

---

## M4-T01 — [[Observability]]

- [ ] M4-T01.1 Every metric in its §2 emitted, with bounded label cardinality
- [ ] M4-T01.2 Structured logging conventions applied across all four binaries
- [ ] M4-T01.3 Tracing spans and ingestion `trace_id` correlation end to end
- [ ] M4-T01.4 Sampling: 100 % errors, 1 % successful searches, one full ingestion chain per hour
- [ ] M4-T01.5 Six Grafana dashboards, provisioned from git
- [~] M4-T01.6 Alerts configured with thresholds and severities — *the §6 alerts whose metrics the API emits today are live and **promtool-unit-tested**: `SearchDown`, `SearchLatencyHigh`, `ZeroResultsSpike`, `SummaryDropHigh`, `QueueBacklog`, `DLQGrowth` (plus the existing tool-data set). The social/proxy/signer alerts wait on the metrics their features will emit and are deliberately not configured against absent series. (promtool caught — and I fixed — a `clamp_min(rate,1)` distortion in the ratio exprs.)*
- [ ] M4-T01.7 **`TelemetryLeak` alert wired and tested** with a deliberate synthetic leak
- [ ] M4-T01.8 Log volume within the 2 GB/day budget at projected load

## M4-T02 — [[Error Handling and Resilience]]

- [ ] M4-T02.1 `ErrorClass`-driven retry layer applied everywhere (no string matching)
- [x] M4-T02.2 Circuit breakers with shared Redis state and exponential cooldown — *two variants: `xustive_core::circuit` (in-process, injectable clock, wired into the STT sidecar with admin visibility) and `xustive_queue::breaker::RedisBreaker` (fleet-wide, atomic Lua transitions, one half-open probe across all instances). Both fully unit/integration-tested — the Redis one verified live against a clean instance (trip → cooldown → probe → close → exponential backoff). **Wired into all three sidecar clients** (STT/OCR/CLIP) with the in-process breaker, and into the **indexer's Meili writes** with the *shared* one: `BreakeredSink` wraps the worker's sink so the whole fleet backs off together when Meili is down — a fast, **retryable** failure that leaves batches for redelivery (nothing lost or dead-lettered), probing for recovery in unison. Verified: the worker arms the shared breaker and drains normally. (A breaker on the *search read* path is deliberately still absent — a down node refuses connections instantly, so there is no timeout to save there.)*
- [ ] M4-T02.3 Backpressure thresholds wired from queue depth to crawl dispatch
- [ ] M4-T02.4 DLQ tooling: stats, peek, replay, retention
- [ ] M4-T02.5 Degradation ladder verified by fault injection, step by step
- [ ] M4-T02.6 Idempotency audit: every stage re-runnable, with a test per stage
- [x] M4-T02.7 Graceful shutdown: drain in-flight, ack, exit within the grace period — *shared `shutdown` helper (SIGTERM/Ctrl-C + a 25 s grace bound); the worker stops taking batches (unacked in-flight redelivers, idempotent by id), the crawler drains bounded, the API arms a grace timer over axum's drain. Worker exit verified live at ~1 s*
- [ ] M4-T02.8 **Chaos exercises**: kill Redis, kill Meilisearch, kill `xustive-ml`, fill the disk —
      assert the documented behaviour in each case

## M4-T03 — Load testing

- [x] M4-T03.1 Load harness with realistic query mix by language — *`xustive-loadgen`: a Rust-native **open-loop** generator (no k6/oha dependency), weighted ar/ary/fr/en query mix, p50/p95/p99, distinguishes ok/error/shed, exits non-zero on a budget miss. `make load S=…`. Verified live against the local API. The 10M-scale runs below (T03.2–.6) are the exercise it instruments*
- [ ] M4-T03.2 500 rps search for 10 min → p95 ≤ 200 ms
- [ ] M4-T03.3 2 000 rps suggest → p95 ≤ 40 ms
- [ ] M4-T03.4 20 concurrent summaries → drop ≤ 2 %, **search latency unaffected**
- [ ] M4-T03.5 **Contention case: 2 000 docs/s indexing while serving 500 rps**
- [ ] M4-T03.6 Crawler at 100 pages/min/worker with politeness intact
- [ ] M4-T03.7 Publish results against [[Performance Budgets]]; **correct the budgets where reality
      disagrees**, with a [[Decision Log]] entry for each change
- [ ] M4-T03.8 Decide: read replica for Meilisearch, yes or no?

## M4-T04 — Backup and restore

- [~] M4-T04.1 Meilisearch snapshots, shipped off-host — *`scripts/backup.sh` triggers + waits for the snapshot task and copies it out; the schedule (every 6 h) is a cron/timer wrapper, and the snapshot-dir path is deployment-specific (warns if the file is not found)*
- [x] M4-T04.2 Qdrant collection snapshots — *`backup.sh` snapshots each collection and downloads it over HTTP; verified live (394 KB snapshot pulled)*
- [x] M4-T04.3 Redis AOF + RDB copies — *AOF already on (`appendonly yes`); `backup.sh` BGSAVEs and copies `dump.rdb` out; verified live (572 MB captured)*
- [x] M4-T04.4 Registry export — *`backup.sh` copies the git-versioned registry into the backup set*
- [~] M4-T04.5 **Restore drill** — *`scripts/restore.sh` recovers Qdrant (snapshot upload) and Redis (dump.rdb + restart), prints the Meili startup-import steps, and lists the verify checks (stats / a search / reconcile-vectors) that are the drill's real pass/fail; guarded by `CONFIRM=yes`. Still owes a real wipe-and-restore run in staging to measure RTO/RPO (T04.6)*
- [ ] M4-T04.6 Measure actual RTO/RPO; correct [[Deployment Topology]] §7 if they differ
- [ ] M4-T04.7 Resolve where off-host backups physically live, given data sovereignty
- [x] M4-T04.8 Index migration drill: build staging, verify, alias flip, roll back — *`xustive-cli reindex` builds `<index>_next` with the current settings, copies every document (all fields, not just displayed), verifies the count, and **atomically swaps** it in (Meili `swap-indexes`); `--rollback` swaps back (previous contents kept in staging), `--dry-run` previews. **Verified live end-to-end against a throwaway index** — swap, search, rollback, cleanup. (Continuous **dual-write** during the copy is the one piece left for a hot index; the drill mechanism and the flip/rollback are proven.)*

## M4-T05 — Scale to 10M documents

- [ ] M4-T05.1 Expand the registry and crawl budget toward 10M
- [ ] M4-T05.2 Measure real index size on disk; validate the 180–260 GB estimate
- [ ] M4-T05.3 Decide the `translit_body` question with real numbers ([[Data Model]] §9)
- [ ] M4-T05.4 Tune `MEILI_MAX_INDEXING_THREADS` so indexing cannot starve search
- [ ] M4-T05.5 Qdrant at 5M vectors: memory, recall, latency
- [~] M4-T05.6 Redis memory profile; act on the raw-blob storage decision if needed — *observed live: the dev Redis is **already at its 1 GB `maxmemory` cap** from crawl state, and with `noeviction` (correct — it holds queue/frontier state) that means writes start being refused (`OOM`). This is the binding constraint materialising early; the object-storage option for raw blobs (Deployment Topology) is the lever. Needs a proper profile of what occupies the 1 GB before deciding.*
- [ ] M4-T05.7 **Re-run the full relevance evaluation on the real corpus** — M1's tuning was done on
      fixtures and will need revisiting

## M4-T06 — [[Sentiment Engine]] evaluation

- [ ] M4-T06.1 Re-evaluate lexicon mode against real crawled content
- [ ] M4-T06.2 Track `xustive_sentiment_coverage` for lexicon staleness as slang shifts
- [ ] M4-T06.3 Resolve where labelled Algerian sentiment data comes from
- [ ] M4-T06.4 If data exists: train and calibrate transformer mode; else document the decision
- [ ] M4-T06.5 Fairness check: no language's macro-F1 below 0.60; stratified reporting
- [ ] M4-T06.6 Consider comment-aggregate "discussion mood" as a distinct signal

## M4-T07 — Spam and quality tuning

- [ ] M4-T07.1 Re-tune quality and spam weights against real distributions
- [ ] M4-T07.2 Watch the score histograms for drift signalling parser regressions
- [ ] M4-T07.3 Expand the spam phrase list from observed content
- [ ] M4-T07.4 Verify suppression (not deletion) behaviour end to end
- [ ] M4-T07.5 Confirm precision ≥ 0.90 — false positives silently hide legitimate content

## M4-T08 — Security review

- [ ] M4-T08.1 Threat-model walkthrough against [[Security and Privacy]] §2, updated for what got built
- [ ] M4-T08.2 External penetration test of the public surface
- [ ] M4-T08.3 Re-run the SSRF suite against the live crawler
- [ ] M4-T08.4 **Nightly log scan for query leakage** — must find zero
- [ ] M4-T08.5 Verify egress segmentation on the real deployment
- [ ] M4-T08.6 Secrets audit: rotation, scoping, no secrets in images or logs
- [ ] M4-T08.7 Dependency and model-licence audit refresh
- [ ] M4-T08.8 Prompt-injection red team against the live summariser

## M4-T09 — Runbooks

- [x] M4-T09.1 One runbook section per alert — *[[Runbooks]]: a section per configured alert (fires / means / confirm / resolve-in-order), covering all 10 armed alerts. The unbuilt-feature alerts in §6 have no runbook because they have no metric yet, stated explicitly*
- [ ] M4-T09.2 Incident procedure: severity levels, escalation, comms
- [~] M4-T09.3 Common operations — *the runbooks cover scale-workers, drain/replay-DLQ, roll back a reindex, restart toold inline where a resolution needs them; a standalone operations-cookbook section (force-recrawl, disable-source, takedown) is still to gather in one place*
- [ ] M4-T09.4 Recovery procedures per row of [[Error Handling and Resilience]] §8
- [x] M4-T09.5 **Delete any alert that does not have a runbook** — *enforced mechanically: `scripts/lint-runbooks.sh` fails CI if any configured alert lacks a `## <Alert>` section (and flags stale runbook sections too). Wired into `make lint` and CI. Negative-tested.*

---

## Exit Gate

| Check | Threshold |
|:---|:---|
| Scale | 10M documents indexed and searchable |
| Latency | all [[Performance Budgets]] §2–3 numbers met at scale, or corrected with a recorded decision |
| Contention | indexing at full rate does not push search p95 out of budget |
| Resilience | every chaos exercise produces the documented behaviour |
| Restore | full restore from backup in staging, within the stated RTO |
| Security | pen test findings resolved or accepted; zero query leakage in the log scan |
| Observability | every alert fires correctly in a test and has a runbook |
| Relevance | evaluation re-run on the real corpus; nDCG@10 ≥ 0.60 still holds |

## Risks

| Risk | Mitigation |
|:---|:---|
| Single Meilisearch node misses the latency budget at 10M | this is an expected outcome; M4-T03.8 makes it a planned decision rather than a beta incident |
| Real-corpus relevance is worse than fixture relevance | M4-T05.7 schedules the re-tune explicitly |
| Redis memory becomes the binding constraint | monitored since M2; the object-storage option is pre-designed |
| Alert fatigue from a large new alert set | M4-T09.5 forces deletion of unactionable alerts |
| Operational work is deprioritised for features | it is the exit gate; there is no M5 without it |

## Related

[[TODO]] · [[Performance Budgets]] · [[Observability]] · [[Error Handling and Resilience]] ·
[[Deployment Topology]] · [[Security and Privacy]] · [[Milestone 5 - Beta Launch]]
