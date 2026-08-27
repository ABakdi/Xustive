---
tags:
  - component
  - platform
component-id: C24
binary: xustive-api
status: partial
updated: 2026-08-27
---

# Admin and Source Submission

> **ID** C24 · **Binary** `xustive-api` (JSON under `/api/v1/admin/*`) + `xustive-cli`
> (`registry`, `takedown`, `keys`) + the Next.js console under `web/app/(operator)/admin` ·
> **Upstream** operators (public users: **not yet**) · **Downstream** [[Crawler Orchestrator]],
> [[Search Index]], [[Indexer Worker]]

## 0. What exists today (audited 2026-08-27)

Two of the three surfaces in this note are built; one is not.

| Surface | Status |
|:---|:---|
| Operator admin API + Next.js console | **Built** — see §3a for the real routes |
| Registry lifecycle (approve / activate / disable) + per-source policy | **Built** — CLI and console |
| Domain takedown (preview → typed confirmation → delete across three stores) | **Built** |
| Scoped Meilisearch keys (`xustive-cli keys`) | **Built** |
| Public "Submit a Source" (`POST /api/v1/sources`), moderation queue, captcha | **Not built** |
| Argon2id-hashed, scoped, rotating admin keys | **Not built** — one plain shared key, §10 |
| Append-only audit log file | **Not built** — actions go to the structured log only |
| Permanent `blocklist:takedown` | **Not built** — a takedown does not stop re-crawl, §4.3 |

The specification below is kept because it says *why* each of those matters; where it is
superseded, the paragraph says so.

## 1. Purpose

Two related surfaces:

1. **Public "Submit a Source"** — anyone can propose a site or public account for indexing. This is
   how a small search engine grows coverage it could never discover alone, and it is a
   [[Milestone 5 - Beta Launch]] deliverable. *(Still unbuilt as of 2026-08-27; coverage growth so
   far came from discovery instead — [[ADR-0013 - Direct SERP Collection for Discovery]],
   [[ADR-0017 - Query-Time Federation with External Metasearch]].)*
2. **Operator admin** — manage the source registry, force recrawls, inspect ingestion health, and
   execute takedowns.

## 2. Responsibilities

**In scope**: submission intake and validation; a moderation queue; registry CRUD; recrawl triggers;
takedown execution; audit logging.

**Out of scope**: crawling (→ [[Crawler Orchestrator]]); user accounts (there are none).

*Superseded 2026-08-27:* the original "no full admin UI in v1, CLI + minimal HTML" stance. The
console is now a full set of Next.js pages ([[UI - Admin Console]], [[PROB-003 - Admin Console Coverage]]); the hand-written HTML renderer in the Rust process was deleted with
[[ADR-0010 - Next.js for the Frontend]].

## 3. Interface

### 3a. As built — `crates/xustive-api/src/lib.rs`, nested at `/api/v1/admin`

All routes share one guard (`admin::authorise`, §10). All are JSON. The search-budget timeout
layer wraps the whole group.

| Method | Path | Handler | Purpose |
|:---|:---|:---|:---|
| `GET` | `/status` | `admin.rs` | device, GPU, models, logging, index alias, politeness flag |
| `GET` | `/config` | `admin.rs` | effective config with secrets redacted |
| `POST` | `/device` | `admin.rs` | switch CPU/GPU preference and `gpu_layers` at runtime |
| `POST` | `/log-level` | `admin.rs` | temporary tracing filter override with expiry |
| `POST` | `/politeness` | `admin.rs` | the guarded `ignore_politeness` flag |
| `GET` | `/media` · `/interaction` · `/eval` · `/queue` | `admin.rs`, `admin_eval.rs`, `admin_queue.rs` | read-only dashboards |
| `GET`/`POST` | `/integrations` | `admin.rs` | federation runtime switch ([[Federation Gateway]]) |
| `POST` | `/queue/replay` · `/queue/dead/replay` · `/queue/dead/drop` | `admin_queue.rs` | DLQ ([[Task Queue]]) |
| `GET` | `/crawler/status` · `/crawler/events` (SSE) · `/crawler/documents` · `/crawler/channels` · `/crawler/weak-coverage` · `/crawler/sources` · `/crawler/sources/health` | `admin_crawler.rs` | [[Crawler Console]] |
| `POST` | `/crawler/enqueue` · `/crawler/pause` · `/crawler/weak-coverage/forget` | `admin_crawler.rs` | [[Crawler Console]] |
| `POST` | `/crawler/sources` | `admin_crawler::add_source` | add a seed (`url`, `source_id?`, `trust?`, `category?`, `note?`) and crawl it **next** |
| `POST` | `/crawler/sources/remove` | `admin_crawler::remove_source` | drop a seed line; already-indexed documents stay |
| `POST` | `/crawler/registry` | `admin_crawler::registry_edit` | `{id, action?: approve\|activate\|disable, reason?, policy?}` |
| `POST` | `/takedown` | `admin_maintenance.rs` | `{domain, confirm, execute}` — §4.3 |

Seed categories are a fixed display list (`news, government, education, health, science-tech,
sport, culture, business, reference`); an unknown category is stored verbatim and grouped under
"other" rather than rejected.

The `policy` object on `/crawler/registry` accepts `enabled`, `frequency`, `max_docs_per_run`,
`crawl_delay_ms` (floored at 500 ms — it can be raised, never made impolite), `depth_limit`.
`respect_robots` is deliberately **not** editable from the console.

The console reaches these through the Next.js `/api/v1/:path*` rewrite (`web/next.config.*`), so
every call in `web/lib/admin.ts` is a same-origin fetch and the browser never holds a key.

CLI equivalents: `xustive-cli registry list|stats|lint|fmt|approve|activate|disable`,
`xustive-cli takedown --domain X [--yes]`, `xustive-cli keys [--show]`.

### 3b. As specified — the public submission surface (not built)

| Method | Path | Auth | Purpose |
|:---|:---|:---|:---|
| `POST` | `/api/v1/sources` | none + anti-abuse | public submission |
| `POST` | `/api/v1/admin/sources/{id}/approve` | key | promote from the moderation queue |

Submission body:

```jsonc
{ "url": "https://example.dz", "kind": "web",
  "name": "Example DZ", "languages": ["ar","fr"],
  "reason": "local news site for Béjaïa",
  "contact_email": "optional@example.dz" }
```

Response: `202 { "status": "pending_review", "submission_id": "01J8…" }` — never "accepted", because
nothing is crawled without review.

The rate-limit bucket for it already exists (`ratelimit::SOURCES = 5 per 3600 s`) and is the only
limit in `ratelimit.rs` with no route attached — a placeholder, not a feature.

## 4. Internal Design

### 4.1 Submission validation (synchronous) — specified, not built

| Check | Failure |
|:---|:---|
| `SafeUrl` (scheme, no private IPs, redirect-safe) | 400 |
| Not already in the registry | 200 with `status: "already_indexed"` |
| Not on the takedown blocklist | 400 `not_eligible`, no detail leaked |
| Host resolves and responds ≤ 10 s | queued anyway, flagged `unreachable` |
| Rate limit: 5/hour per IP /24, plus a proof-of-work or captcha | 429 |

The submitted URL is fetched **once**, sandboxed, only to extract a title, detect a language, and
check `robots.txt` — enough for a reviewer to judge without opening it themselves.

### 4.2 Moderation queue — specified, not built; the *lifecycle* half is built

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

What *is* built is the registry's lifecycle, which the queue would feed into: `proposed →
approved → active`, and `disable` (with a recorded reason) which starts a 90-day archival clock.
An archived source must be re-proposed, not resurrected from a button — the console and the CLI
enforce the same transitions ([[Data Sources Registry]]). The registry is the JSON-Lines file
`data/sources/registry.jsonl`, versioned in git; `registry fmt` re-canonicalises it after a hand
edit so the next machine write is a one-line diff.

### 4.3 Takedown execution

As built (`admin_maintenance.rs`, mirrored by `xustive-cli takedown`):

```
POST /admin/takedown { "domain": "example.dz", "confirm": "example.dz", "execute": true }
1. one filtered query `domain = "…"`, paged 1000 at a time, to collect ids + urls
2. execute:false → { matched, executed:false }  (preview; the default)
3. execute:true requires confirm == domain, else 400 confirm_mismatch
4. per document: delete from Meilisearch → delete image vectors (Qdrant, if image search is on)
   → forget the raw body (Redis raw store, if present)
5. tracing::warn! with counts; return { documents_removed, vector_groups_removed,
   raw_bodies_removed, note }
```

The `note` says it in the response and the UI says it again: **future crawling is NOT blocked** —
pair a takedown with disabling the source. The specified design below wanted a permanent
`blocklist:takedown` precisely because "deletion without blocklisting is a bug that looks like it
works"; that entry does not exist yet, and disabling the source is the stand-in. Comments are not
deleted because there is no comments pipeline yet ([[Deduplication Service]], [[Social Connector - Facebook]]).

As specified:

```
POST /admin/takedown { "target": {"document_id"|"url"|"domain"}, "reason": "...", "scope": "..." }
1. resolve affected document ids
2. [[Indexer Worker]] delete path: vectors → comments → document (order matters)
3. add URL/domain to blocklist:takedown (permanent)
4. if scope == domain: disable the Source, remove from frontier
5. append to the immutable audit log
6. return the count removed
```

Target SLA: **72 hours** from a valid request ([[Security and Privacy]] §8). Only `domain`
targeting exists today; a single-URL takedown means a domain takedown or a hand edit.

### 4.4 Audit log

Specified: every admin action appends `{ ts, key_id, action, target, reason, result }` to an
append-only file (and to `stdout` as structured JSON). This is the one deliberate log of activity
in the system, and it contains no end-user data.

Built: the `stdout` half only. Each mutating handler emits a `tracing` line (`warn!` for takedown,
pause, device and log-level changes; `info!` for enqueue, with the peer address). There is no
separate file and no `key_id` — there is one key (§10).

### 4.5 Admin UI

*Superseded 2026-08-27.* The console is the Next.js route group `web/app/(operator)/admin/*`
(overview, live, documents, sources, sources/health, discovery, weak-coverage, evaluation,
integrations, interaction, media, compute, config, queue, maintenance) with a sidebar in
`web/components/admin/AdminSidebar.tsx`. The old `/admin/crawler` URL redirects to `/admin/live`.
Interface budgets and layout are in [[UI - Admin Console]].

## 5. Configuration

As built — `[api]` in `config/*.toml`, `ApiConfig` in `crates/xustive-core/src/config.rs`:

| Key | Default | Notes |
|:---|:---|:---|
| `api.admin_key` | `""` | env `XUSTIVE_ADMIN_KEY`. Empty = loopback-only admin |
| `crawl.registry_path` | `data/sources/registry.jsonl` | |
| `crawl.seeds_path` | `data/sources/seeds.tsv` | what `/crawler/sources` edits |
| `crawl.documents_page_size` | 50 | admin document list |

Specified for the submission surface (none of these keys exist):
`submissions_enabled`, `submission_rate_limit`, `require_captcha`, `auto_approve` (locked false),
`default_trust_tier` C, `probe_timeout_s` 10, `takedown_sla_hours` 72, `audit_log_path`.

## 6. Data

Reads/writes the registry JSON-Lines file and the seed TSV (both in git, `data/sources/`), the
frontier and crawl-state keys in Redis, and the Meilisearch `documents` index for takedown.
Not yet: `submissions:pending`, `blocklist:takedown`, an audit file, a `sources` Meilisearch
index (the registry is file-backed, not indexed — [[Data Model]] §5 describes the intended shape).

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Submission flood | rate limiter + queue length | *(unbuilt)* tighten limits; pause submissions; alert |
| Probe fetch hangs | 10 s timeout | *(unbuilt)* queue as `unreachable`, still reviewable |
| SSRF attempt via an operator-entered URL | `SafeUrl` + trap detector | 400 `unsafe_url` / `trap` — built on `enqueue` and `add_source` |
| Takedown partially applied | per-store result counts | counts are reported per store; no retry loop yet |
| Redis unreachable on pause / enqueue | connect failure | 503 `redis_unavailable` / `no_frontier` — never a silent zero |
| Registry/index divergence | nightly reconciliation | *(unbuilt)* |
| Admin key compromised | audit anomalies | rotate the single key via env and restart |

## 8. Performance

| Operation | Budget |
|:---|:---|
| `POST /sources` (excluding probe) | ≤ 100 ms *(unbuilt)* |
| Takedown of one domain | one paged query + N deletes; bounded by the admin group's search timeout |
| Registry list | ≤ 200 ms |

## 9. Observability

Specified: `xustive_submissions_total{outcome}`, `xustive_submission_queue_depth`,
`xustive_admin_action_total{action}`, `xustive_takedown_docs_removed_total`,
`xustive_takedown_duration_seconds`, `xustive_sources_total{kind,approved}`,
`xustive_ssrf_blocked_total`.

Built: none of those metric names. Admin calls are counted only by the generic
`xustive_http_requests_total{route,status}`; actions are visible as log lines (§4.4).

## 10. Security

As built (`admin::authorise`): with `api.admin_key` set, every admin call must carry it in
`X-Admin-Key`, compared in constant time; with no key configured **only loopback peers are
admitted**, and an unknown peer address counts as remote. That is what lets `make web` work in a
browser with no setup without exposing device settings on a `0.0.0.0` bind. The Next.js server
proxies from loopback, which is why the browser never needs the key.

Specified and still ahead: Argon2id-hashed keys, scopes (`registry`, `takedown`, `readonly`),
90-day rotation ([[Security and Privacy]] §7). One plain key, one scope, is the honest current
state.

Operator-entered URLs (`enqueue`, `add_source`) pass `SafeUrl` and the frontier trap detector
exactly as a discovered link does. **The console reorders; it does not grant.**

For the public submission endpoint, when it exists: it would be the only unauthenticated *write*
path in the system, defended by `SafeUrl`, strict rate limits, captcha/proof-of-work, mandatory
human review, and default `trust_tier` C. Contact emails would be the only personal data we hold
and would be deleted after 30 days.

## 11. Testing

Built: `admin_crawler.rs` / `admin_maintenance.rs` unit tests cover the registry transitions,
policy floors, seed add/remove, and the takedown confirmation guard; the SSRF suite lives with
`SafeUrl` in `xustive-core`.

Still specified: the submission validation table, the moderation queue, the end-to-end takedown
that asserts comments/vectors/document are all gone **and that a subsequent crawl does not
re-index it** (impossible to pass until the blocklist exists), the rate-limit test, the audit test.

## 12. Open Questions

- [ ] Who are the reviewers at beta, and what is their throughput? The moderation queue is only as
      good as the humans behind it.
- [ ] Should approved submitters get notified when their source goes live? (Requires storing the
      email longer — trade-off against §10.)
- [ ] Do we publish a transparency report of takedowns (count, category, no identifying details)?
- [ ] Is captcha acceptable given the no-third-party-scripts rule? A self-hosted proof-of-work is the
      likely answer ([[Security and Privacy]] P7).
- [ ] When does the takedown blocklist get built? Until it does, every takedown is a two-step
      operation (delete + disable) and a re-added seed silently undoes it.

## Related

[[Data Sources Registry]] · [[Crawler Console]] · [[Crawler Orchestrator]] · [[Indexer Worker]] ·
[[Politeness and Robots]] · [[Security and Privacy]] · [[Legal and Compliance]] · [[API Contract]] ·
[[UI - Admin Console]] · [[PROB-003 - Admin Console Coverage]]
