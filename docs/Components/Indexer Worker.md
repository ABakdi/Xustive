---
tags:
  - component
  - ingestion
component-id: C19
binary: xustive-worker
status: specified
updated: 2026-08-06
---

# Indexer Worker

> **ID** C19 · **Binary** `xustive-worker` · **Upstream** `q:index` · **Downstream** [[Search Index]], [[Vector Index]]

## 1. Purpose

The only component that writes to the indexes. It batches, validates, submits, and confirms — and it
owns the guarantee that a message is acknowledged **only after** the data is durably indexed.

## 2. Responsibilities

**In scope**: schema validation; batching and flushing; Meilisearch task submission and polling;
Qdrant point upserts; partial-failure isolation; deletions and takedowns; index alias handling during
migrations.

**Out of scope**: enrichment (→ [[Enrichment Pipeline]]); index settings (owned by the migration job,
see [[Search Index]] §4.2).

## 3. Interface

Consumes `q:index`: `{ document, comments }`. Writes to Meilisearch and Qdrant. Emits nothing
downstream — this is the end of the pipeline.

Also serves the deletion path from [[Admin and Source Submission]]:
`delete(document_id)` → removes the document, its comments, and its vectors, in that order.

## 4. Internal Design

### 4.1 Batching

Accumulate until **any** trigger fires:

| Trigger | Default |
|:---|:---|
| `batch_size` | 1 000 documents |
| `batch_bytes` | 8 MiB |
| `batch_timeout_ms` | 2 000 |

The timeout matters as much as the size: at low ingestion rates a size-only trigger would leave
documents unindexed for hours, and index freshness is an SLO ([[Observability]] §7).

### 4.2 Submit and confirm

```
1. validate(batch)                       → reject invalid docs to DLQ, keep the rest
2. POST /indexes/documents/documents     → task_uid
3. POST /indexes/comments/documents      → task_uid
4. PUT  /collections/image_clip/points   → vector upsert
5. poll GET /tasks/{uid} until succeeded/failed  (backoff 50ms → 1s, cap 60s)
6. XACK the whole batch on success
```

**Acknowledgement happens last.** If the worker dies at step 5, the consumer group redelivers the
batch and it is re-indexed — which is safe because indexing is an upsert by primary key
([[Error Handling and Resilience]] §7).

### 4.3 Partial failure isolation

Meilisearch fails a task as a unit. When a batch fails:

1. Split in half, retry both halves.
2. Recurse until batches of 1 isolate the offending document(s).
3. Send the bad document(s) to `q:dlq:index` with the Meilisearch error; index the rest.

Binary search costs `log2(1000) ≈ 10` extra round trips in the worst case — cheap compared to losing
999 good documents to one malformed one.

### 4.4 Validation before submit

| Check | Action on failure |
|:---|:---|
| Required fields present (`id`, `url`, `source_type`, `crawled_at`) | DLQ |
| `schema_version` ≤ current | DLQ + alert (a newer producer is running) |
| Field types match the index schema | DLQ |
| `body` ≤ 200 KB | truncate, flag |
| `published_at` sane (1995 … now + 1 day) | clamp + flag `date_clamped` |
| Filterable enum values valid (`source_type`, `sentiment.label`) | DLQ |
| Embedding dimension == 512 | drop the vector, keep the document |

Validating here rather than trusting upstream is deliberate: this is the last gate before data
becomes permanent, and a schema error that reaches the index requires a reindex to fix.

### 4.5 Deletions

Order matters. Delete vectors **first**, then comments, then the document:

```
1. Qdrant delete by filter document_id == X
2. Meilisearch delete-batch on comments filtered by document_id
3. Meilisearch delete document X
4. add URL to the takedown blocklist (so re-crawl cannot resurrect it)
```

Reversing the order would leave orphan vectors findable by image similarity after the document is
gone — a real privacy failure ([[Security and Privacy]] §8, [[Vector Index]] §7).

### 4.6 Reindex/migration mode

When `documents_v2` is being built, the worker dual-writes to both `documents_v1` and `documents_v2`
until the backfill completes, then the alias flips and v1 writes stop
([[Data Model]] §7). Dual-write is enabled by config, not by a code deploy.

## 5. Configuration

| Key | Default |
|:---|:---|
| `batch_size` | 1000 |
| `batch_bytes` | 8 MiB |
| `batch_timeout_ms` | 2000 |
| `task_poll_initial_ms` / `task_poll_max_ms` | 50 / 1000 |
| `task_poll_timeout_s` | 60 |
| `max_retries` | 5 |
| `split_on_failure` | `true` |
| `vector_batch` | 256 |
| `dual_write_targets` | `[]` |
| `meili_index_key` | secret (index-only scoped key) |

## 6. Data

Writes `Document` → `documents`, `Comment[]` → `comments`, `MediaEmbedding` → `image_clip`
([[Data Model]]). Reads nothing but its input messages.

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Meilisearch unreachable | connection error | retry with backoff; **do not ack** — messages stay queued |
| Meilisearch task queue full | 503 / pending count | slow down; raise `batch_timeout_ms`; alert `QueueBacklog` |
| Task fails on a batch | task status | split and isolate (§4.3) |
| Task poll timeout (60 s) | timer | assume in-flight; do not resubmit blindly — check by primary-key read, then decide |
| Qdrant down | connection error | index the documents anyway; queue vectors to `q:vector_retry` |
| Disk full on Meilisearch | task error | `DiskPressure` page; stop ingestion, keep search alive |
| Invalid document | validation | DLQ that document only |
| Duplicate delivery | at-least-once | harmless — upsert by primary key |
| Dual-write partial success | per-target status | retry the failed target; never ack until both succeed |

The "do not ack" rule is the core durability property: nothing is acknowledged until it is in the
index, so a crash costs re-work, never data.

## 8. Performance

| Metric | Budget |
|:---|:---|
| Throughput | ≥ 2 000 docs/s/worker in steady state |
| Batch submit + confirm (1 000 docs) | ≤ 2 s p95 |
| Vector upsert (256 points) | ≤ 200 ms |
| End-to-end lag `q:index` → searchable | ≤ 30 s p95 |
| Memory | ≤ 1 GB (batch buffers) |

## 9. Observability

`xustive_docs_indexed_total{source_type}`, `xustive_index_batch_size` (histogram),
`xustive_index_batch_duration_seconds`, `xustive_index_task_failed_total`,
`xustive_index_split_total` (batch isolation events — a rising value means bad data upstream),
`xustive_index_lag_seconds`, `xustive_vector_upsert_total{outcome}`,
`xustive_index_validation_rejected_total{reason}`.

## 10. Security

Uses an **index-only** scoped Meilisearch key; it cannot read search traffic or change settings
([[Security and Privacy]] §7). Validation prevents malformed crawled data from corrupting the schema.
The deletion path is the enforcement mechanism for takedowns and upstream deletions, so its
correctness is a privacy requirement, not just a data-hygiene one.

## 11. Testing

- Unit: batching triggers; validation table; split-on-failure recursion.
- Integration: against a real Meilisearch container — index 10k documents, assert count and
  searchability; inject one malformed document into a batch of 1 000 and assert 999 index and 1 goes
  to DLQ.
- Crash safety: kill the worker between submit and ack; on restart, assert no duplicates and no loss.
- Deletion: delete a document with images and comments; assert all three stores are clean and the URL
  is blocklisted.
- Dual-write: enable two targets, fail one, assert no ack and correct retry.
- Load: sustain 2 000 docs/s for 10 minutes while running search traffic; assert search p95 stays
  within [[Performance Budgets]].

## 12. Open Questions

- [ ] Is 1 000 the right batch size for Meilisearch at our document size? Needs measurement — the
      optimum is usually bytes-bound, not count-bound.
- [ ] Should vector upserts share the document batch transaction-ally? They cannot truly, so what is
      the acceptable window of inconsistency? (Currently: seconds, reconciled nightly.)
- [ ] Do we need a write-ahead log independent of Redis for a stronger durability story?

## Related

[[Search Index]] · [[Vector Index]] · [[Enrichment Pipeline]] · [[Task Queue]] · [[Data Model]] ·
[[Error Handling and Resilience]] · [[Admin and Source Submission]]
