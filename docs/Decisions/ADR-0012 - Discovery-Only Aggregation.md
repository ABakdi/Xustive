---
tags:
  - adr
adr-id: "0012"
status: accepted
date: 2026-08-13
---

# ADR-0012 - Discovery-Only Aggregation

## Status

**Accepted.**

Constrains [[Crawler Orchestrator]], [[Query Pipeline]], [[API Gateway]],
[[Data Sources Registry]], [[Admin and Source Submission]].

## Context

Crawling from a seed list alone reaches corpus scale slowly. The proposal was to query established
engines when a user searches, aggregate their results, and crawl those URLs in the background so
they become Xustive documents.

The goal is right — the fastest way to learn which URLs exist is to ask something that already
knows. The mechanism needs splitting in two, because the proposal contains one idea that works and
one that does not.

### Serving other engines' results is not available to us

- **Bing Search API — retired 11 August 2025.** No successor with the same shape; Microsoft's
  migration path is a platform commitment inside Azure AI Foundry.
- **Google Custom Search JSON API — closed to new customers, full retirement 1 January 2027.** We
  cannot sign up, and it would be withdrawn within months if we could.
- **Scraping Google's result pages** is against their terms and actively defended.
- **Brave Search API** — an independent index of ~30B pages, terms permit this use, ~$5/1000 queries.
  The one viable paid route.

Two objections apply regardless of which provider survives:

**It breaks the egress boundary.** The serving plane has no internet egress, asserted by
`scripts/test-egress.sh`. A third-party call on the query path violates one of the load-bearing
invariants of this architecture — the property that means a compromised query path cannot reach
out, and that user queries cannot leave the building.

**It contradicts [[ADR-0008 - No Query Logging]].** We decline to log queries ourselves; forwarding
every one of them to a third party who *will* log them gives up the same privacy through a side
door, and gives it up to someone whose retention we do not control.

There is also latency — 200–800 ms added to every search — and a product objection: an engine that
proxies another engine is a front-end, not an index.

### Discovery is the half that works

Learning *which URLs exist* and crawling them ourselves is separable from serving anyone's results.
It runs in the background, off the serving path, touches no user query in real time, and breaks
none of the above. It is also what the original goal actually asked for: those pages become **our**
documents.

## Decision

**Use external sources for URL discovery only. Never call a third-party engine on the serving
path.**

Four discovery inputs, in descending order of value to this project:

1. **Common Crawl** — 250B+ pages, free, monthly snapshots, with a columnar/CDX index that filters
   by host and domain. Filtering for `.dz` plus Arabic and French content on Algerian hosts
   bootstraps a frontier of millions of real URLs **without crawling anyone**. For an Algeria-first
   engine this is the shortest path to corpus scale, and it costs the sites nothing.

2. **Query-driven discovery** — needs no third party at all. A search returning zero or few results
   is a precise, free signal of where coverage is weak. Those become crawl priorities.

   This is the most valuable form of the original idea: it closes the gap between what users ask for
   and what we hold, using only our own traffic. Under [[ADR-0008 - No Query Logging]] it must work
   from aggregate counts over normalised query terms with a frequency floor — never a stored query
   log, never anything attributable to a person.

3. **Sitemaps and feeds** from sources already in the registry — free, and they enumerate URLs that
   link-following never reaches.

4. **Brave Search API** — for the residual only: weak queries that (1) and (2) did not resolve.
   Rate-limited, budgeted, and off by default.

Anything discovered this way enters the ordinary frontier and is subject to the ordinary rules —
robots, politeness, `SafeUrl`, dedup, trust tiering. **A URL from an external source gets no
privileges.**

## Consequences

**Good.** Corpus scale stops depending on link-following from 111 seeds. Common Crawl gives volume,
query-driven discovery gives relevance, and neither adds a millisecond to a search. The egress
boundary and the no-logging position both survive intact.

**Costs.** Common Crawl ingestion is a batch pipeline over Parquet/WARC — real work, and the index
is a month behind, so it bootstraps but never keeps anything fresh. That is [[ADR-0011 - Adaptive
Recrawl over Static Crawling]]'s job, and the two are complementary: one gets the URL, the other
keeps it true.

**Rejected: live metasearch.** Blocked on availability, forbidden by the egress boundary, in
tension with our own privacy position, and it makes the product a proxy.

**Rejected: scraping Google's SERPs.** Against their terms and adversarially defended. Distinct
from [[ADR-0009 - Direct Collection for Social Platforms]], where the owner accepted a specific
risk for a corpus unavailable any other way — here the corpus *is* available another way, so there
is nothing to trade for.

## Revisit when

- **Common Crawl's `.dz` coverage proves too thin.** M2-T16.8 answers this with measured yield. If
  the Algerian web is largely absent from it, the bootstrap premise fails and the weight shifts to
  query-driven discovery and registry curation.
- **A provider offers terms permitting result serving.** That alone is not enough — the no-egress
  boundary and [[ADR-0008 - No Query Logging]] would both still have to be resolved, and that is a
  new ADR, not an amendment to this one.
- **Brave's index or terms change.** It is the only paid route we can currently use; losing it
  removes T16.6 but leaves T16.1–T16.5 untouched, which is why it is last and off by default.
- **Query-driven discovery cannot be built within [[ADR-0008 - No Query Logging]].** If aggregate
  counts with a frequency floor turn out to be too coarse to be useful, drop the feature. Do not
  relax the privacy constraint to save it.

## Related

- [[ADR-0011 - Adaptive Recrawl over Static Crawling]] — the other half of the corpus problem
- [[ADR-0008 - No Query Logging]] — constrains how query-driven discovery may work
- [[Milestone 2 - Ingestion at Scale]] — M2-T16
