---
tags:
  - adr
  - search
  - scaling
status: accepted
date: 2026-08-29
---
# ADR-0035 - Sharding in Our Own Router

> Part of [[Decision Log]] · Milestone: [[Milestone 14 - One Server, Many Hands]] ·
> Follows [[ADR-0002 - Meilisearch as System of Record]] · Components: [[Search Index]],
> [[Query Pipeline]], [[Deployment Topology]]

## Context

One Meilisearch holds the corpus, and [[Problems#PROB-004|PROB-004]] showed what happens when
the index outgrows the machine's memory: indexing fell from 260 documents a minute to 8, because
a memory-mapped index that does not fit the page cache is read back from disk on every batch. The
fix was more memory. At 300k documents the index is 12.5 GB; the corpus this engine wants is
tens of millions.

Meilisearch itself now offers sharding — a `/network` route, named shards assigned to remotes, a
leader that coordinates writes, and federated search that fans out and merges
([documentation](https://www.meilisearch.com/docs/learn/multi_search/implement_sharding)). It
requires **Enterprise Edition v1.37 or later on every instance**. We run the open-source build.

Our API is already a fan-out-and-merge engine: it runs a lexical leg, a conditional expansion
leg, a dense leg fused by reciprocal rank, and a federation strip, then re-ranks the union with
its own signals ([[Ranking and Relevance]]). Merging N shards is the same shape as merging those
legs, and `_rankingScore` is normalised to `[0, 1]`, so scores from different shards are
comparable by construction.

## Decision

1. **Shard in the router.** `search.shards` lists Meilisearch instances; **rendezvous hashing**
   on the document id picks the owner, so adding a shard moves about `1/N` of the corpus and
   nothing else moves at all.
2. **Reads fan out, merge, then re-rank as today.** Each shard is asked for the candidate pool;
   results merge on `_rankingScore`, facet counts and totals sum, and the existing re-rank runs
   over the union. No ranking code changes — that is the test of whether this is right.
3. **Writes route by id.** The indexer worker, the endorse sink, the hits counters and takedowns
   all resolve the shard from the document id. `migrate` applies settings to every shard, and the
   settings are identical by construction.
4. **A slow or dead shard is a degradation, not an error.** It joins the deadline ladder like
   every other stage: partial results, `degraded{stage="shard"}`, a console tile — never a 5xx,
   which is the rule [[ADR-0027 - Narrow the Search Under Load Instead of Failing]] already set.
5. **Each shard keeps the PROB-004 rule**: its container's memory limit must exceed its own
   `usedDatabaseSize`. Sharding is how that rule stays satisfiable as the corpus grows.
6. **Enterprise sharding stays the documented alternative.** If a licence is ever bought, the
   router collapses to one remote and Meilisearch does the fan-out; nothing else in the system
   depends on which one is in use.

## Consequences

- Horizontal growth without a licence, on the hardware a self-hoster can actually buy.
- Search latency becomes the *slowest* shard's latency plus a merge, so shard sizing matters and
  a stray slow shard is visible in p95. The deadline ladder bounds the damage.
- Two things get harder: exact `estimatedTotalHits` (it is a sum of estimates, and the response
  already says `estimated`) and rebalancing (a background job that moves documents when `N`
  changes, with the alias flip pattern per shard).
- Facet counts and pagination are already computed after merge, so neither changes.
- We own a piece of distributed-systems code we would rather not own. It is roughly two hundred
  lines and it is testable without a cluster, which is what makes it the cheaper of the two.

## Related

[[Search Index]] · [[Query Pipeline]] · [[Deployment Topology]] ·
[[ADR-0027 - Narrow the Search Under Load Instead of Failing]]
