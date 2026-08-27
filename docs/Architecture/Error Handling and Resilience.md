---
tags:
  - architecture
  - ops
type: architecture
status: implemented
updated: 2026-08-27
---

# Error Handling and Resilience

> The rules every component follows when something breaks. Component-specific tables live in each
> note's `## 7. Failure Modes` section; this note defines the shared vocabulary.
>
> Audited against the code on 2026-08-27. Where the original 2026-08-06 design and the code
> disagree, the code is described and the design is kept, marked as superseded, because the *why*
> still applies.

---

## 1. Error Taxonomy

| Class | Retryable? | Example | Handling |
|:---|:---|:---|:---|
| **Transient** | yes | 5xx, 408/425, connection reset, timeout | retry with backoff |
| **Throttled** | yes, slower | 429, `Retry-After` | honour header, open circuit |
| **Permanent** | no | other 4xx (404, 410), malformed HTML, unsupported codec | mark and drop, don't retry |
| **Poison** | no | payload that panics the parser | DLQ + fixture for regression test |
| **Degraded** | n/a | model overloaded, index slow | shed the optional part, serve the rest |
| **Fatal** | no | config invalid at boot, model file missing | fail fast, exit non-zero |

Rust representation: `xustive_core::error::ErrorClass` plus a `Classify` trait that every crate's
`thiserror` enum implements. The retry layer switches on `ErrorClass`, never on a string.
`ErrorClass::is_retryable()` is true only for Transient and Throttled; `is_dead_letter()` only for
Poison — Permanent and Poison are both non-retryable but are disposed of differently.
`class_for_status(u16)` is the one HTTP-status mapping, used by the fetcher and the search client.

```rust
pub enum ErrorClass { Transient, Throttled, Permanent, Poison, Degraded, Fatal }
```

The serving side has its own small enum, `xustive_api::error::ApiError`, whose stable `code` strings
are the contract with the frontend ([[API Contract]]): `invalid_query`, `query_too_long`,
`invalid_filter` (400), `search_unavailable` (503), `upstream_timeout` (504), `internal_error`
(500), plus per-feature codes carried by `Untranslatable` (400), `ModelUnavailable` (503) and
`BadImage` (422). A `SearchError` from Meilisearch maps by class: timeout → `upstream_timeout`,
unreachable / 5xx / Transient / Throttled → `search_unavailable`.

---

## 2. Retry Policy

The design (2026-08-06) was exponential backoff with full jitter, `delay = rand(0, min(cap,
base · 2^attempt))`, with these budgets:

| Context | base | cap | max attempts |
|:---|:---|:---|:---|
| HTTP fetch ([[Web Fetcher]]) | 1 s | 60 s | 4 |
| Social connector | 5 s | 300 s | 3 |
| Meilisearch write ([[Indexer Worker]]) | 200 ms | 10 s | 5 |
| Redis command | 50 ms | 2 s | 3 |
| Model inference ([[Summarizer]]) | — | — | **0** (fail fast, shed) |

What exists today (2026-08-27) is simpler, and the table above is superseded except for its
rules:

- **Fetch.** `xustive-ingest::fetch` classifies every outcome (`FetchError::is_retryable`) but
  has no inline backoff loop; a `gone` (404/410) is dropped by the orchestrator without retry, and
  a retryable outcome is left to the frontier's next claim rather than retried on the spot. The
  per-host crawl-delay (robots or the 1.5 s default) is the spacing between attempts.
- **Indexing.** Delivery-level: a job is dead-lettered after `MAX_ATTEMPTS = 3` deliveries
  (`xustive_queue::dlq`). A batch that Meilisearch rejects is **bisected** rather than retried
  whole — split, retry each half, recurse — so one bad document is isolated in about nine
  attempts for a batch of five hundred and the other 499 land. A *timeout* is not bisected: halving
  a batch does not make a busy index answer faster, and doing so once discarded 125 real documents.
- **Social connectors.** Not built ([[ADR-0009 - Direct Collection for Social Platforms]] is
  design only).
- **Model inference.** Zero retries, as designed: the summary is a separate request and is shed.

Rules that still hold:
- **Never retry a Permanent error.** Attempting it wastes budget and looks like abuse to the host.
- Retries are counted in the [[Task Queue]] envelope (`Delivery.attempts`, from the stream's
  pending-entry list), not in memory — a worker crash must not reset the counter. A job reclaimed
  by `XAUTOCLAIM` is reported with the delivery count Redis has for it.
- The "global retry budget → open circuit" rule (> 20 % retries to a host trips the breaker) is
  **not implemented**; breakers count consecutive failures (§3).

---

## 3. Circuit Breakers

Three implementations share one state machine — Closed → Open at `failure_threshold` →
Half-open after a cooldown that doubles per trip up to a ceiling → Closed on a successful probe,
exactly one probe outstanding at a time:

| Breaker | Where | Scope | Defaults |
|:---|:---|:---|:---|
| `xustive_core::circuit::{Breaker, SharedBreaker}` | in-process | one per dependency | 5 failures, 2 s → 60 s |
| `xustive_queue::breaker::RedisBreaker` | Redis, Lua scripts | shared across worker replicas | same as above, plus a failure window and a probe TTL |
| `xustive_ingest::proxy::breaker` | Redis | `Host`, `Platform`, `Asn` | host/ASN base 60 s, platform base 900 s, ceiling 1 800 s |

The serving plane wraps every optional dependency in a `SharedBreaker` so a dead sidecar costs one
probe per cooldown instead of one timeout per request: STT sidecar and the Federation Gateway
(3 failures, 5 s → 60 s), the OCR sidecar, CLIP and text embedders (3 failures, 10 s → 120 s).
State is visible on `/api/v1/admin/status` and the [[Crawler Console]] integrations page.

The design's "one 429/403 opens the host breaker immediately" is handled on the fetch side by the
politeness layer observing `Retry-After` per host ([[Politeness and Robots]]) rather than by the
breaker itself. Owned by [[Proxy Manager]] and [[Politeness and Robots]].

---

## 4. Backpressure

Design (2026-08-06): queue depth as the single global load signal, with green/yellow/red/black
bands at 10k / 50k / 200k that throttled discovery and paused the crawl. Superseded 2026-08-26 by
PROB-001 ([[PROB-001 - Bounded Frontier and Queue]]), which bounds every store instead of
watching one number:

| Bound | Where | Default |
|:---|:---|:---|
| stream length (`MAXLEN ~`) | `queue.index_stream_max_len` | 100 000 entries |
| frontier size | `crawl.frontier_max_urls` | per config |
| pages per host, outlinks per page | `crawl.max_pages_per_host`, `crawl.max_outlinks_per_page` | per config |
| **Redis memory high-water** | `crawld` pauses above 85 % of `maxmemory` | `MEMORY_HIGH_WATER = 0.85` |

The memory high-water pause is the universal backstop: every other bound keeps normal operation
far below it, and it is what makes the OOM wall structurally unreachable by the crawler's own
writes. Paused rather than stopped — the frontier and in-flight claims are kept, so a restart
resumes. The operator can also pause explicitly (`POST /api/v1/admin/crawler/pause`).

`xustive_queue_depth` is the consumer group's **lag**, not `XLEN` — a stream keeps acknowledged
entries until trimmed, so its length counts work already done. The `QueueBacklog` alert fires at
depth > 50 000 for 30 minutes ([[Observability]]).

Never drop ingestion messages to relieve pressure — slow down the producer instead. The serving
plane is unaffected by any of this by design ([[System Architecture]]).

---

## 5. Dead Letter Queues

Every stream has a sibling `<stream>:dead` — for the one stream in use today, `q:index:dead`.
(The design's `q:dlq:<stage>` naming was never used; there is one queued stage, indexing, because
fetch → parse → dedup → enrich run in the crawl daemon's process, see [[Task Queue]].) A message
lands there when `attempts > MAX_ATTEMPTS (3)` or on a Poison error.

DLQ entry (`xustive_queue::dlq::DeadLetter`): `{ original_id, payload, attempts, reason,
failed_at }` — the payload exactly as it was, so a replay does not have to reconstruct it.
Ordering is deliberate: produce to the dead stream **first**, acknowledge the original second; the
failure mode that leaves a duplicate dead letter is recoverable, the one that loses a job is not.

Operator workflow:
1. `make dlq A=stats` / `make dlq A=peek` (`xustive-cli dlq`) or the [[Crawler Console]] queue
   page (`GET /api/v1/admin/queue`) — see the dominant `reason`.
2. Fix the cause (parser bug, selector change, source policy).
3. Add the payload to `tests/fixtures/poison/` as a regression test ([[Testing Strategy]]).
4. `make dlq A=replay`, or per letter from the console: `POST /api/v1/admin/queue/dead/replay`
   re-enqueues one entry and deletes it from the dead stream (enqueue first, delete second — the
   same argument as above); `POST /api/v1/admin/queue/dead/drop` is the only place a dead letter
   is discarded on purpose. `POST /api/v1/admin/queue/replay` re-queues the pending set.

Retention: the dead stream is trimmed to the same `max_len` as its parent, not by age (the design's
"30 days" is superseded). `xustive_queue_dead_letters` is a gauge; `DLQGrowth` alerts when it
increases by more than 10 in 15 minutes ([[Observability]]).

---

## 6. Graceful Degradation Ladder

The serving plane degrades in this order — each step is independently triggerable:

| Step | Trigger | Effect | User-visible |
|:---|:---|:---|:---|
| 1 | summary budget gone / `xustive-ml` saturated | drop AI summary | summary block absent, results normal |
| 2 | expansion budget gone | skip the expansion leg (and the semantic leg) | worse Arabizi / Darija recall |
| 3 | facet budget gone | return results without facet counts, `facets_degraded: true` | filter chips show no counts |
| 4 | rerank budget gone | results in the engine's own order | slightly worse ordering |
| 5 | Meilisearch slow | 504 `upstream_timeout` | error page ([[UI - States and Errors]]) |
| 6 | Meilisearch down | 503 `search_unavailable` | error page |

The design's step "comment index slow → search `documents` only" and "serve cached top queries
(60 s TTL)" are both absent: the `comments` index is never queried at search time (2026-08-27:
`matched_comments` is always empty, see [[Ranking and Relevance]]), and the result cache was
rejected on privacy grounds — a cache keyed by query hash is a query log with extra steps
([[ADR-0008 - No Query Logging]]).

**Invariant:** losing the summary, expansion, facets, or re-rank must never fail the request. Only
steps 5 and 6 return an error.

### How the ladder is enforced

One `Deadline` (`xustive_api::deadline`) is created per request from `api.timeout_search_ms`
(1 500 ms in `config/prod.toml`, 2 500 ms in `config/dev.toml`) and asked **before** each stage
starts. A stage is skipped when less than its share of the *total* budget remains — fractions, not
fixed milliseconds, so the ladder still holds when the budget is changed in configuration:

| Stage | Skipped below | Why this order |
|:---|:---|:---|
| Summary | 55 % | already a separate request; nearly free to abandon |
| Expansion | 35 % | costs Arabizi queries their results, so given up only after the summary |
| Facets | 20 % | the chips disappear; the results do not |
| Rerank | 8 % | worse order is still results |
| Retrieval | never | |

Passing "you have 1 500 ms" down a call chain gives every stage the full budget; a shared deadline
is what stops four polite stages from producing a six-second request. Each skip increments
`xustive_degraded_total{stage}`. The federation strip waits at most `federation.budget_ms`, capped
to what the deadline has left — with the budget equal to the timeout it once produced a 504
([[Milestone 9 - Images and Videos]] fix). The summary has its own budget, `ml.deadline_ms` (30 s), shared
between the external attempt and the local model; under 3 s remaining the local attempt is not
started.

Other per-call caps: Meilisearch client `search.timeout_ms` (1 200 ms dev); `/suggest`
`api.timeout_suggest_ms` (150 ms); OCR / STT / embedder sidecars `*.timeout_ms` (10–30 s) behind
breakers (§3). The design's 20 ms language-detection and 30 ms expansion caps were never separate
timers — both are synchronous, in-process and fast; the deadline covers them.

---

## 7. Idempotency

Every stage must be safely re-runnable, because Redis Streams guarantee *at-least-once* delivery.

| Stage | Idempotency key | Mechanism |
|:---|:---|:---|
| fetch | `url + fetch_window` | frontier `seen` set, rotated every `crawl.seen_rotate_days` |
| parse | `content_hash` | pure function of input |
| dedup | `content_hash` / simhash | set membership — naturally idempotent |
| enrich | `document.id` | overwrite fields, deterministic models |
| index | `document.id` | Meilisearch upsert by primary key |

Corollary: no stage may perform a non-idempotent side effect (counter increment, external POST).
Counters live in metrics, which tolerate double-counting at the observability level.

---

## 8. Crash and Restart Semantics

| Component | On crash | Data loss |
|:---|:---|:---|
| `xustive-api` | LB removes it via `readyz` (Meilisearch health); in-flight requests fail | none (stateless; the tool cache lives in Redis) |
| local model (`xustive-ml`, in the API process) | in-flight summaries drop → degradation step 1 | none |
| crawl daemon (`xustive-cli crawld`) | frontier and claims are in Redis; restart resumes | none |
| indexer worker (`xustive-cli worker`) | unacked jobs reclaimed after `RECLAIM_AFTER` = 5 min | none |
| `xustive-toold` | cards keep serving from cache; `ToolDataAgeing` fires after 90 min | none |
| sidecars (OCR, STT, CLIP, text-embed) | breaker opens; feature withheld | none |
| `redis` | AOF replay on boot; up to 1 s of writes lost | re-crawl work only |
| `meilisearch` | restarts from last snapshot | up to snapshot interval — restore per [[Deployment Topology]] |

Consumer group claim timeout is 5 minutes (`xustive_queue::RECLAIM_AFTER`): `XAUTOCLAIM` moves
messages from dead consumers to live ones in one round trip, which cannot race two reclaimers into
a double claim the way `XPENDING` + `XCLAIM` can. Set it above the slowest legitimate stage. (The
design cited a ~90 s headless fetch; there is no headless fetcher today.)

---

## 9. Anti-Patterns (rejected)

- ❌ Retrying on a generic `Error` without classification — turns a 404 into 4 wasted requests.
- ❌ `unwrap()` / `expect()` outside `main()` and tests. CI runs `cargo clippy --workspace
  --all-targets -- -D warnings`; the specific `clippy::unwrap_used` deny is **not** configured
  (2026-08-27), so this is convention, not enforcement.
- ❌ Swallowing errors with `let _ =`. Either handle, log-with-class, or propagate.
- ❌ Infinite retry loops with no jitter — synchronised thundering herds against one host.
- ❌ Failing a whole batch because one document is bad — the [[Indexer Worker]] bisects.
- ❌ A tool matcher that panics failing the search — `xustive_tools::best` catches and skips it.

---

## 10. Open Questions

- [x] Should the step-5 result cache exist at all, given [[Security and Privacy]]? — **No**
      (decided; never built). Answer caches are keyed by entity or wilaya, never by query.
- [ ] Per-source circuit breakers vs per-host: the proxy breaker has `Platform` and `Asn` scopes,
      but no social platform is collected yet ([[ADR-0009 - Direct Collection for Social Platforms]]).

## Related

[[Task Queue]] · [[Observability]] · [[Proxy Manager]] · [[Politeness and Robots]] ·
[[Performance Budgets]] · [[Deployment Topology]] · [[PROB-001 - Bounded Frontier and Queue]]
