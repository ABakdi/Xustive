---
tags:
  - component
  - platform
component-id: C24
binary: xustive-api
status: specified
updated: 2026-08-06
---

# Admin and Source Submission

> **ID** C24 · **Binary** `xustive-api` · **Upstream** operators, public users · **Downstream** [[Crawler Orchestrator]], [[Search Index]], [[Indexer Worker]]

## 1. Purpose

Two related surfaces:

1. **Public "Submit a Source"** — anyone can propose a site or public account for indexing. This is
   how a small search engine grows coverage it could never discover alone, and it is a
   [[Milestone 5 - Beta Launch]] deliverable.
2. **Operator admin** — manage the source registry, force recrawls, inspect ingestion health, and
   execute takedowns.

## 2. Responsibilities

**In scope**: submission intake and validation; a moderation queue; registry CRUD; recrawl triggers;
takedown execution; audit logging.

**Out of scope**: crawling (→ [[Crawler Orchestrator]]); a full admin UI in v1 (CLI + minimal HTML);
user accounts (there are none).

## 3. Interface

| Method | Path | Auth | Purpose |
|:---|:---|:---|:---|
| `POST` | `/api/v1/sources` | none + anti-abuse | public submission |
| `GET` | `/api/v1/admin/sources` | `X-Api-Key` | list/filter registry |
| `GET` | `/api/v1/admin/sources/{id}` | key | detail + last run stats |
| `POST` | `/api/v1/admin/sources` | key | create directly |
| `PATCH` | `/api/v1/admin/sources/{id}` | key | enable/disable, policy, trust tier |
| `POST` | `/api/v1/admin/sources/{id}/approve` | key | promote from the moderation queue |
| `POST` | `/api/v1/admin/recrawl` | key | inject `{source_id}` or `{url}` into the frontier |
| `POST` | `/api/v1/admin/takedown` | key | remove content + blocklist |
| `GET` | `/api/v1/admin/queues` | key | queue depths, DLQ counts |

Submission body:

```jsonc
{ "url": "https://example.dz", "kind": "web",
  "name": "Example DZ", "languages": ["ar","fr"],
  "reason": "local news site for Béjaïa",
  "contact_email": "optional@example.dz" }
```

Response: `202 { "status": "pending_review", "submission_id": "01J8…" }` — never "accepted", because
nothing is crawled without review.

## 4. Internal Design

### 4.1 Submission validation (synchronous)

| Check | Failure |
|:---|:---|
| `SafeUrl` (scheme, no private IPs, redirect-safe) | 400 |
| Not already in the registry | 200 with `status: "already_indexed"` |
| Not on the takedown blocklist | 400 `not_eligible`, no detail leaked |
| Host resolves and responds ≤ 10 s | queued anyway, flagged `unreachable` |
| Rate limit: 5/hour per IP /24, plus a proof-of-work or captcha | 429 |

The submitted URL is fetched **once**, sandboxed, only to extract a title, detect a language, and
check `robots.txt` — enough for a reviewer to judge without opening it themselves.

### 4.2 Moderation queue

Submissions land in `submissions:pending` with an auto-computed review packet:

- title, description, detected language, TLD
- `robots.txt` verdict from [[Politeness and Robots]]
- approximate size from the sitemap
- whether it duplicates an existing source's domain
- a spam heuristic score (recently-registered domain, no content, redirect chains)

A reviewer approves (→ registry with `trust_tier` C by default), rejects (with a reason), or defers.
Approval writes the `Source` record and injects the entry points into the frontier.

**Nothing is auto-approved.** The abuse surface is real: submissions are how someone poisons the
index ([[Security and Privacy]] T2), and one careless approval is worth more to a spammer than a
thousand attempts at any other vector.

### 4.3 Takedown execution

```
POST /admin/takedown { "target": {"document_id"|"url"|"domain"}, "reason": "...", "scope": "..." }
1. resolve affected document ids
2. [[Indexer Worker]] delete path: vectors → comments → document (order matters)
3. add URL/domain to blocklist:takedown (permanent)
4. if scope == domain: disable the Source, remove from frontier
5. append to the immutable audit log
6. return the count removed
```

Target SLA: **72 hours** from a valid request ([[Security and Privacy]] §8). The blocklist entry is
what prevents a re-crawl from silently undoing the removal — deletion without blocklisting is a bug
that looks like it works.

### 4.4 Audit log

Every admin action appends `{ ts, key_id, action, target, reason, result }` to an append-only file
(and to `stdout` as structured JSON). This is the one deliberate log of activity in the system, and
it contains no end-user data.

### 4.5 Admin UI

v1 is a `xustive-cli` binary plus a minimal server-rendered HTML page behind the API key. A real UI
is out of scope until [[Milestone 5 - Beta Launch]] — reviewers are a handful of operators, and CLI
ergonomics beat a half-built dashboard.

## 5. Configuration

| Key | Default |
|:---|:---|
| `submissions_enabled` | `false` until M5 |
| `submission_rate_limit` | 5/hour per IP /24 |
| `require_captcha` | `true` |
| `auto_approve` | **`false` (locked)** |
| `default_trust_tier` | `C` |
| `probe_timeout_s` | 10 |
| `takedown_sla_hours` | 72 |
| `audit_log_path` | `/var/log/xustive/audit.jsonl` |

## 6. Data

Reads/writes `Source` records ([[Data Model]] §5) in Meilisearch `sources` and Redis;
`submissions:pending` list; `blocklist:takedown` set; the audit log file. The registry is also
exported to git on change so it is reviewable and restorable ([[Deployment Topology]] §7).

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Submission flood | rate limiter + queue length | tighten limits; pause submissions; alert |
| Probe fetch hangs | 10 s timeout | queue as `unreachable`, still reviewable |
| SSRF attempt via submitted URL | `SafeUrl` | 400 + `xustive_ssrf_blocked_total`, alert |
| Takedown partially applied | per-step result | retry; **never** report success on partial removal |
| Blocklist write fails | error | fail the whole takedown loudly — a deletion without a blocklist is not a takedown |
| Registry/index divergence | nightly reconciliation | re-export registry from Meilisearch to git |
| Admin key compromised | audit anomalies | rotate; keys are scoped and revocable |

## 8. Performance

| Operation | Budget |
|:---|:---|
| `POST /sources` (excluding probe) | ≤ 100 ms |
| Probe fetch | ≤ 10 s, async |
| Takedown of one document | ≤ 5 s |
| Takedown of a domain (10k docs) | ≤ 5 min |
| Registry list | ≤ 200 ms |

## 9. Observability

`xustive_submissions_total{outcome}`, `xustive_submission_queue_depth`,
`xustive_admin_action_total{action}`, `xustive_takedown_docs_removed_total`,
`xustive_takedown_duration_seconds`, `xustive_sources_total{kind,approved}`,
`xustive_ssrf_blocked_total`. Alert if the submission queue exceeds 500 (review capacity exceeded) or
if any takedown misses the 72 h SLA.

## 10. Security

- Admin endpoints require an `X-Api-Key` verified against an Argon2id hash in constant time; keys are
  scoped (`registry`, `takedown`, `readonly`) and rotated every 90 days
  ([[Security and Privacy]] §7).
- The public submission endpoint is the only unauthenticated *write* path in the system. Its defences:
  `SafeUrl`, strict rate limits, captcha/proof-of-work, mandatory human review, and default
  `trust_tier` C so even an approved source cannot immediately dominate rankings.
- Submitted `reason` and `name` are stored raw and escaped at render time in the review UI.
- Contact emails, if provided, are stored for the moderation decision only and deleted after
  30 days — that is personal data and it is the only such data we hold.

## 11. Testing

- Unit: validation table; duplicate detection; blocklist precedence.
- Security: SSRF suite against the submission endpoint (private IPs, redirects, DNS rebinding);
  assert every case is blocked and counted.
- Takedown: end-to-end — index a document with images and comments, take it down, assert all three
  stores are clean, the URL is blocklisted, and a subsequent crawl does not re-index it.
- Partial failure: make the blocklist write fail; assert the takedown reports failure rather than
  success.
- Rate limit: 6 submissions in an hour → the 6th is 429.
- Audit: every admin action produces exactly one audit line with no user data.

## 12. Open Questions

- [ ] Who are the reviewers at beta, and what is their throughput? The moderation queue is only as
      good as the humans behind it.
- [ ] Should approved submitters get notified when their source goes live? (Requires storing the
      email longer — trade-off against §10.)
- [ ] Do we publish a transparency report of takedowns (count, category, no identifying details)?
- [ ] Is captcha acceptable given the no-third-party-scripts rule? A self-hosted proof-of-work is the
      likely answer ([[Security and Privacy]] P7).

## Related

[[Data Sources Registry]] · [[Crawler Orchestrator]] · [[Indexer Worker]] · [[Politeness and Robots]] ·
[[Security and Privacy]] · [[Legal and Compliance]] · [[API Contract]]
