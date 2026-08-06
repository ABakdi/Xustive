---
tags:
  - adr
adr-id: "0006"
status: accepted
date: 2026-08-06
---

# ADR-0006 - Redis Streams for the Ingestion Pipeline

## Status

Accepted. Constrains [[Task Queue]], every ingestion component.

## Context

The ingestion plane is a five-stage pipeline (fetch → parse → dedup → enrich → index) whose stages
have wildly different costs: a parse is 25 ms, a headless render is 12 s, an enrich with four images
is 2 s. They must scale independently, survive crashes without losing work, and expose a load signal.

The candidates were: Redis Lists, Redis Streams, a real broker (RabbitMQ/NATS/Kafka), or a database
table as a queue.

We already need Redis for the crawl frontier, dedup keys, circuit breakers, and cursors
([[Task Queue]] §3). Adding a second infrastructure component has to earn its place.

## Decision

**Redis Streams with consumer groups** as the transport for every stage.

- One stream per stage (`q:fetch`, `q:parse`, `q:enrich`, `q:index`), one consumer group per stage.
- `XREADGROUP` to claim, `XACK` only after the work is durably done.
- `XAUTOCLAIM` with a 5-minute idle window reclaims work from dead consumers.
- A dead-letter stream per stage, with attempt counts carried in the message envelope.
- A trim task keeps streams bounded.
- `maxmemory-policy noeviction` — non-negotiable.

## Consequences

**Good**
- One infrastructure component covers queueing *and* all the shared crawl state.
- Per-message acknowledgement: a worker that dies mid-message does not lose it — the pending-entries
  list holds it until reclaimed.
- `XLEN` per stream is a free, accurate load signal, which is what the entire backpressure design is
  built on ([[Error Handling and Resilience]] §4).
- Consumer groups give horizontal scaling with no coordination code.
- Operationally simple: one process to run, back up, and understand.

**Bad**
- **At-least-once only.** Every stage must be idempotent, and we have to enforce that discipline
  ourselves ([[Error Handling and Resilience]] §7).
- Redis is memory-bound: streams, dedup keys, frontier, and raw blobs all compete for the same
  6 GB. Raw blobs are the term that will force a decision first ([[Web Fetcher]] §12).
- Acked entries are not auto-removed — forgetting the trim task means unbounded growth. This is the
  most common Streams operational mistake and it is called out explicitly in [[Task Queue]] §4.3.
- Single-instance Redis is the ingestion SPOF. Acceptable because search survives its loss
  ([[ADR-0001 - Two-Plane Architecture]]), but it is a real limitation.
- Weaker durability than a broker with disk-first semantics: AOF `everysec` can lose ~1 s of writes.

**Commits us to**
- Idempotency at every stage, tested ([[Testing Strategy]] §4).
- Treating queue depth as *the* load signal rather than inventing per-component ones.

## Alternatives

| Option | Why not |
|:---|:---|
| Redis Lists (`LPUSH`/`BRPOP`) | in-flight work is lost on a consumer crash — disqualifying |
| RabbitMQ | good semantics, but a second component to operate for capability we mostly get free |
| Kafka | built for a scale and a retention model we do not have; heavy ops burden |
| NATS JetStream | genuinely good fit; rejected only because Redis was already required |
| Postgres table as a queue | we have no Postgres ([[ADR-0002 - Meilisearch as System of Record]]) |

## Revisit when

- Redis memory pressure from raw blobs forces object storage anyway — at which point a dedicated
  broker's cost changes.
- Ingestion volume exceeds ~50k messages/s sustained.
- We need stronger delivery guarantees than at-least-once (unlikely; idempotency is cheaper).

## Related

[[Task Queue]] · [[Error Handling and Resilience]] · [[Crawler Orchestrator]] · [[Indexer Worker]] ·
[[ADR-0001 - Two-Plane Architecture]] · [[Decision Log]]
