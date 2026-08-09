---
tags:
  - engineering
  - data
type: reference
status: draft
updated: 2026-08-06
---

# Data Sources Registry

> What Xustive crawls, under which policy, and on whose authority. The registry is *data*, versioned
> in git and exported from the live store on every change.
> Schema: [[Data Model]] §5 · Managed by [[Admin and Source Submission]] · Consumed by [[Crawler Orchestrator]]

---

## 1. Why the Registry Is the Product

Xustive is not a general web crawler. Its value comes from crawling **the right few thousand sources
well**, not the whole web badly. The registry is therefore a curated editorial artefact as much as a
config file — and its quality determines result quality more than any ranking tweak.

A corollary worth internalising: adding a source is a decision with legal, quality, and cost
consequences. Nothing is auto-approved ([[Admin and Source Submission]] §4.2).

---

## 2. Record Shape

Full schema in [[Data Model]] §5. The fields that carry judgement:

| Field | Meaning |
|:---|:---|
| `trust_tier` | A / B / C → a ranking boost ([[Ranking and Relevance]] §3) |
| `crawl_policy.frequency` | realtime / hourly / daily / weekly |
| `crawl_policy.crawl_delay_ms` | politeness floor, over and above `robots.txt` |
| `approved` | **false until a human approves** |
| `legal_basis` | why we are permitted to crawl this — see §5 |
| `added_by` | operator or submission id |
| `notes` | anything a future operator needs to know |

---

## 3. Trust Tiers

| Tier | Meaning | Examples | Boost |
|:---|:---|:---|:---|
| **A** | established, edited, accountable | major national news outlets, government portals, universities | 1.0 |
| **B** | credible but unedited or narrow | regional news, institutional pages, verified organisation accounts | 0.6 |
| **C** | user-generated or unverified | groups, personal pages, submitted sites | 0.3 |

Tier is about **accountability**, not about agreement or quality of opinion. A tier-A source can be
wrong; the tier says someone is answerable for it. Community submissions start at C and are promoted
only on evidence.

---

## 4. Source Categories (target coverage)

Concrete counts belong in the live registry; this is the shape of the ambition.

| Category | Kind | Frequency | Tier | Notes |
|:---|:---|:---|:---|:---|
| National news (ar/fr) | web | hourly | A | RSS/sitemap available; the freshness backbone |
| Regional/local news | web | daily | B | thin coverage today — a real gap worth filling |
| Government & administration | web | daily | A | procedures, forms, announcements — high-intent queries |
| Universities & research | web | weekly | A | |
| Public services (utilities, telecom, transport) | web | daily | A | Sonelgaz, Seaal, Air Algérie, operators |
| Classifieds & marketplaces | web | hourly | B | high query volume, high spam risk |
| Blogs & forums | web | weekly | C | Darija-rich; valuable for [[Query Expander]] lexicons |
| Facebook Pages | facebook | hourly | B | requires API authorisation |
| Facebook Groups | facebook | hourly | C | **requires group-admin app installation** |
| Instagram business/creator | instagram | daily | B | requires account authorisation |
| TikTok creators & hashtags | tiktok | daily | C | requires Research API approval |

Every social row depends on an access path that is not ours to grant — see
[[Legal and Compliance]] §4.

---

## 5. `legal_basis` — a required field

No source is crawled without one:

| Value | Meaning |
|:---|:---|
| `public_web_robots_ok` | open web, `robots.txt` permits, we are identified |
| `platform_api_authorized` | an approved app + a token scoped to this object |
| `owner_consent` | the owner asked us to index them (submissions, partnerships) |
| `hashtag_api_quota` | within the platform's documented public-search allowance |

A source whose basis lapses (token revoked, app un-installed, robots changed) is **auto-disabled** by
the connector, not merely flagged ([[Social Connector - Facebook]] §7).

---

## 6. Lifecycle

```
proposed → reviewed → approved → active → (degraded) → disabled → archived
```

| Transition | Trigger |
|:---|:---|
| proposed → reviewed | a human opens the review packet ([[Admin and Source Submission]] §4.2) |
| reviewed → approved | passes quality, legal basis recorded, tier assigned |
| approved → active | entry points injected into the frontier |
| active → degraded | error rate > 40 % for 24 h, or extraction quality drops (`parser_rule_miss_total`) |
| any → disabled | legal basis lapses, host opts out, takedown at domain scope, or persistent failure |
| disabled → archived | after 90 days; its documents are removed from the index |

**Quarterly review** of the whole registry: sources that produce nothing, produce spam, or have gone
dark are demoted or removed. Registries rot silently otherwise — a dead source costs crawl budget
forever and nobody notices.

---

## 7. Quality Signals per Source

Tracked per source and visible in the admin surface:

| Signal | Healthy | Action if bad |
|:---|:---|:---|
| Fetch success rate | > 95 % | investigate; check robots and breaker state |
| Extraction success | > 90 % | write or fix a per-domain rule ([[Content Parser]] §4.1) |
| Duplicate ratio | < 30 % | mostly republished content → consider demoting |
| Spam score (mean) | < 0.2 | > 0.5 sustained → demote or disable |
| Docs indexed per run | non-zero | zero for 3 runs → degraded |
| Date precision `unknown` ratio | < 10 % | > 40 % → the parser rule needs a date selector |

`parser_rule_miss_total` rising for a domain is the classic silent failure: the site redesigned,
extraction fell back to the generic path, and quality dropped without a single error
([[Content Parser]] §7).

---

## 8. Seeding Plan

| Milestone | Target |
|:---|:---|
| [[Milestone 0 - Foundations]] | 10 sources — fixture site + a handful of news sites with clean sitemaps |
| [[Milestone 1 - Text Search MVP]] | ~50 web sources across news, government, and services |
| [[Milestone 2 - Ingestion at Scale]] | ~500 web sources + whatever social access has actually been granted |
| [[Milestone 5 - Beta Launch]] | ~2 000, including community submissions |

The social numbers are deliberately not specified: they depend entirely on outreach outcomes
([[Legal and Compliance]] §9), and writing an aspirational figure here would disguise a dependency as
a plan.

---

## 9. Storage and Review

- Live: Meilisearch `sources` index + Redis for hot policy lookups.
- Mirror: `data/sources/registry.json`, exported on every change and committed — so registry history
  is reviewable in git and restorable in minutes ([[Deployment Topology]] §7).
- Changes go through PR review like code. A tier promotion or a new source is a reviewable diff.

---

## 10. Open Questions

- [ ] Who owns curation? This needs a named editor, not "the team" — registry quality is the single
      biggest lever on result quality and it degrades without an owner.
- [ ] Should trust tiers be public? Transparent, but invites arguments about placement.
- [ ] Do we crawl `.dz` domains we did not explicitly seed (open discovery), or stay registry-only?
      Registry-only for v1 ([[Crawler Orchestrator]] §12) — revisit with legal input.
- [ ] How do we cover the Algerian diaspora (`.fr`, `.com` sites about Algeria) without the TLD
      heuristic collapsing?

## Related

[[Data Model]] · [[Crawler Orchestrator]] · [[Admin and Source Submission]] · [[Legal and Compliance]] ·
[[Ranking and Relevance]] · [[Content Parser]] · [[Politeness and Robots]]
