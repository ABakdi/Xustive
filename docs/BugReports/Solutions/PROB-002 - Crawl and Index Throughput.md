---
tags:
  - solution
  - performance
problem: PROB-002
date: 2026-08-25
status: solved
---
# PROB-002 Solution — Crawl and Index Throughput

> Problem: [[Problems#PROB-002 — Crawl and index throughput is low|Problems register → PROB-002]] ([../Problems.md](../Problems.md))
> Related: [[PROB-001 - Bounded Frontier and Queue]] — the connection-sharing and pipelining
> groundwork landed there; the host-diversity lever depends on its budgets.
> Outcome: **the per-page overhead is gone from the pipeline** (hundreds of serial Redis round
> trips → a handful; per-claim guard probes → one per two seconds; Meilisearch fed few large
> batches instead of task-per-dribble; healthy robots-silent hosts earn 1 rps). The remaining
> ceiling is host diversity, which is an operator decision documented below — code cannot add
> hosts, only stop wasting time between them.

## Where the time went, and what changed

### 1. A page's outlinks: hundreds of round trips → three
`Frontier::add` cost ~5 serial round trips per link on a fresh TCP connection each time; a 200-link
page could burn ~1,000 serial round trips. After PROB-001 (shared connection, pipelined add,
best-64 selection) and now **`add_batch`** — batched reads for the whole page, one `SADD`
arbitration pipeline (per-URL race semantics preserved), one write pipeline — a page's entire
enqueue is **~3 round trips**. Every growth bound behaves identically: per-host caps count the
batch's own admissions, ceiling eviction fires per admission, and `add()` is now a one-element
wrapper over the same code path. Stats publishes collapsed from per-link to one counted publish
per page. *(commits `20475fd`, `e1a5c8d`)*

### 2. Guard probes: per claim → per two seconds
Each of 64 workers checked indexer lag (`XINFO GROUPS`) and, post-PROB-001, Redis memory on
**every step** — two round trips per fetch for values that change on the scale of seconds. Each
worker now re-probes at most every 2s (`PROBE_EVERY`), holds the last verdict in between, and
re-probes immediately after any pause so recovery is prompt. *(commit `e1a5c8d`)*

### 3. The indexer: task-per-dribble → few large batches
`XREADGROUP` returns the moment anything exists, so light flow produced tiny batches, each paying
a full `add_documents` + task-wait cycle against Meilisearch's **single writer**. The consumer now
dwells: a small first read is topped up by short follow-up reads (300ms blocks) until half of
`MAX_BATCH` or a 2-second dwell elapses — released early when the producer goes quiet. Cost: ≤2s
of indexing latency, only under light flow; benefit: Meilisearch's documented
few-large-batches best practice. *(commit `e1a5c8d`)*

### 4. Politeness: the earned floor
Per-host pacing already shrank ×0.9 per success toward a floor — but the floor for a robots-silent
host was the conservative 1500ms default forever. Such a host can now **earn** a 1000ms floor
after 20 consecutive clean responses; one 429/5xx both grows the delay and revokes the streak, so
the rate is continuously re-proven. A declared `Crawl-delay` is never undercut — the site's own
word always wins. Effect: up to +50% throughput per healthy host, inside accepted practice
(1 req/s), with grow-fast/shrink-slow unchanged. *(commit `e1a5c8d`)*

## The lever that is not code: host diversity

Throughput ≈ `min(workers, distinct due hosts) / per-host delay`. With ~20 seed hosts the ceiling
is ~12-20 pages/s **no matter what the code does** — most of the 64 workers sleep. The proof that
one machine is not the limit: IRLbot sustained ~1,790 pages/s politely on 2008 hardware by being
host-diverse. The options, in order of preference:

1. **Enable discovery** (`crawld --discover`): now safe, because PROB-001 gave every new host a
   10k queued cap, a 20k lifetime budget, best-64 branching, and a global 200k ceiling with
   worst-tail eviction — the reasons discovery used to be dangerous are gone.
2. **Add seeds** (admin Sources page): each new host adds ~0.6-1 page/s of ceiling.
3. Raise `workers` only after the due-host count exceeds 64 — before that it buys nothing.

This is an operator decision (crawl scope), so it is recorded here rather than flipped in config.

## Knobs and constants touched

| What | Where | Value |
|---|---|---|
| `PROBE_EVERY` | crawld.rs | 2s between guard probes per worker |
| `BATCH_DWELL` / `DWELL_TOPUP_BLOCK` | indexer.rs | 2s dwell / 300ms top-up reads |
| `EARNED_MIN_DELAY` / `HEALTHY_STREAK` | robots.rs | 1000ms floor after 20 clean responses |
| `crawl.max_outlinks_per_page` | config (PROB-001) | best-64 selection — also a throughput win |

## Verification

- Unit: the earned floor is reached by a clean streak, revoked by one error, and a declared
  `Crawl-delay` is never undercut (`robots::tests`); indexer/queue suites green.
- Integration: the full frontier Redis suite exercises `add_batch` through the `add` wrapper on
  every existing test plus the four PROB-001 bound tests; whole workspace green.
- Not measured here: end-to-end pages/s, which is dominated by the host-diversity decision above —
  the honest metric to watch after enabling discovery is the Live page's fetch counters and the
  heartbeat's `redis_memory_pct` staying flat.
