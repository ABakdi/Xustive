---
tags:
  - solution
  - performance
  - meilisearch
problem: PROB-004
date: 2026-08-29
status: solved
---
# PROB-004 Solution — Index Throughput Decay

> Problem: [[Problems#PROB-004 — Indexing throughput decays as the index grows (260 → 10 documents a minute)|Problems register → PROB-004]] ([../Problems.md](../Problems.md))
> Related: [[PROB-002 - Crawl and Index Throughput]] (the pipeline side, unchanged here);
> [[Search Index]] §4.2 (the settings); [[Operating Xustive]] (the capacity check below).
> Outcome: **Meilisearch gets the memory its index needs and stops building its most expensive
> database.** Measured before/after in §3.

## 1. What changed

| Where | Before | After | Why |
|:---|:---|:---|:---|
| `deploy/docker-compose.yml` → `meilisearch.mem_limit` | 5g | **16g** | the cgroup limit counts the page cache; the mmap'd LMDB index (12.5 GB used) must fit, or every batch is re-read from disk |
| `MEILI_MAX_INDEXING_MEMORY` | 3Gb | 4Gb | indexing working memory, inside the larger limit |
| `documents_settings()` → `proximityPrecision` | `byWord` (default) | **`byAttribute`** | the word-pair proximity database was the dominant per-batch cost on 6 kB bodies; Meilisearch's documented setting for large text indexes |

Nothing in the crawler, the queue or the worker changed: PROB-002's pipeline was not the
bottleneck. The stream backlog (20,000 entries at the cap, 98 k pending) drains on its own once
Meilisearch accepts batches at the restored rate; the worker's five-minute retry is what carries
the pending entries back.

## 2. How to recognise it again

- `docker stats xustive-meilisearch`: memory pinned at the limit, block-I/O *read* growing by
  terabytes.
- `/sys/fs/cgroup/…/memory.events` for the container: `max` in the millions;
  `memory.stat` `workingset_refault_file` in the hundreds of millions.
- `curl :7700/stats` → `usedDatabaseSize` approaching or above the container's memory limit.
- `curl ':7700/tasks?types=documentAdditionOrUpdate&statuses=succeeded'` → durations of tens of
  minutes for batches of a few hundred documents; `startedAt − enqueuedAt` in the tens of minutes.
- The worker log: `transient index failure; leaving for retry … search backend timed out`.

**The rule:** the Meilisearch container's memory limit must stay above `usedDatabaseSize` with
headroom. Check it whenever the corpus doubles. The console's Index queue page shows the lag;
the number to watch is in `/stats`.

## 3. Verification

Before (08-27 → 08-29): 8–21 documents per processing-minute, 16–18 minutes of queue wait per
task, 3,000–5,000 documents an hour.

After (08-29, once task 9224 — the `byAttribute` reindex — finished): see the figures appended
below by the measurement that closed this record.

## 4. Left undone, deliberately

- A body-length cap at indexing time. p50 is 3,400 characters and p99 42,700; a cap at p99
  trims 1 % of documents and saves a similar share of tokens — not worth the loss of long
  articles' text. Revisit if `usedDatabaseSize` growth outpaces memory.
- A larger index stream. The cap is a *bytes* decision (PROB-001); with the indexer restored
  the stream stops trimming. If it trims again with a healthy Meilisearch, the crawler is
  simply faster than the index and PROB-002's host-diversity note applies in reverse.
