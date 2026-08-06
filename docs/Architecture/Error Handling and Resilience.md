---
tags:
  - architecture
  - ops
type: architecture
status: specified
updated: 2026-08-06
---

# Error Handling and Resilience

> The rules every component follows when something breaks. Component-specific tables live in each
> note's `## 7. Failure Modes` section; this note defines the shared vocabulary.

---

## 1. Error Taxonomy

| Class | Retryable? | Example | Handling |
|:---|:---|:---|:---|
| **Transient** | yes | 503, connection reset, timeout | retry with backoff |
| **Throttled** | yes, slower | 429, `Retry-After` | honour header, open circuit |
| **Permanent** | no | 404, 410, malformed HTML, unsupported codec | mark and drop, don't retry |
| **Poison** | no | payload that panics the parser | DLQ + fixture for regression test |
| **Degraded** | n/a | model overloaded, index slow | shed the optional part, serve the rest |
| **Fatal** | no | config invalid at boot, model file missing | fail fast, exit non-zero |

Rust representation: one `thiserror` enum per crate, each variant carrying `ErrorClass`. The retry
layer switches on `ErrorClass`, never on a string.

```rust
pub enum ErrorClass { Transient, Throttled, Permanent, Poison, Degraded, Fatal }
```

---

## 2. Retry Policy

Exponential backoff with full jitter:

```
delay = rand(0, min(cap, base · 2^attempt))
```

| Context | base | cap | max attempts |
|:---|:---|:---|:---|
| HTTP fetch ([[Web Fetcher]]) | 1 s | 60 s | 4 |
| Social connector | 5 s | 300 s | 3 |
| Meilisearch write ([[Indexer Worker]]) | 200 ms | 10 s | 5 |
| Redis command | 50 ms | 2 s | 3 |
| Model inference ([[Summarizer]]) | — | — | **0** (fail fast, shed) |

Rules:
- **Never retry a Permanent error.** Attempting it wastes budget and looks like abuse to the host.
- Retries are counted in the [[Task Queue]] envelope (`attempt`), not in memory — a worker crash must
  not reset the counter.
- Retry budget is global: if > 20 % of requests to a host are retries, the circuit opens (§3) rather
  than each worker retrying independently.

---

## 3. Circuit Breakers

One breaker per **(host or platform, proxy pool)** pair, state in Redis so all crawler replicas share
it.

| State | Entry condition | Behaviour | Exit |
|:---|:---|:---|:---|
| `closed` | default | pass through | — |
| `open` | ≥ 5 failures in 60 s, or one 429/403 | reject immediately, requeue with delay | after cooldown |
| `half_open` | cooldown elapsed | allow 1 probe request | success → `closed`; failure → `open`, cooldown ×2 |

Cooldown: 60 s initial, doubling to a 30-minute ceiling. Owned by [[Proxy Manager]] and
[[Politeness and Robots]].

---

## 4. Backpressure

Queue depth is the **single global load signal**. Thresholds per stream ([[Task Queue]]):

| Level | Depth | Action |
|:---|:---|:---|
| green | < 10k | normal |
| yellow | 10k–50k | [[Crawler Orchestrator]] halves its dispatch rate |
| red | 50k–200k | discovery stops; only in-flight work drains |
| black | > 200k | crawlers pause entirely; alert fires |

Never drop ingestion messages to relieve pressure — slow down the producer instead. The serving
plane is unaffected by any of this by design ([[System Architecture]]).

---

## 5. Dead Letter Queues

Every stage has `q:dlq:<stage>`. A message lands there when `attempt > max_attempts` or on a Poison
error.

DLQ entry: `{ original, error, error_class, attempts, failed_at, worker_id, stage }`.

Operator workflow:
1. `xustive-cli dlq stats` — group by `error_class` + host; find the dominant class.
2. Fix the cause (parser bug, selector change, source policy).
3. Add the payload to `tests/fixtures/poison/` as a regression test ([[Testing Strategy]]).
4. `xustive-cli dlq replay --stage parse --since 24h` — replays into the stage's input stream.

DLQ retention: 30 days. `xustive_dlq_total` alerts at > 10/min ([[Observability]]).

---

## 6. Graceful Degradation Ladder

The serving plane degrades in this order — each step is independently triggerable:

| Step | Trigger | Effect | User-visible |
|:---|:---|:---|:---|
| 1 | `xustive-ml` saturated | drop AI summary | summary block absent, results normal |
| 2 | [[Query Expander]] slow (> 30 ms) | skip expansion | slightly worse Darija recall |
| 3 | comment index slow | search `documents` only | no `matched_comments` on cards |
| 4 | facet computation slow | return results without facet counts | filter chips show no counts |
| 5 | Meilisearch degraded | serve cached top queries (60 s TTL) | stale results, banner shown |
| 6 | Meilisearch down | 503 `search_unavailable` | error page ([[UI - States and Errors]]) |

**Invariant:** losing the summary, expansion, comments, or facets must never fail the request. Only
step 6 returns an error.

Timeouts enforcing the ladder:

| Call | Timeout | On timeout |
|:---|:---|:---|
| language detection | 20 ms | assume `und`, continue |
| query expansion | 30 ms | use raw query |
| Meilisearch search | 800 ms | 504 `upstream_timeout` |
| facet computation | 150 ms | omit facets |
| summary TTFT | 3 s | close SSE with `error` |
| whole `/search` | 1.5 s | 504 |

---

## 7. Idempotency

Every stage must be safely re-runnable, because Redis Streams guarantee *at-least-once* delivery.

| Stage | Idempotency key | Mechanism |
|:---|:---|:---|
| fetch | `url + fetch_window` | dedup on `crawl:{host}:seen` |
| parse | `content_hash` | pure function of input |
| dedup | `content_hash` | set membership — naturally idempotent |
| enrich | `document.id` | overwrite fields, deterministic models |
| index | `document.id` | Meilisearch upsert by primary key |

Corollary: no stage may perform a non-idempotent side effect (counter increment, external POST).
Counters live in metrics, which tolerate double-counting at the observability level.

---

## 8. Crash and Restart Semantics

| Component | On crash | Data loss |
|:---|:---|:---|
| `xustive-api` | LB removes it via `readyz`; in-flight requests fail | none (stateless) |
| `xustive-ml` | in-flight summaries drop → degradation step 1 | none |
| `xustive-crawler` | unacked messages redelivered after 5 min | none |
| `xustive-worker` | same | none |
| `redis` | AOF replay on boot; up to 1 s of writes lost | re-crawl work only |
| `meilisearch` | restarts from last snapshot + WAL | up to snapshot interval — restore per [[Deployment Topology]] |

Consumer group claim timeout is 5 minutes: `XAUTOCLAIM` moves messages from dead consumers to live
ones. Set it above the slowest legitimate stage (headless fetch, ~90 s) to avoid duplicate work.

---

## 9. Anti-Patterns (rejected)

- ❌ Retrying on a generic `Error` without classification — turns a 404 into 4 wasted requests.
- ❌ `unwrap()` / `expect()` outside `main()` and tests. Enforced by `clippy::unwrap_used` at deny level.
- ❌ Swallowing errors with `let _ =`. Either handle, log-with-class, or propagate.
- ❌ Infinite retry loops with no jitter — synchronised thundering herds against one host.
- ❌ Failing a whole batch because one document is bad — [[Indexer Worker]] splits and isolates.

---

## 10. Open Questions

- [ ] Should the step-5 result cache exist at all, given [[Security and Privacy]]? (a cache keyed by
      query hash is a query log with extra steps — currently leaning **no**)
- [ ] Per-source circuit breakers vs per-host: a Facebook block affects all groups at once.

## Related

[[Task Queue]] · [[Observability]] · [[Proxy Manager]] · [[Politeness and Robots]] ·
[[Performance Budgets]] · [[Deployment Topology]]
