---
tags:
  - adr
adr-id: "0013"
status: accepted
supersedes: "0012"
date: 2026-08-13
---

# ADR-0013 - Direct SERP Collection for Discovery

## Status

**Accepted.** Supersedes [[ADR-0012 - Discovery-Only Aggregation]].

Constrains [[Crawler Orchestrator]], [[Query Pipeline]], [[Proxy Manager]], [[Session Manager]],
[[Fingerprint Engine]], [[Data Sources Registry]].

## Context

[[ADR-0012 - Discovery-Only Aggregation]] ruled out querying Google on three grounds: the official
APIs are closing, the terms forbid scraping, and a call on the serving path would breach the
no-egress boundary. It kept Common Crawl, query-driven discovery, sitemaps, and Brave.

**I have directed that Google be queried directly. The legal position in my jurisdiction is
settled, and this is a personal project rather than a production service.** That is the decision
this ADR records, and it is the same shape as
[[ADR-0009 - Direct Collection for Social Platforms]]: I accept a contractual risk, and the
engineering question becomes *how to do it without it collapsing*.

Two of ADR-0012's three objections were never about terms of service, and they survive unchanged:

- **The no-egress boundary.** The serving plane cannot reach the internet, asserted by
  `scripts/test-egress.sh`. This is our own invariant, not Google's, and abandoning it would mean a
  compromised query path could phone out.
- **[[ADR-0008 - No Query Logging]].** We decline to retain queries. Forwarding each one verbatim to
  a third party gives the same thing away through a side door.

Both are satisfiable without giving anything up, because collection is a **background** activity.
The serving plane never calls out; it enqueues. That is the design below.

### What direct SERP collection is actually worth

Worth stating plainly, because it determines how much to invest: **a SERP query yields about ten
URLs.** Common Crawl yields billions, free, with no adversary. As a bulk corpus channel this is
three orders of magnitude in the wrong direction.

Its value is *targeting*, not volume: it answers "what exists for this specific query where our
coverage is thin", which no bulk source can. So it is scoped as the narrow, last-resort channel and
budgeted accordingly.

## Decision

**Collect SERPs directly, in the ingestion plane only, as the narrowest discovery channel.**

1. **Cross-plane by queue, never by call.** A weak-coverage query is published to a Redis stream by
   the serving plane and consumed by the ingestion plane, which owns all egress. The serving plane
   makes no outbound request and blocks on nothing. The no-egress test stays green and stays
   meaningful.

2. **Reuse the collection layer already specified.** [[Proxy Manager]], [[Session Manager]] and
   [[Fingerprint Engine]] exist for M2-T01a/b/c. A SERP source is another consumer of that layer,
   subject to the same pinning invariant — `identity ↔ proxy ↔ fingerprint ↔ device`. Building a
   second, parallel evasion path would be the mistake here.

3. **Residential egress is required, not optional.** Datacenter ranges are classified almost
   immediately. This is the single largest determinant of whether the channel works at all, and it
   is a cost, so the channel is off by default.

4. **Prefer the cheapest endpoint that answers.** A ladder from lightweight HTML endpoints up to a
   rendered browser, demoting on failure — the same shape as [[Signature Service]] §4.6. Most
   discovery needs a list of URLs, which the plainest endpoint gives.

5. **Low, jittered, diurnally shaped volume.** Human-shaped pacing from [[Session Manager]] §4.5.
   The failure mode is not one detected request; it is a pattern.

6. **A challenge means stop.** CAPTCHA, interstitial, or consent wall → the identity is quarantined
   and the channel backs off. **We detect challenges; we do not solve them.** Beyond being a line
   worth keeping, it is the correct engineering response: a challenge means the identity is already
   classified, and pushing through burns it faster than resting it. This is exactly the
   quarantine-on-challenge behaviour M2-T01a.7 and M2-T01a.9 already specify.

7. **Silent degradation is the real risk.** Search engines return plausible, degraded results to
   suspected bots rather than blocking outright. Canary queries with known-stable results, checked
   against expected URLs — borrowed wholesale from the silent-cloaking detection in M2-T01a.8. A
   channel that has been quietly neutered must not report itself healthy.

8. **Discovered URLs get no privileges.** Ordinary frontier, ordinary robots, politeness, `SafeUrl`,
   dedup and trust tiering. We ignore Google's terms; we do not ignore the terms of the sites Google
   points at.

9. **Query handling still obeys [[ADR-0008 - No Query Logging]].** Only normalised terms above a
   frequency floor cross the plane boundary — never a per-user query, never anything attributable.
   My direction covers Google's terms, not our own privacy position, so that constraint is
   unchanged.

## Consequences

**Good.** Discovery gets a targeted channel that answers questions bulk sources cannot. It costs
little to add, because the collection layer is already being built for social.

**Costs.** A permanent maintenance tail — the same one [[ADR-0009 - Direct Collection for Social
Platforms]] names. SERP markup and defences change without notice, and this will break repeatedly.
Budget it as ongoing work. Residential bandwidth is a real bill. And the channel is now a second
consumer competing for the identity pool, which M2-T01a.12 already says must halt rather than
degrade.

**Scope discipline.** Because the yield is ~10 URLs per query, this must stay last in the ladder.
If it ever becomes the main discovery path, something upstream has failed — most likely the Common
Crawl bootstrap — and that is the thing to fix.

**Unchanged from ADR-0012.** No live metasearch: we do not serve another engine's results, and no
third-party call sits on the serving path. That was never the terms-of-service objection.

## Revisit when

- **M2-T16.8 shows the channel yielding less than its cost.** Ten URLs per query against residential
  bandwidth and a maintenance tail is a narrow margin; measure it rather than assuming it.
- **Challenge rate exceeds the M2-T01a threshold sustainably.** That means the approach is no longer
  working, and the answer is to stop, not to escalate.
- **Common Crawl coverage of `.dz` proves strong.** Then this channel's remaining value is freshness
  on weak queries only, and it may not be worth the tail.
- **The project stops being personal.** My stated basis is personal, non-production use. If that
  changes, this decision does not automatically carry over.

## Related

- [[ADR-0012 - Discovery-Only Aggregation]] — superseded; its non-ToS reasoning is retained above
- [[ADR-0009 - Direct Collection for Social Platforms]] — same shape, same maintenance tail
- [[ADR-0008 - No Query Logging]] — still binding on what crosses the plane boundary
- [[ADR-0011 - Adaptive Recrawl over Static Crawling]] · [[Milestone 2 - Ingestion at Scale]] M2-T16
