---
tags:
  - adr
adr-id: "0009"
status: accepted
supersedes: "0007"
date: 2026-08-06
---

# ADR-0009 - Direct Collection for Social Platforms

## Status

**Accepted.** Supersedes [[ADR-0007 - API-First Social Access]].

Constrains [[Social Connector - Facebook]], [[Social Connector - Instagram]],
[[Social Connector - TikTok]], [[Proxy Manager]], [[Session Manager]], [[Fingerprint Engine]],
[[Signature Service]], [[Web Fetcher]], [[Politeness and Robots]].

## Context

[[ADR-0007 - API-First Social Access]] committed the project to platform APIs only, with no direct
collection path in the code. Under that constraint the reachable corpus is:

| Platform | API-reachable | Practical result |
|:---|:---|:---|
| Facebook Pages | with App Review + business verification | months of process, partial coverage |
| Facebook Groups | **only if each group admin installs our app** | effectively zero at scale |
| Instagram | authorised Business/Creator accounts; 30 hashtags / 7 days | negligible |
| TikTok | Research API, institutional approval | uncertain, slow |

Facebook groups are where Algerian classifieds, job posts, and civic discussion actually happen. An
index without them is missing the corpus that most differentiates the product. The API-only path
makes the product's core value contingent on thousands of individual admin installations.

**I have directed that direct collection be used. I am covered legally, and I accept the
contractual and legal risk.** That is the decision this ADR records. The engineering question that
remains is not *whether* but *how to do it well* — because a collection system that is detected and
banned within a week is a failed system regardless of its legal footing.

## Decision

**Direct collection is a first-class ingestion path, not a fallback.**

1. **Hybrid by cost, not by principle.** Where a platform API is available, authorised, and cheaper
   per document, use it — APIs are more stable and cost nothing to defend. Where it is not, collect
   directly. The connector chooses per source, not per platform.

2. **Three new components** carry the collection-specific concerns, so the connectors stay readable:
   - [[Session Manager]] (C25) — account pool, cookies, login, challenge handling, per-account budgets
   - [[Fingerprint Engine]] (C26) — coherent TLS / HTTP2 / header / browser fingerprint profiles
   - [[Signature Service]] (C27) — executes platform request-signing JS (X-Bogus, msToken, LSD, …)

3. **[[Proxy Manager]] is rebuilt** for residential and mobile pools with per-platform policy, sticky
   session pinning, ASN diversity, and bandwidth cost accounting.

4. **`on_blocked` changes** from `halt_and_flag` to a graded response ladder
   ([[Proxy Manager]] §4.6). Detection is now a signal to slow down, rotate identity, and re-warm —
   not to stop.

5. **The pinning invariant** governs everything: `account ↔ proxy ↔ fingerprint ↔ device` is a
   **stable tuple** for the life of an identity. Rotating any element independently is the single
   largest cause of bans, and it is what naive scrapers get wrong
   ([[Session Manager]] §4.2).

6. **Collection scope** is content visible to (a) an anonymous visitor, or (b) an account we control.
   Joining closed groups with our accounts is a per-source config decision recorded in
   [[Data Sources Registry]], not a default.

## What Does *Not* Change

These were not consequences of ADR-0007 and survive it intact:

| Commitment | Where |
|:---|:---|
| Open-web crawling stays fully polite: robots.txt, crawl-delay, honest UA, 1 req/host | [[Politeness and Robots]] §4 — platform collection is a *separate profile*, not a licence to hammer everything |
| No query logging; queries never leave the country | [[ADR-0008 - No Query Logging]] |
| Upstream deletions propagate to our index within 24 h | connectors §4 |
| Takedown path: content + comments + vectors + permanent blocklist | [[Indexer Worker]] §4.5 |
| No face recognition, no person-centric profiling, no author-history view | [[Image Pipeline]] §10 |
| EXIF/GPS stripped from all media | [[Image Pipeline]] §4.1 |
| Personal-data obligations under Law 18-07 | [[Legal and Compliance]] §5 — unaffected by *how* data is collected |

The last row is worth stating plainly: the collection method changed; the duties owed to the people
in the data did not.

## Consequences

**Good**
- Facebook groups, Instagram profiles, and TikTok become reachable at meaningful scale.
- Coverage no longer depends on outreach, approvals, or admin cooperation.
- Collection breadth is an engineering variable we control rather than a partnership outcome.
- Freshness improves: no API quota ceilings, no 30-hashtag windows.

**Bad**
- **Permanent maintenance burden.** Platforms change defences continuously. Signer rotation, DOM
  changes, new challenge types, and fingerprint checks will break collection with no warning and no
  deprecation notice. Budget for this as ongoing work, not a project.
- **Account and proxy cost becomes a real line item.** Residential bandwidth is billed per GB;
  accounts have acquisition and replacement cost. [[Proxy Manager]] §8 tracks it as a first-class
  metric because it determines whether a source is worth collecting.
- **Silent failure is the dominant failure mode.** Platforms increasingly serve *empty or degraded
  results* rather than errors. A connector that reports success while returning nothing is worse
  than one that crashes ([[Session Manager]] §4.6).
- **CI cannot test against live platforms.** Everything is fixture-driven, so real breakage is
  detected in production by canaries, not in CI.
- **Legal and contractual exposure** — accepted by me per §Context.
- Higher operational complexity: three new components and a stateful account pool, which is the
  first genuinely stateful thing in the ingestion plane.

**Commits us to**
- Treating detection metrics (`ban_rate`, `challenge_rate`, `empty_response_rate`) as tier-1
  operational signals with paging alerts ([[Observability]] §6).
- Canary accounts that continuously validate each collection path against known-good fixtures, so
  silent breakage surfaces in minutes.
- Conservative per-identity budgets. The instinct to maximise throughput is what burns account pools;
  the constraint is identity longevity, not requests per second.

## Alternatives

| Option | Why not |
|:---|:---|
| API-only ([[ADR-0007 - API-First Social Access]]) | coverage gap that undermines the product's core value; superseded by owner decision |
| API-first with scraping fallback | in practice the fallback becomes the primary path; better to design it deliberately than to bolt it on |
| Buy data from a third-party aggregator | cost scales badly, provenance and freshness are opaque, and it exports the same risk to a vendor |
| Third-party scraping-as-a-service | sends our target list off-shore, breaks the sovereignty story, and offers no control over detection handling |

## Revisit when

- A platform introduces a compliant bulk-access path that matches direct-collection coverage.
- Detection cost per document exceeds the value of the content — measured, per source, in
  [[Data Sources Registry]] §7.
- Counsel's position changes, or my risk assessment changes.

## Related

[[ADR-0007 - API-First Social Access]] (superseded) · [[Session Manager]] · [[Fingerprint Engine]] ·
[[Signature Service]] · [[Proxy Manager]] · [[Legal and Compliance]] · [[Decision Log]]
