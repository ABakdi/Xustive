---
tags: [adr]
adr-id: "0023"
status: accepted
date: 2026-08-26
---
# ADR-0023 - Live Wikidata Fallback Judged by the Local Resolver

## Status

Accepted, implemented. **Amends [[ADR-0014 - Knowledge Panel from Wikipedia via the Web Tier]]**
(the web-tier fallback now resolves through Wikidata, not Wikipedia's first search hit, and
Rust gains two endpoints 0014 said it would never need) and **extends
[[ADR-0019 - The Knowledge Layer]] §5** (the fallback path) with the mechanism that keeps it
honest. Does not amend [[ADR-0001 - Two-Plane Architecture]]: the only outbound calls are from
the web tier, the one place ADR-0014 sanctioned egress, and the Rust API still fetches nothing.

## Context

ADR-0019 demoted the web-tier fetch to "the fallback for entities the store does not hold". As
built in ADR-0014 that fallback took Wikipedia's first search hit blindly: for a bare surname
that is the name article (*Ronaldo — Name list*), and for a TV show (*for all mankind*) it found
nothing. The store, meanwhile, had a resolver with exact-name-first scoring, a corpus-agreement
signal and a precision floor ([[ADR-0022 - Entity Resolution Prefers Silence to a Wrong Panel]]),
and a renderer with per-kind templates. Writing a second, weaker version of both in TypeScript
for the live path was the obvious mistake.

Two further facts shaped it. A country's Wikidata document is megabytes, so fetching full
documents for a shortlist timed out at six seconds and read as "random no panel". And after a
day of testing that made hundreds of requests, Wikimedia's edge began refusing this host's
surplus connections (`UND_ERR_CONNECT_TIMEOUT`) — the per-request throttling ADR-0019 predicted
for any fan-out, arriving on the live path.

## Decision

**The web tier gathers candidates from Wikidata; the Rust store's own resolver and renderer
judge and draw them. The live path records its miss into the harvester's demand queue so it is
taken less and less.**

1. **Store first, always.** The panel asks `GET /api/v1/knowledge` (the local index). Only a miss
   takes the live route.
2. **Two-phase fetch.** Wikidata `wbsearchentities` in the UI language and English, up to twelve
   candidates (Zidane was eighth for *zidane*; a shortlist of seven never showed him); then one
   `wbgetentities` for labels, aliases, descriptions and sitelinks only; disambiguation,
   given-name and family-name pages removed; one `P31` claim lookup per candidate. Only the one
   winner's full document is fetched.
3. **The Rust resolver judges.** `POST /api/v1/knowledge/resolve-live` takes the raw candidates
   plus the relation's kind hint and applies exactly the store's rules. `POST /api/v1/knowledge/render`
   turns the winner's raw document into the same panel a harvested entity gets — one parser, one
   set of templates. Both endpoints are pure functions of their body: they read no index and
   fetch nothing, so ADR-0001's no-egress property is untouched.
4. **The miss is recorded k-anonymously** (`entity:weak:{term}`, the same store and floor as
   weak-coverage) and the harvester's demand queue promotes the name into its next pass, so a
   subject held in the store never takes the live path again. A name fewer than `k` people asked
   for is never written down.
5. **One outbound pool, IPv4, one retry.** Every knowledge route talks to Wikimedia and Open
   Library through a single keep-alive undici `Agent` (four connections, 5 s connect timeout,
   `family: 4` because the resolver returns an IPv6 address this host cannot route and the
   attempt is a connect timeout rather than a fast failure), memoised on `globalThis` because
   Next bundles each route separately. A refused or reset connection is retried once; the second
   attempt reuses the pool's live connection. Logs carry host and status, never the query.
6. **The panel's own loading skeleton is client-only**, so the no-JS path gets nothing visible
   rather than a frame that never fills; the summary's skeleton stays server-rendered because
   the reader is explicitly waiting on that one.

## Consequences

**Good**
- *ronaldo*, *for all mankind*, *zidane*, *messi* resolve on the live path with the same
  precision rules as the store, and every improvement to the resolver improves both paths.
- A subject that is asked for enough is harvested and stops costing a live round-trip —
  ADR-0019's convergence, made concrete.
- No second parser in TypeScript to drift.

**Bad**
- The live path is still a per-request fan-out to a third party, with everything ADR-0019 said
  about that: slow afternoons upstream become ours, and Wikimedia's edge throttles a busy host.
  The pool and retry contain it; the demand queue is the fix.
- The Rust API gained two endpoints ADR-0014 said it would never need. They are body-only and
  the render route sits outside the global 8 KB body limit (like OCR) because a country's
  document is megabytes — a bigger request surface than the panel had before.
- A bare Arabic surname (*أفلام سبيلبرغ*) misses live because Wikidata's search is a label
  prefix and the Arabic label is the full name; entities in the store resolve by alias in any
  script — one more reason to harvest.
- The web tier now depends on this host having a working IPv4 route to Wikimedia; the IPv6 pin
  is a workaround for this machine, not a property of the design.

## Alternatives

| Option | Why not |
|:---|:---|
| Keep ADR-0014's Wikipedia first-hit | the name-list problem; no kind, no facts |
| Rank candidates in the web tier by sitelink count | the first cut; produced Jesus for *messi* and a game for *zidane* |
| Fetch full documents for the shortlist | six-second timeouts read as "no panel" |
| SPARQL for kinds | 6.8 s measured; per-id `wbgetclaims` at concurrency 3 is faster |
| Route the fan-out through the [[Federation Gateway]] | rejected in ADR-0019 for the same latency and rate-limit reasons; the web tier already had sanctioned egress |

## Revisit when

- The harvester's store covers what people ask, and the live path's share of panels falls to
  noise — then it can be narrowed to `id=` lookups only.
- Wikimedia throttling recurs despite the pool — the answer is a harvester pass, not a bigger
  pool.
- The host gains a routable IPv6 — drop `family: 4`.

## Where it stands (2026-08-27)

`web/app/api/knowledge-live/route.ts` (two-phase candidates, `NOT_A_THING`, `instanceOfMany`,
`resolve()` → `/resolve-live`, two `render` rounds, `Cache-Control: private, max-age=300`),
`web/lib/upstream.ts` (`upstreamAgent`, `viaUpstream`), `crates/xustive-api/src/knowledge.rs`
(`resolve_live`, `render_document`, `record_miss` under `DEMAND_NAMESPACE = "entity"`),
`crates/xustive-toold/src/main.rs` (`demand_seeds`, `MAX_PER_PASS = 10`, its own
`--k-anonymity` default 20). `web/components/search/EntityPanel.tsx` also loads a picked entity
by `?id=` when a relation row emits `xustive:subject`. Commits `29de58f`, `24c2d6a`, `f3c6dca`,
`a231f0b`. ADR-0014's `/api/knowledge` and `/api/wiki-image` routes still exist but have no
callers; they use bare `fetch`, not the shared pool.

## Related

[[ADR-0014 - Knowledge Panel from Wikipedia via the Web Tier]] · [[ADR-0019 - The Knowledge Layer]] ·
[[ADR-0022 - Entity Resolution Prefers Silence to a Wrong Panel]] · [[Instant Answers]] ·
[[Milestone 8 - The Answer Layer]] · [[Decision Log]]
