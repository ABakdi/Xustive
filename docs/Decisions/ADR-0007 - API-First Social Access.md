---
tags:
  - adr
adr-id: "0007"
status: superseded
superseded-by: "0009"
date: 2026-08-06
---

# ADR-0007 - API-First Social Access

> [!warning] Superseded by [[ADR-0009 - Direct Collection for Social Platforms]]
> This ADR is retained for the record. Its reasoning about *coverage cost* proved decisive — but in
> the opposite direction: the coverage gap it accepted was judged too large, and the project owner
> directed that direct collection be used, accepting the risk. Read ADR-0009 for the design in force.
>
> The commitments in §Consequences that were **not** consequences of this decision — open-web
> politeness, deletion propagation, takedowns, no profiling — survive unchanged
> ([[ADR-0009 - Direct Collection for Social Platforms]] §"What Does Not Change").

## Status

**Superseded** by [[ADR-0009 - Direct Collection for Social Platforms]] (2026-08-06).

Originally constrained [[Social Connector - Facebook]], [[Social Connector - Instagram]],
[[Social Connector - TikTok]], [[Proxy Manager]], [[Legal and Compliance]].

## Context

Social content is the most valuable part of the corpus for Algerian users — classifieds, job posts,
announcements, and civic discussion happen in Facebook groups, not on websites.

It is also the part we do not control access to. Meta's and TikTok's Platform Terms prohibit
automated collection outside their APIs, and those APIs impose real limits:

- Facebook Groups require the **group admin** to install our app. There is no unilateral path.
- Instagram public-profile data is not generally available; hashtag search is capped at 30 unique
  hashtags per 7 days.
- TikTok's Research API requires an approved application, typically with institutional backing.

The original technical spec listed stealth-scraping and fingerprint-spoofing libraries as part of the
stack. That approach would produce more coverage in the short term, and it would make the project's
legal position, its partner relationships, and its ability to operate publicly all contingent on not
being noticed.

## Decision

**Every social connector is built exclusively against the platform's compliant API path.**

Concretely:
1. No scraping fallback exists **in the code**. Not disabled by a flag — absent.
2. When authorisation lapses (token expired, app uninstalled, approval revoked), the connector
   **disables itself** and alerts; it does not degrade to another method
   ([[Social Connector - Facebook]] §7).
3. [[Proxy Manager]] operates under `on_blocked = "halt_and_flag"`: when a host or platform refuses
   us, we stop and raise it for a human. Rotating egress and retrying is not a code path, and a test
   enforces this ([[Proxy Manager]] §11).
4. Proxies exist for rate distribution and reliability on the **open web**, not to create access that
   terms withhold.
5. Any deviation requires a superseding ADR with counsel sign-off — not a config change and not a
   pull request.

## Consequences

**Good**
- The project can operate openly: publish a bot page, contact site owners, approach group admins, and
  apply for API access without contradiction.
- No risk of a sudden, total loss of a data source through account termination.
- The privacy and sovereignty story stays coherent — we cannot credibly promise users we respect
  rules while routing around platforms'.
- Partnerships become possible. A group admin can be *asked*, and asking works better when the
  alternative is not "we'll take it anyway".

**Bad**
- **Coverage will be significantly lower than the original spec assumed**, especially for Facebook
  groups. This is the real cost and it is large.
- Coverage now depends on outreach and approvals — work that engineering cannot do alone and cannot
  schedule reliably.
- Some sources may be permanently unreachable.
- Competitors willing to scrape will have more data.

**Commits us to**
- Treating social coverage as a **partnership programme**, not a crawling problem
  ([[Legal and Compliance]] §9).
- Leading the product with web + whatever social access is actually granted, and being honest with
  users about what is and is not indexed.

## Alternatives

| Option | Why not |
|:---|:---|
| Scrape the logged-out web interface | breaches platform terms; risks termination and legal exposure; incompatible with operating openly |
| Scrape with fingerprint spoofing and proxy rotation | the above, plus an arms race that consumes engineering time indefinitely |
| Accept user-uploaded data exports | consented and compliant, but tiny coverage; kept as a possible supplement |
| Skip social entirely | simpler and safest, but abandons the corpus that most differentiates the product |

## Revisit when

- A platform introduces a compliant bulk-access path for public content.
- Counsel advises that a specific alternative access method is lawful **and** contractually
  permissible in Algeria — in writing.
- The partnership approach demonstrably fails to deliver usable coverage, forcing a strategic
  decision about what Xustive is.

## Related

[[Legal and Compliance]] · [[Social Connector - Facebook]] · [[Social Connector - Instagram]] ·
[[Social Connector - TikTok]] · [[Proxy Manager]] · [[Data Sources Registry]] · [[Decision Log]]
