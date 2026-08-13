---
tags:
  - adr
adr-id: "0011"
status: accepted
date: 2026-08-13
---

# ADR-0011 - Adaptive Recrawl over Static Crawling

## Status

**Accepted.**

Constrains [[Crawler Orchestrator]], [[Web Fetcher]], [[Deduplication Service]],
[[Content Parser]], [[Search Index]].

## Context

The crawler treats a URL as crawled once and then forgets it. An index built that way is a
photograph, not a search engine: a ministry publishes a decree, a newspaper corrects a story, a
price page changes, and Xustive keeps serving what it saw the first time. Freshness is not a
polish item — for news and government sources it *is* the product.

The naive fix is to recrawl everything on a fixed timer, or to recrawl each page in proportion to
how often it changes. Both are wrong, and the second is wrong in a way that is worth recording
because it is the intuitive choice.

### Proportional recrawl is worse than uniform recrawl

Cho & Garcia-Molina (*Effective Page Refresh Policies for Web Crawlers*, ACM TODS 28, 2003) showed
that refreshing pages in proportion to their change rate produces **worse** average freshness than
refreshing every page at the same rate.

A page that changes many times between visits can never be kept fresh. Every fetch spent on it is
stale again almost immediately, and that budget is taken from pages that *could* have been kept
current. The optimal policy is **non-monotonic**: investment rises with change rate up to a point
and then falls away, and the fastest-changing pages are best left alone.

### Change rate is the wrong signal; longevity is the right one

Olston & Pandey (*Recrawl Scheduling Based on Information Longevity*, WWW 2008) showed that the
question to ask is not "did the page change?" but "did the change **persist**?"

Ephemeral content — view counters, "most read" sidebars, ad slots, rendered timestamps — changes
constantly and is worthless: by the time it is indexed it no longer represents the page. Persistent
content — a new article, a new decree — may change rarely but stays true for months. Scheduling on
longevity rather than churn gave them **better freshness at lower cost**.

This matters more for Xustive than for a general crawler. Algerian news sites wrap their articles
in exactly this kind of churn; an APS or Echorouk article page differs byte-for-byte on nearly
every fetch while the article body never moves. A crawler that detects *any* difference would
recrawl the whole corpus daily and learn nothing from it.

### Estimating change rate is biased

We never observe how many times a page changed — only whether it differs from the copy we hold.
The obvious estimator (changes seen ÷ visits) therefore undercounts, and undercounts worst exactly
where the error costs most, on fast-changing pages.

## Decision

**Schedule recrawls adaptively, on content longevity rather than byte change, and abandon pages
that cannot be kept fresh.**

1. **Two hashes per fetch.** One over the raw body, one over the extracted article text after
   boilerplate stripping. **Only a change in the second counts as a change.** This is the cheap
   form of longevity scoring and it reuses extraction we already do.

2. **Interval adapts multiplicatively.** Content changed → halve the interval. Unchanged → grow it
   by half again. Floor and ceiling are set per trust tier, so an A-tier ministry site is held to a
   tighter ceiling than a page discovered three links deep.

   Chosen over the formal estimators deliberately: it is robust to the estimation bias above, needs
   no per-page model, and degrades sensibly when the history is short.

3. **Volatile pages are abandoned, not chased.** A page that changes on every visit even at its
   floor is moved to a slow lane. This is the Cho result applied directly, and it shares its
   mechanism with the revision-loop guard in M2-T05.8.

4. **Conditional requests are the enabling primitive.** `If-None-Match` / `If-Modified-Since`, with
   a 304 costing a few hundred bytes rather than a page. Without this, adaptive scheduling is
   unaffordable and the rest of this ADR does not pay for itself.

5. **Sitemap `lastmod` and feeds are preferred over polling.** One fetch reports on hundreds of
   URLs. For news sources this is the highest-yield freshness signal available.

6. **Frontier priority becomes `trust × change_probability × age`.** Today it is `depth × 1000`,
   with depth pinned to a constant, which makes every URL equal priority and leaves `max_depth`
   dead code.

## Consequences

**Good.** Freshness stops being accidental. Conditional requests make revisits nearly free, so
coverage and freshness stop competing for the same budget. Sites see fewer wasted full fetches,
which is politeness as well as economy.

**Costs.** Every document needs change history — `last_fetched`, `last_modified`, `etag`, both
hashes, and a short ring of observations. That is a schema change and a migration. Bad boilerplate
stripping now has a second failure mode: it makes churn look like content, and the crawler chases
it. The stripping quality tests (M2-T15.9) exist for that reason.

**Rejected: fixed-interval recrawl.** Simple, and roughly half the fetches are wasted on pages that
did not change while genuinely fresh pages go stale between ticks.

**Rejected: proportional recrawl.** The intuitive choice, and measurably worse than doing nothing
clever at all. Recorded here so it is not reintroduced as an optimisation.

## Revisit when

- **M2-T15.10 stops showing a win.** If fetches-per-real-change converges on the fixed-interval
  baseline, the adaptive machinery is complexity we are not being paid for — go back to a timer.
- **Boilerplate stripping proves unreliable at scale.** The dual hash assumes extraction is stable
  across fetches. If M2-T15.9 keeps failing on real sites, the cheap proxy for longevity has failed
  and the choice is a real longevity model or abandoning content-based change detection.
- **Per-URL change history outgrows Redis.** The ring is bounded, but at 10M+ documents this is a
  storage decision, and it lands next to the raw-blob question in [[Task Queue]] §12.
- **A source publishes reliable change feeds.** If sitemap `lastmod` and feeds cover most of the
  corpus honestly, polling intervals matter much less and this ADR shrinks to T15.6.

## Related

- [[ADR-0012 - Discovery-Only Aggregation]] — the other half of the corpus problem
- [[Milestone 2 - Ingestion at Scale]] — M2-T15
- [[Crawler Orchestrator]] · [[Web Fetcher]] · [[Deduplication Service]]
