---
tags:
  - component
  - platform
component-id: C20
binary: redis (crates/xustive-queue)
status: built
updated: 2026-08-27
---

# Task Queue

> **ID** C20 · **Service** `redis` (compose `redis`, port 6390) · **Client**
> `crates/xustive-queue` · **Upstream** [[Crawler Orchestrator]] (`crawld`), [[Federation Gateway]]
> (eager documents) · **Downstream** [[Indexer Worker]]

## 1. Purpose

The transport between crawling and indexing, and the single place where load is visible as a
number. It decouples the two so each can fail, restart and scale on its own, and it is what the
crawler reads to know when to stop producing ([[Error Handling and Resilience]]).

The same Redis also holds the frontier, dedup sets, the raw-body store, the embed cache, crawl
stats, the shared circuit breakers and the proxy state — one persistent dependency instead of
several. Behavioural signals live on a **second, ephemeral** Redis ([[Interaction Signals]]).

## 2. What exists today

One stream. The original design's five-stage pipeline (`q:fetch` → `q:parse` → `q:enrich` →
`q:index` → `q:vector_retry`) collapsed into a single hop because fetching, parsing and enrichment
run inside one `crawld` process and the only stage worth decoupling was the index write.

| Stream | Producer | Group | Payload |
|:---|:---|:---|:---|
| `q:index` (`[queue] index_stream`) | `crawld` (producer only, no group), federation eager index | `indexers` | `IndexJob { document, index }` |
| `q:index:dead` | the indexer | `indexers` | `DeadLetter { original_id, payload, attempts, reason, failed_at }` |

Other key spaces on this Redis (owners in their own notes): `frontier:*` (frontier, `seen:`
generations, host budgets, bandwidth), `frontier:seen_hashes` / `frontier:sim:*`
([[Deduplication Service]]), `frontier:raw:*` (raw bodies, [[Web Fetcher]]),
`frontier:vecphash:*` ([[Vector Index]]), the `worker:*` shared breaker, crawl stats and the
operator pause ([[Crawler Console]]), proxy pool state ([[Proxy Manager]]).

## 3. Interface

```rust
// crates/xustive-queue/src/lib.rs
Queue::connect(url, stream, group)        // XGROUP CREATE … MKSTREAM, tolerates BUSYGROUP
Queue::connect_producer(url, stream)      // no group — see §4.2
queue.produce(&job) / produce_many(&jobs) // XADD MAXLEN ~ max_len, one "payload" JSON field
queue.consume(consumer, count, block)     // XREADGROUP … ">"
queue.reclaim(consumer, count)            // XAUTOCLAIM idle > RECLAIM_AFTER (5 min)
queue.ack(id) / ack_all(ids)
queue.depth() / pending() / depth_of(group) / trim()
// dlq.rs
queue.dead_letter().{peek_dead, peek_dead_with_ids, replay_dead(limit), replay_dead_one(id), drop_dead(id), dead_count}
pub const MAX_ATTEMPTS: u64 = 3;
```

One `payload` field holding JSON rather than a field per property: the schema then lives in Rust
where `serde` defaults let it evolve, instead of in Redis where a rename is a migration.

## 4. Internal Design

### 4.1 Why Streams, not Lists

`LPUSH`/`BRPOP` loses work: a worker that pops a job and dies has taken it out of Redis with
nothing recorded anywhere. Streams keep a delivered-but-unacknowledged entry in the group's
pending list, so a dead worker's jobs are visible, reclaimable and countable. For a crawler that
spends someone else's bandwidth to re-fetch, silent loss is the failure that matters most.
[[ADR-0006 - Redis Streams for the Ingestion Pipeline]].

**At least once**, stated plainly. Consumers must be idempotent, and the indexer is (writes keyed
by id). Claiming exactly-once would let a consumer be written as though replay could not happen.

### 4.2 The producer has no group

`crawld` connects with `connect_producer`. A producer's own group never consumes, so its lag is
every message ever written — and when the crawler once measured *that* for backpressure it froze
itself the moment the count passed the threshold while the indexer was keeping up perfectly.
Backpressure now asks `depth_of(INDEXER_GROUP)` (`XINFO GROUPS` lag of the group that actually
drains the stream) without joining it.

### 4.3 Reclaim

`XAUTOCLAIM` rather than `XPENDING` + `XCLAIM`: one round trip, and it cannot race another
reclaimer into a double claim. `RECLAIM_AFTER = 300 s` — long enough that a slow batch is not
stolen mid-flight, short enough that a crashed worker's jobs do not sit idle for an hour. A job
delivered more than `MAX_ATTEMPTS = 3` times is dead-lettered instead of retried.

### 4.4 Trimming

Acked entries are **not** removed by `XACK`; without trimming a stream that processed ten million
pages holds ten million entries and Redis grows until `noeviction` refuses writes, which looks like
the crawler breaking for no reason. Every `XADD` trims approximately (`MAXLEN ~`, whole macro-nodes)
and the worker calls `trim()` after every cycle — on a timer as well as on write, because a queue
that stops receiving work stops trimming, and that is exactly the one nobody is watching.

The cap is `[queue] index_stream_max_len = 20 000` entries (code default `MAX_LEN = 100 000`).
The cap that matters is **bytes**: each entry is a full document, and the old 100k cap allowed
~900 MB of stream on a 1 GB Redis — PROB-001's actual OOM.

### 4.5 Backpressure

Each `crawld` worker probes every 2 s (`PROBE_EVERY`; per-claim probing cost two round trips per
fetch across 64 workers, PROB-002) and pauses itself when the indexer lag reaches
`BACKPRESSURE_AT = 5 000`, when the operator has paused the crawl, or when Redis `used_memory`
passes `MEMORY_HIGH_WATER = 0.85` of `maxmemory` — the universal backstop that makes the OOM wall
structurally unreachable by the crawler's own writes. All fail open with a warning: a crawler that
stops on a Redis blip is worse than one that briefly runs ahead.

### 4.6 Dead letters

Written **before** the original is acked — acking first and then failing to write leaves no
record. The entry keeps the payload as it was, so replay does not reconstruct it. Replay is always
deliberate: `xustive-cli dlq replay`, or the admin Queue page's replay-all / replay-one / drop-one
(`POST /api/v1/admin/queue/replay`, `…/queue/dead/replay`, `…/queue/dead/drop`). A queue that
retries its own poison on a timer will do it at 3 am after someone fixed the bug and went to bed.

### 4.7 Connections

One auto-reconnecting `ConnectionManager` per `Queue`, cloned across workers. The earlier
connection-per-call pattern spun up hundreds of short-lived connections a second under sixteen
workers and produced the "Multiplexed connection driver unexpectedly terminated" flood.

### 4.8 Persistence and memory (`deploy/docker-compose.yml`)

`--appendonly yes`, `--maxmemory 1gb`, `--maxmemory-policy noeviction`. `noeviction` is not
optional: under `allkeys-lru` Redis would silently drop queue entries and frontier state. The
signals Redis (`redis-signals`, 6391) runs `--appendonly no --maxmemory 192mb volatile-lru` on
purpose ([[ADR-0018 - Anonymous Search History]]).

## 5. Configuration

| Key | Dev default | Meaning |
|:---|:---|:---|
| `[queue] url` | `redis://127.0.0.1:6390` | persistent queue Redis |
| `[queue] signals_url` | `redis://127.0.0.1:6391` | ephemeral signals Redis; empty falls back to `url` |
| `[queue] index_stream` | `q:index` | |
| `[queue] index_stream_max_len` | 20 000 | `XADD MAXLEN ~` |
| `RECLAIM_AFTER`, `MAX_ATTEMPTS`, `BACKPRESSURE_AT`, `MEMORY_HIGH_WATER`, `PROBE_EVERY` | 300 s, 3, 5 000, 0.85, 2 s | constants in code |

## 6. Failure Modes

| Failure | Response |
|:---|:---|
| Redis down | ingestion halts; **search is unaffected** — the API touches this Redis only for the eager federation write and the admin pages, both fail-open |
| `maxmemory` reached | writes fail loudly; the crawler already paused at 85 % |
| Stream growth | trimmed on write and per worker cycle; the cap is the backstop |
| Consumer dies mid-job | reclaimed after 5 min; idempotent upsert |
| Poison job | dead-lettered after 3 deliveries, or immediately on validation / permanent rejection |
| Phantom producer group | prevented — producers create none |

## 7. Observability

`xustive-cli dlq stats` (depth, pending, dead letters) and the admin Queue page (backlog, dead
letters with reason and attempts, Redis used/max bytes and percent, frontier waiting/deferred,
capacity). No Prometheus exporter for Redis is deployed yet.

## 8. Security

Internal network only, never a published port beyond the dev host mapping; no user query touches
this Redis. `requirepass`/ACLs and command renaming from the original design are **not** set up
(2026-08-27) — the compose network is the boundary.

## 9. Testing

`crates/xustive-queue/tests/streams.rs` (produce/consume/ack/reclaim/DLQ against a real Redis,
skipped when unreachable), `tests/indexer.rs`, `tests/breaker_redis.rs`.

## 10. Open Questions

- [ ] Raw bodies (`frontier:raw:*`) belong in object storage, not here; today they are off by
      default for exactly that reason ([[Web Fetcher]]).
- [ ] A single Redis is the ingestion SPOF; search survives its loss, so Sentinel waits on how
      much crawl downtime is tolerable.
- [ ] ACL users per binary.

## Related

[[ADR-0006 - Redis Streams for the Ingestion Pipeline]] · [[Indexer Worker]] ·
[[Crawler Orchestrator]] · [[Crawler Console]] · [[Deduplication Service]] ·
[[Interaction Signals]] · [[Error Handling and Resilience]] · [[Deployment Topology]]
