---
tags:
  - component
  - platform
component-id: C20
binary: redis
status: specified
updated: 2026-08-06
---

# Task Queue

> **ID** C20 · **Service** `redis` · **Upstream** every ingestion component · **Downstream** every ingestion component

## 1. Purpose

The transport and the coordination substrate for the ingestion plane. It decouples every stage so
each can fail, restart, and scale independently, and it is the single place where load is visible as
a number ([[Error Handling and Resilience]] §4).

Redis also serves as the shared state store for the frontier, dedup keys, circuit breakers, and
cursors — one dependency instead of four.

## 2. Responsibilities

**In scope**: durable-enough at-least-once message delivery; consumer groups; dead-letter streams;
claim/redelivery of stalled messages; shared crawl state; queue-depth signalling.

**Out of scope**: being a database of record (that is [[Search Index]]); long-term storage; exactly-
once semantics (impossible; idempotency handles it — [[Error Handling and Resilience]] §7).

## 3. Interface

### Streams

| Stream | Producer | Consumer group | Payload |
|:---|:---|:---|:---|
| `q:fetch` | [[Crawler Orchestrator]] | `fetchers` | URL / social cursor |
| `q:parse` | [[Web Fetcher]], connectors | `parsers` | raw ref + fetch metadata |
| `q:enrich` | [[Content Parser]] (via [[Deduplication Service]]) | `enrichers` | pre-enrichment Document |
| `q:index` | [[Enrichment Pipeline]] | `indexers` | Document + Comments |
| `q:vector_retry` | [[Indexer Worker]] | `indexers` | embeddings awaiting Qdrant |
| `q:dlq:{stage}` | any | manual/CLI | failed message + error |

Envelope shape in [[Data Model]] §6.

### Other key spaces

| Pattern | Type | Owner | TTL |
|:---|:---|:---|:---|
| `frontier:{host}`, `frontier:hosts` | zset | [[Crawler Orchestrator]] | — |
| `seen:{host}` | Bloom | [[Crawler Orchestrator]] | 180 d |
| `crawl:{host}` | hash | [[Politeness and Robots]] | 7 d |
| `robots:{host}` | string | [[Politeness and Robots]] | 24 h |
| `breaker:{host}` / `breaker:platform:{p}` | hash | [[Proxy Manager]] | cooldown |
| `dedup:*`, `simhash:*`, `phash:*` | set/hash/Bloom | [[Deduplication Service]] | 180 d |
| `cursor:{source_id}` | string | connectors | — |
| `raw:{trace_id}` | string (zstd) | [[Web Fetcher]] | 7 d |
| `proxy:{id}` | hash | [[Proxy Manager]] | — |

## 4. Internal Design

### 4.1 Why Streams, not Lists

`XADD`/`XREADGROUP` gives consumer groups, per-message acknowledgement, and a pending-entries list —
so a worker that dies mid-message does not lose it. `LPUSH`/`BRPOP` loses in-flight work on crash.
See [[ADR-0006 - Redis Streams for the Ingestion Pipeline]].

### 4.2 Consumer pattern

```rust
loop {
    let msgs = XREADGROUP GROUP {group} {consumer} COUNT {n} BLOCK 5000 STREAMS {stream} ">";
    for m in msgs { match process(m) {
        Ok(_)                  => XACK stream group m.id,
        Err(e) if e.retryable() => { /* leave unacked → redelivered */ }
        Err(e)                 => { XADD dlq(...); XACK stream group m.id }
    }}
    // periodically reclaim stalled work from dead consumers
    XAUTOCLAIM stream group {consumer} 300000 0 COUNT 100;
}
```

`XAUTOCLAIM` idle time is **5 minutes**, chosen to exceed the slowest legitimate stage (a headless
render, ~90 s) with margin. Set it too low and healthy slow work gets duplicated.

### 4.3 Trimming

Acked entries are not removed automatically. A maintenance task runs `XTRIM MINID` against the
lowest pending id every 60 s, keeping streams bounded. Without this, Redis memory grows forever —
this is the single most common Streams operational mistake.

### 4.4 Persistence

AOF `appendfsync everysec` + RDB snapshot hourly. Redis loss costs at most 1 s of queue state and
some re-crawl work; it never costs indexed data ([[Deployment Topology]] §7).

`maxmemory-policy noeviction` — **critical**. Under `allkeys-lru`, Redis would silently evict queue
entries and frontier state under memory pressure, losing work invisibly. With `noeviction`, writes
fail loudly and backpressure engages.

### 4.5 Backpressure signalling

`XLEN` per stream is exported as `xustive_queue_depth{stream}`; the age of the oldest pending entry
is `xustive_queue_lag_seconds`. [[Crawler Orchestrator]] reads these and throttles per the thresholds
in [[Error Handling and Resilience]] §4.

## 5. Configuration

| Key | Default | Notes |
|:---|:---|:---|
| `maxmemory` | 6 GB | leave headroom on an 8 GB allocation |
| `maxmemory-policy` | `noeviction` | non-negotiable |
| `appendonly` | `yes`, `everysec` | |
| `stream_max_len` | 500 000 (approx trim) | safety net above the trim task |
| `claim_idle_ms` | 300 000 | |
| `consumer_batch` | 32 | |
| `block_ms` | 5 000 | |
| `dlq_retention_days` | 30 | |
| `raw_ttl_days` | 7 | |

## 6. Data

See §3. Estimated steady-state memory at 10M documents: streams ~500 MB, dedup ~1.2 GB, frontier
~800 MB, raw blobs ~2–4 GB (the dominant and most variable term — see [[Web Fetcher]] §12).

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Redis down | connection error | ingestion halts entirely; **search is unaffected** ([[System Architecture]] §1) |
| Memory limit hit | `OOM command not allowed` | writes fail → producers backpressure → alert |
| Stream growth unbounded | `XLEN` metric | trim task; investigate a stuck consumer group |
| Consumer group lag | `XPENDING` | scale consumers; check for a poison message |
| Poison message loops | attempt counter in envelope | DLQ after `max_attempts` |
| Stalled consumer (hung, not dead) | `XAUTOCLAIM` idle | reclaimed after 5 min |
| AOF corruption | startup failure | `redis-check-aof --fix`; worst case, re-seed the frontier |
| Split-brain (two orchestrator leaders) | leader lock TTL | lock is single-instance; documented limitation of a single Redis |

## 8. Performance

| Metric | Budget |
|:---|:---|
| `XADD` throughput | ≥ 50 000/s |
| `XREADGROUP` batch of 32 | ≤ 2 ms |
| End-to-end queue latency (enqueue → claimed) | ≤ 500 ms p95 at green pressure |
| Memory | ≤ 6 GB steady |

## 9. Observability

`xustive_queue_depth{stream}`, `xustive_queue_lag_seconds{stream}`,
`xustive_queue_pending{stream,group}`, `xustive_dlq_total{stage}`, `xustive_claim_total`, plus
`redis_exporter` metrics (memory, ops/s, AOF size, evicted keys — which must stay **0**).

`redis_evicted_keys_total > 0` is an immediate alert: it means `noeviction` was misconfigured and
work is being lost silently.

## 10. Security

Internal network only, never a published port ([[Security and Privacy]] T5). `requirepass` set;
dangerous commands (`FLUSHALL`, `FLUSHDB`, `CONFIG`, `KEYS`, `DEBUG`) renamed or disabled via ACL.
Separate ACL users per binary with the minimum key-pattern and command set each needs.

**No user queries ever touch Redis.** The serving plane's only Redis use is rate-limit counters keyed
on a salted IP hash with a 60 s TTL, and those live in a separate logical database
([[Security and Privacy]] P5).

## 11. Testing

- Integration with a real Redis container: produce 10k messages, consume with 4 workers, assert
  exactly-once *effect* (idempotent) and zero loss.
- Crash: kill a consumer mid-message; assert `XAUTOCLAIM` redelivers within the idle window.
- Poison: a message that always fails; assert it reaches the DLQ after `max_attempts` and does not
  block the stream.
- Memory: fill to `maxmemory`; assert writes fail loudly and no key is evicted.
- Trim: run for 1M messages; assert stream length stays bounded.
- Replay: `dlq replay` restores messages into the stage input and they process normally.

## 12. Open Questions

- [ ] Move `raw:{trace_id}` blobs out of Redis to object storage/disk? At high crawl rates they
      dominate memory ([[Web Fetcher]] §12).
- [ ] Is a single Redis instance acceptable, or do we need Sentinel before beta? (Redis is the
      ingestion SPOF; search survives its loss, so the answer depends on tolerable crawl downtime.)
- [ ] Should DLQ replay be automated with a safety valve, or stay a deliberate manual action?

## Related

[[Error Handling and Resilience]] · [[Crawler Orchestrator]] · [[Indexer Worker]] ·
[[Deduplication Service]] · [[Observability]] · [[Deployment Topology]] ·
[[ADR-0006 - Redis Streams for the Ingestion Pipeline]]
