---
tags:
  - adr
adr-id: "0002"
status: accepted
date: 2026-08-06
---

# ADR-0002 - Meilisearch as System of Record

## Status

Accepted. Constrains [[Search Index]], [[Data Model]], [[Indexer Worker]].

## Context

The obvious architecture is PostgreSQL as the store of truth plus a search index derived from it.
That gives transactions, joins, and a clean reindex path — at the cost of running, backing up,
migrating, and keeping two stores consistent.

But look at what we actually store: documents that are **derived from the public internet**. If we
lose them, we can re-crawl them. There is no user-generated data, no transactional state, no
financial record, nothing that cannot be reconstructed. The usual argument for a durable relational
system of record — "this data exists nowhere else" — does not apply.

Meanwhile every write path we have is an idempotent upsert by primary key, and every read is a
search. That is exactly the shape Meilisearch is good at.

## Decision

**Meilisearch is the system of record.** There is no SQL database in v1.

- `documents`, `comments`, `sources` live in Meilisearch ([[Data Model]]).
- Image vectors live in Qdrant ([[Vector Index]]), joined by id.
- Operational state (frontier, dedup keys, cursors) lives in Redis ([[Task Queue]]).
- The source registry is additionally mirrored to git so the *curation* — the part that is genuinely
  irreplaceable — is versioned and reviewable ([[Data Sources Registry]] §9).

Durability comes from Meilisearch snapshots every 6 h plus the ability to re-crawl
([[Deployment Topology]] §7).

## Consequences

**Good**
- One store to run, back up, and reason about instead of three.
- No dual-write consistency problem between a database and an index.
- Reindexing is an alias flip, not a migration dance ([[Data Model]] §7).
- Substantially less code: no ORM, no schema migrations, no connection pooling to tune.

**Bad**
- No transactions. A document and its comments are written as separate operations, so a crash between
  them leaves a brief inconsistency. Acceptable: the next crawl repairs it.
- No joins. Comment→document association is resolved in [[Query Pipeline]] with a second query.
- No ad-hoc SQL for investigation — analysis happens through the search API or an export.
- Reindexing from scratch means re-crawling, which is days rather than hours.
- Meilisearch's own durability guarantees (snapshot + WAL) are weaker than a database's.

**Commits us to**
- Every stored entity being re-derivable from a crawl. Any future feature that needs data which
  exists *only* in Xustive (user accounts, saved searches, editorial annotations) breaks this
  assumption and forces a real database. That is the trip-wire.

## Alternatives

| Option | Why not |
|:---|:---|
| Postgres + Meilisearch | dual-write consistency, two backup stories, more ops, for data we can re-crawl |
| Postgres with `pg_trgm`/`tsvector` only | poor Arabic tokenisation, no typo tolerance, worse latency at 10M docs |
| Tantivy directly (library, no server) | more control, but we would be writing the server ourselves |
| Elasticsearch/OpenSearch | heavier, JVM ops burden, licence complexity, far more tuning to reach the same result |

## Revisit when

- We need to store data that cannot be re-derived by crawling — **this is the hard trip-wire**.
- Cross-entity queries become common enough that in-application joins dominate latency.
- Index size passes ~50M documents, where Meilisearch's operational envelope needs re-evaluation.

## Related

[[Search Index]] · [[Data Model]] · [[Indexer Worker]] · [[Deployment Topology]] ·
[[ADR-0003 - Comments in a Separate Index]] · [[Decision Log]]
