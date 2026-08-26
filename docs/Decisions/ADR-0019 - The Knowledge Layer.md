---
tags: [adr]
adr-id: "0019"
status: accepted
date: 2026-08-26
---
# ADR-0019 - The Knowledge Layer

## Status

Accepted. Constrains [[Knowledge Layer]], [[Instant Answers]], [[Tool Data Plane]],
[[Query Pipeline]] and [[UI - Results Page]]. **Extends
[[ADR-0014 - Knowledge Panel from Wikipedia via the Web Tier]]** rather than superseding it: 0014's
web-tier fetch remains, demoted to the fallback path for entities the store does not hold.
Bounded by [[ADR-0001 - Two-Plane Architecture]] and [[ADR-0008 - No Query Logging]], neither of
which is amended here.

## Context

[[ADR-0014 - Knowledge Panel from Wikipedia via the Web Tier]] put a Wikipedia extract in the
results rail and explained why it could not live in the Rust API: the serving plane has no route to
the internet, and *"every entity a person might type"* is not the bounded, enumerable dataset the
[[Tool Data Plane]] pattern serves. Fetching per request from the Next tier was the honest way to
ship something.

That shipped, and its limits are now the product's limits. One authority yields a paragraph. A film
has a year, a director, a runtime and a score; a person has a face, a birth date and an occupation.
Those facts exist, in public, under licences we can honour — and the panel shows none of them.

The obvious extension is to fan out per search: ask a model which authorities apply, then call them.
Three things rule it out.

1. **Latency and reliability.** Five third-party calls on the request path is five ways for a page
   to be slow and five parties whose bad afternoon becomes ours.
2. **Rate limits.** Every free authority meters per IP. A shared server is one IP. The feature would
   work in development and fail at exactly the traffic that matters.
3. **Privacy.** [[ADR-0008 - No Query Logging]] forbids a cache keyed by a query — *"a query log
   with extra steps"* — and a per-search fan-out puts a live query in front of several companies
   with nothing but their good manners in between.

The unlock is that **the unit of caching does not have to be the query.** `Q42` is an entity id: it
is enumerable, it is identical for everyone who asks, and it says nothing about who asked. Caching
by entity is permitted where caching by query is not, and it is also simply better — one fetch
serves every reader forever, instead of one fetch per reader per search.

The second unlock is that **the authority set is a fact, not a judgement.** Wikidata records
*instance of* (`P31`) and the cross-reference identifiers — `P345` IMDb, `P1258` Rotten Tomatoes,
`P4947` TMDB, `P1712` Metacritic — plus review scores in `P444` attributed to a named reviewer in
`P447`, all under CC0. Which authorities describe a film is a table lookup on the type. A language
model asked the same question would be slower, less reliable, and would occupy a GPU slot on a 4 GB
card to reproduce a `match` statement.

## Decision

**Build a knowledge layer as a locally-stored entity index, harvested on the ingestion plane on a
fixed cadence, keyed by entity rather than by query. Resolve a query to an entity locally; describe
the entity from claims aggregated at harvest time; link to authorities by identifier rather than
fetching or scraping them. Use the language model only for disambiguation and blurb composition,
and cache both against the entity id.**

Specifically:

1. **Storage is a Meilisearch index.** Resolution is name matching with aliases, four scripts,
   transliteration and typos — a search problem, in an engine we already run and already tune for
   exactly these languages. Redis holds no entity text.
2. **Harvesting is `toold`-shaped**: ingestion plane, fixed cadence, no user input, `SafeUrl`,
   validation before write, `observed_at` distinct from `fetched_at`, licence carried per field.
3. **The serving plane only reads.** A cold entity has no panel, and that is correct rather than
   unfortunate — the [[Tool Data Plane]] doctrine, unchanged.
4. **Authorities are linked, never scraped.** Identifiers come from CC0 Wikidata; links are built
   from them. Facts we cannot obtain under a licence we can honour are facts the panel does not
   have.
5. **ADR-0014's web-tier path becomes the fallback** for entities absent from the store, keeping
   today's behaviour as the floor. Its image proxy and host allowlist are the model for every
   remote asset; the allowlist grows by named host, in an ADR, one at a time.
6. **Coverage is demand-driven through the existing k-anonymous mechanism.** A resolution miss is
   recorded exactly as a weak-coverage term is, under the same floor of `k ≥ 20`, in the same
   ephemeral signals instance. An entity fewer than `k` people asked for is never written down.
7. **The model assists, it does not route.** Disambiguation between close candidates and blurb
   composition where no extract exists, both bounded, both optional, both cached per entity, and
   the feature is fully useful with the model switched off — which is the default.

## Consequences

**Good**

- The panel is fast and stays fast: a local index lookup, no third party on the request path, and
  the same latency for the thousandth reader as for the first.
- The no-egress property is untouched. Nothing in the serving plane gains a route it did not have,
  and no new hop is added to the audited surface.
- No query-keyed cache exists anywhere, so [[ADR-0008 - No Query Logging]] needs no amendment.
- Rate limits stop being a scaling ceiling: harvest volume is a function of the corpus, not of
  traffic.
- Attribution is structural. A licence travels with the field it covers, so an unattributed
  reproduction requires deleting code rather than forgetting to add it.
- The store gets better with use, through a mechanism already built and already proven private.

**Bad**

- A cold entity has no panel until the harvester reaches it. Long-tail and breaking-news entities
  are worst served, and the demand queue is a lag, not a fix.
- We carry a store: harvest scheduling, staleness, refresh, and disk. That is real operational
  surface that the per-request design would not have had.
- Facts can go stale between harvests in a way a live fetch could not. Mitigated by refresh
  cadence proportional to demand, and by showing `as_of` where a fact is time-sensitive.
- Entity coverage is bounded by what Wikidata knows, which under-represents Algeria relative to
  Europe and North America. This is a real gap and the demand queue is how we find its edges.

## Alternatives

| Option | Why not |
|:---|:---|
| Per-search fan-out to authorities from the Next tier, model-routed | The design this ADR was written to evaluate. Fails on latency, on per-IP rate limits at real traffic, and on putting a live query in front of several third parties. Its caching would have to be query-keyed to help at all, which ADR-0008 forbids. |
| Route the fan-out through [[Federation Gateway]] instead | Keeps the query on our infrastructure, and the gateway exists for exactly this shape. But it inherits the latency and rate-limit problems unchanged, adds a third outbound client to a component deliberately bounded to two, and widens the audited egress surface for a result we can precompute. |
| Scrape IMDb and Rotten Tomatoes for the facts | Their terms forbid it. [[ADR-0013 - Direct SERP Collection for Discovery]] already drew this line: that behaviour is confined to the ingestion plane, for discovery, and does not extend to harvesting a third party's data product. |
| Extend `toold`'s Redis cache to entities rather than adding an index | Redis is the wrong tool for fuzzy multi-script name resolution, and would mean hand-building a matcher that Meilisearch already provides in exactly our four languages. |
| Ship Wikipedia-only forever | The status quo, and the reason this milestone exists. |

## Revisit when

- Wikidata coverage of Algerian entities proves too thin to carry the panel, and a licensed
  regional source appears that is worth a second harvest path.
- Sustained demand for entities that change hourly (live events, prices) makes the harvest lag the
  dominant complaint — at which point the gateway alternative above is worth re-costing, for those
  kinds only.
- A national or regional authority publishes a licensed dataset worth harvesting alongside
  Wikidata.
