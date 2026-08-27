---
tags:
  - adr
adr-id: "0001"
status: implemented
date: 2026-08-06
---

# ADR-0001 - Two-Plane Architecture

## Status

Implemented; amended by [[ADR-0017 - Query-Time Federation with External Metasearch]] (one allowlisted internal hop to the Federation Gateway). Constrains [[System Architecture]], [[Component Map]], [[Deployment Topology]].

## Context

Xustive does two very different things. Serving a search is synchronous, latency-bound, and must be
boringly reliable. Crawling is asynchronous, throughput-bound, and *inherently* unreliable — hosts go
down, platforms rate-limit, parsers break on redesigns, proxies fail.

The tempting design is one service that crawls and serves, sharing code and a process. It is simpler
to start and it fails badly: a crawl burst starves search threads, a parser panic takes down the
public API, and scaling for crawl throughput means over-provisioning the API.

## Decision

Two planes, coupled **only** through the index:

- **Serving plane** — stateless, latency-bound: [[API Gateway]] → [[Query Pipeline]] → [[Search Index]].
- **Ingestion plane** — queue-driven, throughput-bound: [[Crawler Orchestrator]] → fetchers → workers
  → [[Indexer Worker]].

No ingestion component may call the serving API. No serving component may call an ingestion
component. They communicate exclusively by one writing to the index and the other reading it.

Separate binaries, separate scaling, separate failure domains, separate Docker networks
([[Deployment Topology]] §3).

## Consequences

**Good**
- Ingestion can be entirely down and search still serves — the index is already there.
- Search can be down and ingestion keeps filling the index.
- Each plane scales on its own signal: rps for serving, queue depth for ingestion.
- The ingestion SLO can be loose (95 %) while search is tight (99.5 %) —
  [[Performance Budgets]] §8.
- Crawler egress is confined to one network, which is what makes "no user data leaves the country"
  structurally enforceable ([[Security and Privacy]] P2).

**Bad**
- Shared logic must live in shared crates (`xustive-core`, `xustive-text`) with real API discipline,
  not casual reuse.
- More binaries, more containers, more deployment surface.
- Index freshness becomes an explicit SLO rather than an implicit property.

**Commits us to**
- The index being the only integration point — any future "ingestion asks search a question"
  requirement is a smell that should be solved by putting the answer in the index instead.

## Alternatives

| Option | Why not |
|:---|:---|
| Single monolith | crawl load degrades search latency; a parser panic is a search outage |
| Serving calls ingestion for on-demand crawl | unbounded latency on a user-facing path; trivially abusable |
| Fully separate deployments with no shared code | duplicate normalisation logic, which breaks Arabic search silently ([[Content Parser]] §4.4) |

## Revisit when

- Ingestion needs to read live search state for something other than debugging.
- Operational overhead of four binaries measurably exceeds the isolation benefit (unlikely below
  team size ~10).

## Related

[[System Architecture]] · [[Component Map]] · [[Deployment Topology]] · [[Performance Budgets]] ·
[[Decision Log]]

## Where it stands (2026-08-27)

- Planes are separate binaries: `crates/xustive-api` (serving) and `crates/xustive-cli` (`crawld`, `toold`, ingestion commands). They share only Meilisearch, Qdrant and Redis; no serving code calls ingestion code (`crates/xustive-api/src` has no `xustive_ingest` dependency beyond the leaf `xustive-federation` client re-export).
- Amended by ADR-0017: `xustive-api` gains one outbound target, `xustive-federator` (`crates/xustive-federator`, on `core` + `ingest` networks in `deploy/docker-compose.yml`). SearXNG sits on `ingest` only, behind the `federation` compose profile.
- Divergence in deployment, not in code: `deploy/docker-compose.yml` defines no `api` service — `xustive-api` runs on the host and reaches the containers over published ports. The "separate Docker networks" property therefore holds for the data stores (`core` is `internal: true`) but the API's own egress is bounded by host configuration, not by compose (see [[ADR-0008 - No Query Logging]] Where it stands).
- The summariser is not a separate `xustive-ml` process: `crates/xustive-ml` is a library linked into `xustive-api` (no `main.rs`).
