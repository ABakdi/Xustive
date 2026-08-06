---
tags:
  - component
  - ingestion
  - social
  - collection
component-id: C13
binary: xustive-crawler
status: specified
updated: 2026-08-06
---

# Social Connector - Facebook

> **ID** C13 · **Binary** `xustive-crawler` · **Upstream** [[Crawler Orchestrator]] · **Downstream** [[Content Parser]]
> **Collection stance:** direct, per [[ADR-0009 - Direct Collection for Social Platforms]]. Graph API
> is used opportunistically where a source has authorised it, because it is cheaper and more stable.

## 1. Purpose

Ingest public Algerian Facebook content — the largest pool of Algerian-language user content, where
classifieds, job posts, announcements, and civic discussion actually happen.

Facebook is the **hardest** of the three platforms to collect and the most valuable. It has the most
mature anti-automation stack, the most aggressive account challenges, and the least useful anonymous
surface. Expect the highest identity burn rate and the highest cost per document here.

## 2. Responsibilities

**In scope**: access-path selection; page/group enumeration; post and comment pagination; GraphQL
`doc_id` handling; mapping to the canonical envelope; cursor management; per-identity pacing;
detection response.

**Out of scope**: identity/session state (→ [[Session Manager]]); fingerprints
(→ [[Fingerprint Engine]]); `fb_dtsg`/`lsd` minting (→ [[Signature Service]]); parsing
(→ [[Content Parser]]).

## 3. Interface

Consumes `q:fetch` with `kind = "facebook"`:
`{ source_id, object_id, object_type: "page"|"group", cursor?, since?, path_hint? }`

Produces `q:parse`: `{ kind: "facebook", raw_ref, source_id, object_id, fetched_at, access_path, page_info }`

`access_path` is recorded on every message so that when a path breaks, we can tell exactly which
documents came from it and re-collect only those.

## 4. Internal Design

### 4.1 Access-path ladder

Tried in order of stability and cost. The connector records which path produced each document.

| # | Path | Auth | Yield | Fragility |
|:--|:---|:---|:---|:---|
| 1 | **Graph API** — where the source authorised our app | token | full, clean | low |
| 2 | **`mbasic.facebook.com`** — minimal HTML, no JS | logged-in identity | good; simple stable HTML | medium (being wound down) |
| 3 | **`m.facebook.com`** — mobile web | logged-in identity | good | medium |
| 4 | **`www.facebook.com` GraphQL** — `/api/graphql/` with `doc_id`, `fb_dtsg`, `lsd` | logged-in identity | richest, paginates cleanly | **high** — `doc_id` values rotate |
| 5 | **Public page HTML** — logged-out | anonymous | thin, often a login wall | high |

Design guidance: **prefer the lightest path that yields the fields we need.** `mbasic` returns a few
KB of plain HTML per page versus hundreds of KB of JSON and assets on `www` — which directly
determines residential bandwidth spend ([[Proxy Manager]] §8). The instinct to use the richest
endpoint is usually the wrong one here.

Path selection is per source, stored in the registry, with automatic demotion: if path 4 fails
repeatedly for an object, fall back to 2 or 3 and flag it.

### 4.2 GraphQL specifics (path 4)

- `doc_id` (persisted query id) values are harvested from the page bundle and treated as session
  constants by [[Signature Service]] §4.1. They rotate on Facebook's release cadence.
- `fb_dtsg` and `lsd` are per-session CSRF tokens, harvested at bootstrap and reused for the session.
- `jazoest` is a checksum over `fb_dtsg`; computed, not harvested.
- Pagination uses the `end_cursor` from `page_info`; cursors are opaque and expire, so a stalled
  window is restarted rather than resumed after `cursor_max_age`.

### 4.3 Groups

Groups are the highest-value target and require a logged-in identity in essentially all cases.

| Group type | Requirement |
|:---|:---|
| Public group | logged-in identity, no membership needed |
| Public group with content gating | membership |
| Closed / private group | membership — **per-source decision, default off** ([[ADR-0009 - Direct Collection for Social Platforms]] §Decision.6) |

Joining is not automated. Where a source's registry entry sets `join_required = true`, an operator
performs the join with a designated identity and records it. Automated join requests are a strong
detection signal and generate visible activity in the group.

Group feed ordering is unstable (Facebook reorders by "top posts" unpredictably), so incremental
collection uses `since`-based windows with a **2-hour overlap** and relies on
[[Deduplication Service]] rather than trusting cursor continuity.

### 4.4 Pacing

Facebook challenges accounts faster than the other two platforms. Starting budgets, deliberately low
([[Session Manager]] §4.5):

| Setting | Value |
|:---|:---|
| Requests/hour/identity | 45 |
| Requests/day/identity | 300 |
| Min gap | 3 000 ms ±40 % |
| Session length | 8–25 requests, then idle 20–90 min |
| Active hours | 6–14 per identity, offset, `Africa/Algiers` |
| Concurrent identities per group | 1 |

Comment fetching is the expensive part — a post with 300 comments costs several paginated requests.
`max_comments_per_post` (200) caps it, and comment collection is skipped entirely for posts below
`min_engagement_for_comments` (5 comments), since thin threads rarely carry the answer.

### 4.5 Mapping to `Document`

| Source field | `Document` field |
|:---|:---|
| post id | `platform_post_id` |
| message / text | `body`; `title` = first 80 chars |
| creation time | `published_at` (precision `second`) |
| permalink | `url` |
| author name / id | `author.name` / `author.id` |
| reactions total | `engagement.likes` |
| comment count | `comments_count` |
| share count | `engagement.shares` |
| attachment media | `media[]` (capped at 4) |
| — | `source_type = "facebook"` |

Relative timestamps ("2 h", "منذ ساعتين", "hier à 14:03") appear on `mbasic`/`m` paths and are
resolved against `fetched_at` in `Africa/Algiers`, with `published_at_precision` downgraded to `day`
when only a date is available ([[Content Parser]] §4.3).

Comments become `Comment` records with one level of threading retained.

### 4.6 Refresh and deletion

- New posts every run; engagement refreshed for posts < 7 days old on a 24 h cycle.
- Deleted or newly-restricted upstream (404 / login wall on refetch): mark `gone`, remove from the
  index within 24 h. **This is unchanged by the collection-method decision** — honouring upstream
  deletion is a duty owed to the person who deleted the post, not to the platform.

## 5. Configuration

| Key | Default |
|:---|:---|
| `access_path_order` | `["graph","mbasic","m","graphql","public_html"]` |
| `posts_page_size` | 50 |
| `max_comments_per_post` | 200 |
| `min_engagement_for_comments` | 5 |
| `overlap_window_s` | 7200 |
| `cursor_max_age_s` | 3600 |
| `engagement_refresh_days` | 7 |
| `join_required` | per source, default `false` |
| `pool` | `residential`, `geo = DZ`, sticky |
| `budgets` | §4.4 |

## 6. Data

Raw HTML/JSON → `raw:{trace_id}` (7-day TTL); messages → `q:parse`; cursors and window markers in
Redis. Identity state belongs to [[Session Manager]]; this connector never handles credentials.

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Login wall on a previously-anonymous object | body fingerprint | retry with `LoggedIn` capability |
| Checkpoint / captcha | body fingerprint | quarantine identity, retry on another |
| `doc_id` rotated | GraphQL 4xx / empty | [[Signature Service]] re-harvest; demote to path 2/3 meanwhile |
| `mbasic` path removed | 404/redirect across all identities | demote permanently, alert, re-plan bandwidth |
| **Empty feed on a group known to be active** | canary disagreement | soft ban → quarantine identity |
| Cursor expired mid-window | pagination error | restart the window with overlap |
| Group membership revoked | access error | mark source degraded, notify operator |
| Platform-wide challenge spike | > 30 % in 15 min | **halt Facebook entirely**, page |
| DOM/selector change (HTML paths) | extraction miss rate | alert; fix parser rules ([[Content Parser]] §4.1) |

The empty-feed case is the one to design for. Facebook prefers to serve a plausible empty page over
an error, so a connector that trusts HTTP 200 will report healthy while collecting nothing.

## 8. Performance

| Metric | Budget |
|:---|:---|
| Posts/hour/identity | ~200 (budget-bound, not CPU-bound) |
| Bandwidth per post (`mbasic`) | ≤ 60 KB |
| Bandwidth per post (`www` GraphQL) | ≤ 400 KB |
| Identity lifespan | ≥ 60 days median (lower than other platforms — expected) |
| Cost per 1 000 documents | tracked; the metric that decides source viability |

## 9. Observability

`xustive_fetch_total{source_type="facebook",outcome,access_path}`,
`xustive_fb_access_path_total{path}`, `xustive_fb_empty_feed_total`,
`xustive_fb_challenge_total{kind}`, `xustive_fb_doc_id_age_days`,
`xustive_fb_bandwidth_bytes{path}`, `xustive_source_disabled_total{reason}`.

Alerts: `FacebookChallengeSpike` (page), `FacebookCanaryDown` (page), `AccessPathDemoted` (ticket).

## 10. Security and Obligations

Collection method changed; downstream duties did not
([[ADR-0009 - Direct Collection for Social Platforms]] §"What Does Not Change"):

- **No person-centric profiling.** We index posts. There is no "all posts by this author" view, no
  author timeline, no social-graph reconstruction. Author fields exist for attribution.
- **Upstream deletions are honoured** within 24 h.
- **Takedown path** removes content, comments, and vectors permanently and blocklists the URL
  ([[Indexer Worker]] §4.5).
- **Personal data obligations under Law 18-07 are unaffected** by how the data was collected
  ([[Legal and Compliance]] §5).
- No face recognition on any collected imagery ([[Image Pipeline]] §10).
- Identity credentials never leave [[Session Manager]]; this connector receives a lease, not secrets.

## 11. Testing

- Recorded fixtures per access path (`tests/fixtures/facebook/{mbasic,m,graphql,graph}/`): post with
  photo, post with video, 200-comment thread, edited post, deleted post, login wall, checkpoint page,
  **empty-but-200 feed**.
- Mapping tests for every row of §4.5 including relative-timestamp resolution across timezones.
- Path-ladder test: fail path 4, assert demotion to 2/3 and correct `access_path` recording.
- Cloaking test: empty feed + healthy canary → identity quarantined, source **not** marked empty.
- Pacing test: assert budgets and jitter are honoured over a simulated 24 h.
- Deletion propagation: fixture 404 on refresh → removed within the SLA.
- **No live-platform requests in CI**, ever. Production canaries cover live breakage.

## 12. Open Questions

- [ ] How long does `mbasic` survive? It is the cheapest path by a wide margin; plan for its removal.
- [ ] Identity pool size for Facebook — at 300 requests/day/identity, what pool covers the target
      group set with 40 % quarantine headroom?
- [ ] Do we index comment author display names, or text only? (Leaning: name only, no identifiers.)
- [ ] Private/closed groups — which, if any, and who authorises each?
- [ ] What is the retention policy for content from a group we later lose access to?

## Related

[[ADR-0009 - Direct Collection for Social Platforms]] · [[Session Manager]] · [[Proxy Manager]] ·
[[Fingerprint Engine]] · [[Signature Service]] · [[Content Parser]] · [[Data Model]] ·
[[Social Connector - Instagram]] · [[Social Connector - TikTok]] · [[Legal and Compliance]]
