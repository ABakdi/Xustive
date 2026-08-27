---
tags: [adr]
adr-id: "0027"
status: accepted
date: 2026-08-26
---
# ADR-0027 - Narrow the Search Under Load Instead of Failing

## Status

Accepted, implemented (BUG-041). Constrains [[API Gateway]], [[Query Pipeline]],
[[Error Handling and Resilience]] and [[Performance Budgets]]. Extends the deadline ladder
[[ADR-0004 - Stream Summary Separately from Results]] began (drop the summary first) down to the
retrieval stage itself.

## Context

The search path already degraded under load in stages — drop the summary, the expansion leg, the
facets, the re-rank — and still answered. Retrieval itself was the one stage that could not
degrade: a Meilisearch timeout was a 504 reading "That search took too long". That is exactly the
stage a busy engine shows on. While Meilisearch is also indexing a crawl backlog, the
200-candidate query with facets and highlighting takes several hundred milliseconds and trips the
engine timeout; a page's worth with nothing extra answers in ~30 ms even then.

A second, quieter bug made every degradation invisible: the transport timeout layer around
`/search` was set *equal* to the search deadline. Any slack consumed anywhere let the layer cut
the request first and answer a bare 504 with no body — which the web tier could only render as
"Search failed" — before the ladder had a chance to shape a degraded page.

## Decision

1. **On a retrieval timeout, retry once with a page-sized query and nothing extra** — no facets,
   no highlighting, limit `offset + hits_per_page` (capped by `MAX_TOTAL_HITS`) — keeping the
   reader's filters and sort. Worse ranking (a smaller pool to re-rank) and no filter chips, but
   results. The response marks `facets_degraded` so the page can say so.
2. **Count it.** `xustive_degraded_total{stage="retrieval"}` alongside the other ladder stages,
   so an operator can see the engine narrowing rather than learning from complaints.
3. **The transport cut sits above the search deadline, not on it.** `SEARCH_GRACE_MS = 1000` is
   added to `api.timeout_search_ms` for the `/search` timeout layer: the ladder needs that time to
   shape and send a degraded page after its last stage fires. The layer is now the backstop for a
   search that has genuinely hung, not the common case.
4. **Every sub-wait stays inside the deadline.** The federation strip wait and similar caps are
   configured well under `timeout_search_ms` so the ladder keeps the last say (the dev 504 bug
   from [[Milestone 9 - Images and Videos]] was this rule violated).

## Consequences

**Good**
- A reader cannot tell a search that returns nothing from an outage — that was the goal. Under
  load they get a page, then chips and better ranking come back as load drops.
- Degradation is measured per stage, so "the engine is narrowing" is a graph, not a feeling.

**Bad**
- A narrowed page has fewer candidates to re-rank and no facets; a reader on a busy day gets a
  slightly worse page and a chip-less filter bar. Accepted over a 504.
- The grace second means a truly hung search takes `timeout_search_ms + 1000` to fail, not
  `timeout_search_ms`.

## Alternatives

| Option | Why not |
|:---|:---|
| Raise the engine timeout | pushes the whole page toward the transport cut and hides the load rather than shaping it |
| Serve a cached page | a query-keyed cache is a query log ([[ADR-0008 - No Query Logging]]) |
| Make retrieval itself faster (fewer candidates always) | trades ranking quality on every search for a problem that only exists under load |

## Revisit when

- `xustive_degraded_total{stage="retrieval"}` is non-trivial in steady state — then the pool size
  or the Meilisearch node is the problem, not the ladder.
- A Meilisearch read replica ([[Decision Log]] §2) arrives; the narrowing may then be unnecessary.

## Where it stands (2026-08-27)

`crates/xustive-api/src/search.rs` (the `SearchError::Timeout` arm building the `narrow` query,
`retrieval_narrowed`, `facets_degraded`), `crates/xustive-api/src/metrics.rs` (`DEGRADED`),
`crates/xustive-api/src/lib.rs` (`SEARCH_GRACE_MS` on the `/search` `TimeoutLayer`). Commit
`f0112b8`; the bug is recorded in [[2026-08-25 - Code Audit Findings]] as BUG-041.

## Related

[[API Gateway]] · [[Query Pipeline]] · [[Error Handling and Resilience]] ·
[[Performance Budgets]] · [[ADR-0004 - Stream Summary Separately from Results]] · [[Decision Log]]
