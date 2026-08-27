---
tags:
  - component
  - ingestion
component-id: C19
binary: xustive-cli worker
status: built
updated: 2026-08-27
---

# Indexer Worker

> **ID** C19 · **Binary** `xustive-cli worker` (`crates/xustive-cli/src/worker.rs`; the logic is
> `crates/xustive-queue/src/indexer.rs`) · **Upstream** `q:index` ([[Task Queue]]) ·
> **Downstream** [[Search Index]]

## 1. Purpose

The only component that writes documents to Meilisearch. It batches, validates, submits, and
confirms — and it owns the one guarantee the whole ingestion plane rests on: a message is
acknowledged **only after** the data is durably indexed.

## 2. Responsibilities

**In scope**: validation; batching and flushing; Meilisearch task submission and polling;
bisecting a failed batch; dead-lettering; applying index settings on start; the shared circuit
breaker over the index backend.

**Out of scope**: enrichment (→ [[Enrichment Pipeline]]); vector upserts — the crawler writes
those itself ([[Vector Index]]); deletions and takedowns (→ `xustive-cli takedown` and the admin
takedown route, which delete from the index, Qdrant and the raw store directly); index migrations
(→ `xustive-cli reindex`, [[Search Index]]).

## 3. Interface

Consumes `q:index` (`QueueConfig.index_stream`) in consumer group `indexers`
(`xustive_queue::INDEXER_GROUP`) as consumer `{hostname}-{pid}`. Payload:

```rust
pub struct IndexJob { pub document: serde_json::Value, pub index: Option<String> }
```

`index` is carried per job so one queue could feed documents and comments; today every producer
leaves it `None` and the worker writes to the resolved `search.documents_index`. Emits nothing
downstream — this is the end of the pipeline.

```rust
pub trait Sink { async fn submit(&self, index: &str, documents: &[Value]) -> Result<(), SubmitError>; }
```

The `Sink` trait is why the ordering and bisection logic is unit-testable without Meilisearch —
making a real engine fail one specific batch on demand is not something it offers.

## 4. Internal Design

### 4.1 Startup

`ensure_index(index, "id")` then `apply_settings(documents_settings())`, every start. Meilisearch
auto-creates an index on first write with **no** settings, and `resolve` then prefers that bare
index over the configured one — one premature write left search unable to filter and every query
500ing. Applying settings is idempotent, so it costs nothing on a healthy index and closes that
door for good.

### 4.2 Batching

| Trigger | Value | Where |
|:---|:---|:---|
| `MAX_BATCH` | 1 000 documents | `indexer.rs` |
| `MAX_BATCH_BYTES` | 4 MiB | a batch of long articles trips this well before the count |
| `BATCH_TIMEOUT` | 5 s | the first read blocks this long for a partial batch |
| `BATCH_DWELL` / `DWELL_TOPUP_BLOCK` | 2 s / 300 ms | a small first read tops up until half-full or the dwell expires (PROB-002) |
| `TASK_TIMEOUT` | 120 s | waiting for a Meilisearch task |

The dwell trades two seconds of latency under light flow for far fewer Meilisearch tasks: its
single writer amortises large batches and drowns in tiny ones. The timeout matters at the other
end — without it the last few documents of a crawl sit unindexed until the next crawl, which on a
small site is never.

### 4.3 Submit and confirm

```
reclaim (XAUTOCLAIM > 5 min idle)  →  consume  →  dead-letter attempts > 3
  →  validate (DLQ the invalid)  →  split by bytes  →  add_documents  →  wait_task
  →  XACK the chunk
```

**Acknowledgement happens last.** Meilisearch returns a task id immediately and writes afterwards;
acking on submit would let the worker lose documents with no record. A crash between the write
landing and the ack redelivers the chunk, which is safe because indexing is keyed by `id` — a
repeated write is a no-op. At-least-once plus idempotence, which is achievable, rather than
exactly-once, which is not.

Reclaim runs before consume: a crashed worker's jobs are older than anything new.

### 4.4 Transient vs permanent

`SubmitError` carries `retryable`. A timeout or unreachable backend (`SearchError::is_retryable`)
is **transient**: the chunk is left unacked for redelivery. A task that ran and failed is
**permanent**: retrying it unchanged would fail identically. When this was a plain string every
failure looked permanent and a slow index discarded real documents — that is why the type exists.

### 4.5 Bisection

Meilisearch fails a batch as a unit and its error does not say which document. A permanent
failure on a chunk larger than one is split in half and each half resubmitted, recursively; a
chunk of one that still fails is dead-lettered with the engine's message. About nine extra round
trips isolate one bad document in five hundred, and the other 499 land.

### 4.6 Validation (`indexer::validate`)

| Check | Reason |
|:---|:---|
| is a JSON object | `not_an_object` |
| non-empty `id` | `missing_id` — without it a write is not idempotent |
| a `title` or a `body` | `empty` — unsearchable, only inflates the count |
| serialised ≤ 1 MiB | `too_large` — better refused with a reason than failing a batch |

Rejected documents go to the DLQ with the reason. There is no schema-version or field-type check
and no date clamping; the parser is trusted for shape, the index for types.

### 4.7 Shared breaker

`BreakeredSink` wraps the Meili sink in a `RedisBreaker` (namespace `worker`, name `meili-index`):
Lua-atomic transitions so every worker in the fleet trips together when the index is down, and
one `SET NX` half-open probe at a time. When Redis is unreachable the breaker is a no-op and
indexing proceeds unbroken. An open breaker returns a transient error, so batches are left for
redelivery.

### 4.8 Shutdown

SIGTERM/Ctrl-C stops taking new batches; a batch mid-flight is abandoned **unacked**, so it is
reclaimed and reprocessed on the next start. `queue.trim()` runs after every `run_once`.

## 5. Configuration

| Key | Default | Meaning |
|:---|:---|:---|
| `[queue] url` | `redis://127.0.0.1:6390` | the persistent queue Redis |
| `[queue] index_stream` | `q:index` | |
| `[queue] index_stream_max_len` | 20 000 | `XADD MAXLEN ~` cap — the runaway backstop |
| `[search] documents_index`, `meili_url`, `meili_key` | | the write target |
| `worker --once` | | drain and exit (tests, `make`) |

Batch sizes and timeouts are constants in `indexer.rs`; there is no dual-write target and no
index-only scoped key — the worker uses the configured `meili_key`.

## 6. Failure Modes

| Failure | Response |
|:---|:---|
| Meilisearch unreachable / timeout | transient — chunk left unacked; breaker opens across the fleet |
| Task fails on a batch | bisect; single offender dead-lettered with the engine's message |
| Invalid document | DLQ that document only, with `missing_id` / `empty` / `too_large` / `not_an_object` |
| Job delivered > 3 times | DLQ `exceeded delivery attempts` |
| Worker dies mid-batch | redelivered after 5 min via reclaim; upsert by id makes it harmless |
| Bare auto-created index | prevented by applying settings on every start |

## 7. Observability

`Stats { indexed, rejected, dead_lettered, batches }` printed per cycle; `xustive-cli dlq stats`
and the admin **Queue** page (`GET /api/v1/admin/queue`: backlog, dead count, dead letters, Redis
memory, frontier depth) show the same numbers ([[Crawler Console]]). No Prometheus counters yet.

## 8. Security

Writes with the configured Meilisearch key; a scoped index-only key is an open item. Validation
keeps a malformed crawled document from failing a batch, not from shaping the schema — Meilisearch
is schemaless and settings are applied by this worker. Deletion is not on this path.

## 9. Testing

`crates/xustive-queue/tests/indexer.rs` (ordering, validation, bisection against a fake `Sink`),
`tests/streams.rs`, `tests/breaker_redis.rs`; `make dlq` and `worker --once` are the manual loop.

## 10. Open Questions

- [ ] Vectors are written by the crawler, not here, so a document can be indexed with its vector
      already present or never present; `reconcile-vectors` cleans orphans the other way. Is the
      window acceptable?
- [ ] Comments: `IndexJob.index` exists for them; nothing produces them.
- [ ] A scoped write-only Meilisearch key for the worker.

## Related

[[Search Index]] · [[Task Queue]] · [[Vector Index]] · [[Enrichment Pipeline]] · [[Data Model]] ·
[[Error Handling and Resilience]] · [[Crawler Console]] · [[Admin and Source Submission]]
