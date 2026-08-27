---
tags:
  - component
  - ingestion
  - social
  - collection
component-id: C14
binary: none (would be xustive-cli crawld)
status: not built
updated: 2026-08-27
---

# Social Connector - Instagram

> **ID** C14 · **Binary** none yet (would live in `xustive-cli crawld`) ·
> **Upstream** [[Crawler Orchestrator]] · **Downstream** [[Content Parser]], [[Image Pipeline]]
> **Collection stance:** direct, per [[ADR-0009 - Direct Collection for Social Platforms]].

## 0. Status (2026-08-27)

**Not built.** No connector code exists for Instagram — nothing in `crates/`, `services/`, `web/`
or `config/` fetches, paginates or maps Instagram content, and there is no `xustive-crawler` binary;
the crawler is `xustive-cli crawld` ([[Crawler Orchestrator]]). What *does* exist is the ground a
connector would stand on, so the design below is kept as the plan, not as a description:

- `xustive_core::model::SourceType::{Web, Facebook, Instagram, Tiktok}` with `is_platform()`
  (`platform` vs `open_web` crawl profile), parsed from `"facebook" | "fb"`, etc., and usable in a
  search `source_type` filter ([[Data Model]]).
- `Engagement { likes, comments, shares, views, captured_at }` on `Document`.
- `xustive_ingest::session::Platform::{Instagram, Facebook, Tiktok}`, the identity pool, pinning
  invariant and budgets ([[Session Manager]]); the proxy ladder with `Datacenter`/`Residential`/
  `Mobile` pools ([[Proxy Manager]]); fingerprint coherence ([[Fingerprint Engine]]). All are
  "engine built, fuel deferred" — decision logic with tests, no real accounts or proxies.
- The queue has one stream, `q:index` ([[Task Queue]]); `q:fetch` / `q:parse` below are the
  intended shape, not streams that exist.

The only social content reachable today comes sideways: query-time federation may return a
Instagram URL from an external metasearch engine ([[ADR-0017 - Query-Time Federation with External Metasearch]]), and it is indexed as an ordinary web result, not through this connector.

## 1. Purpose

Ingest public Instagram content from Algerian accounts and hashtags: brands, media outlets, public
institutions, event organisers, and creators.

Instagram content is **image-first**, which makes it the highest-value input to [[Image Pipeline]] —
much of its actual text lives inside images rather than in captions. A post with an empty caption and
a text-heavy graphic is a real document once OCR reaches it.

## 2. Responsibilities

**In scope**: access-path selection; profile and hashtag enumeration; media and comment retrieval;
mapping to the canonical envelope; expiring-CDN-URL handling; pacing; detection response.

**Out of scope**: identities (→ [[Session Manager]]); fingerprints (→ [[Fingerprint Engine]]);
`X-IG-WWW-Claim` and app-id handling (→ [[Signature Service]]); image analysis
(→ [[Image Pipeline]]).

## 3. Interface

_Intended shape (2026-08-27): none of these streams or message kinds exist yet; see §0._

Consumes `q:fetch` with `kind = "instagram"`:
`{ source_id, ig_user_id | hashtag, cursor?, since?, path_hint? }`

Produces `q:parse`: `{ kind: "instagram", raw_ref, source_id, fetched_at, access_path }`

## 4. Internal Design

### 4.1 Access-path ladder

| # | Path | Auth | Yield | Fragility |
|:--|:---|:---|:---|:---|
| 1 | **Graph API** — authorised business/creator accounts | token | full, clean | low |
| 2 | **Web GraphQL** — `/graphql/query` with `X-IG-App-ID`, `X-IG-WWW-Claim` | logged-in identity | full profile feeds, paginates well | medium |
| 3 | **Mobile private API** — `i.instagram.com/api/v1/` | logged-in identity + device profile | richest; best pagination | medium-high |
| 4 | **Embedded JSON** — hydration blob in public profile HTML | anonymous | first ~12 posts, no pagination | **low** ← most stable |
| 5 | **oEmbed** | none | single-post metadata | low |

Path 4 deserves attention despite its thin yield: it needs no signing, no login, and survives most
platform changes ([[Signature Service]] §4.6). For a source we only need to check daily for a handful
of new posts, it is often sufficient — and it costs nothing in identity risk. **Prefer it for
low-frequency sources**; reserve logged-in paths for deep backfill.

### 4.2 Anonymous access reality

Instagram's logged-out surface has narrowed considerably: anonymous requests are aggressively rate
limited and frequently redirect to a login wall after a handful of requests from one address. The
connector therefore:

- Uses `Capability::Anonymous` first for path 4/5 (cheap, risks only a proxy IP).
- Treats a login wall as a normal, expected outcome — not an error — and escalates to a logged-in
  identity for that object.
- Keeps a **separate low-value identity tier** for anonymous-path work, so a burn there costs nothing
  warm.

### 4.3 Hashtags

Hashtag collection is high-noise and high-cost. Rules:

- Curated hashtag list per topic in [[Data Sources Registry]] (`#algerie`, `#dz`, `#oran`,
  `#emploi_algerie`, …), ranked and rotated on a schedule.
- Hashtag results are assigned `trust_tier` C and elevated `spam_score` scrutiny — hashtag feeds are
  where marketing spam concentrates.
- `max_posts_per_hashtag_run` (200) caps spend, because a broad hashtag is effectively infinite.

### 4.4 Expiring media URLs ★

Instagram CDN URLs are signed and expire within hours. This is the connector's defining operational
constraint:

- The gap between collecting a post and fetching its image must stay short. Instagram media is
  **priority-boosted** in `q:enrich` ([[Enrichment Pipeline]] §4.4).
- Target: post → image fetched within **30 minutes**.
- On a 403 fetching media, re-fetch the parent post once to refresh the URL, then retry. A second
  failure drops the media and keeps the document.
- Media is fetched through `direct` or `datacenter` where the CDN permits, keeping image bandwidth off
  the residential meter ([[Proxy Manager]] §8) — subject to the open question in that note about
  whether a split egress is itself correlatable.

### 4.5 Mapping to `Document`

| Source field | `Document` field |
|:---|:---|
| media id | `platform_post_id` |
| caption | `body`; `title` = first 80 chars |
| taken_at / timestamp | `published_at` (precision `second`) |
| permalink | `url` |
| username | `author.handle` / `author.name` |
| like_count / comment_count | `engagement.likes` / `comments_count` |
| media_url, carousel children | `media[]` (one per child, capped at 4) |
| video → `type = "video"` + cover | `media[].thumb_url` |
| hashtags in caption | `topics` |
| — | `source_type = "instagram"` |

**Empty-caption handling:** if the caption is empty but OCR yields usable text, the document is
indexed with `body` from OCR and `body_source = "ocr"` recorded. If both are empty, the document is
**dropped** — an image-only post with no extractable text is noise in a text index.

### 4.6 Pacing

| Setting | Value |
|:---|:---|
| Requests/hour/identity | 60 |
| Requests/day/identity | 400 |
| Min gap | 2 500 ms ±40 % |
| Session length | 8–25 requests, then idle |
| Concurrent identities per profile | 1 |
| Anonymous-path requests/hour/proxy | 30 |

### 4.7 Refresh and deletion

New media every run; engagement refreshed for posts < 7 days old; 404 on refresh → `gone`, removed
within 24 h. Unchanged by the collection decision.

## 5. Configuration

_Intended keys (2026-08-27): no `[instagram]` section exists in `config/*.toml`._

| Key | Default |
|:---|:---|
| `access_path_order` | `["graph","embedded_json","web_graphql","mobile_api","oembed"]` |
| `media_page_size` | 50 |
| `max_children_per_post` | 4 |
| `max_comments_per_post` | 100 |
| `max_posts_per_hashtag_run` | 200 |
| `media_fetch_deadline_min` | 30 |
| `engagement_refresh_days` | 7 |
| `anonymous_identity_tier` | `low_value` |
| `pool` | `residential`, `geo = DZ`, sticky |
| `drop_if_no_text` | `true` |

`access_path_order` puts `embedded_json` **second**, ahead of the richer logged-in paths, precisely
because it costs no identity risk.

## 6. Data

Raw JSON/HTML → `raw:{trace_id}`; messages → `q:parse`; cursors and hashtag rotation state in Redis.
Media URLs are not downloaded here — [[Enrichment Pipeline]] fetches them, so image bandwidth is
accounted separately from collection bandwidth.

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Login wall on anonymous path | body fingerprint | escalate to logged-in identity; not an error |
| Rate limited | 429 / interstitial | quarantine identity, back off |
| Checkpoint | body fingerprint | quarantine → [[Session Manager]] recovery |
| **Media URL expired** | 403 on image fetch | re-fetch parent once, then drop media, keep document |
| `X-IG-WWW-Claim` not propagated | rising challenge rate | header-propagation test ([[Signature Service]] §4.4) |
| Empty profile feed on an active account | canary disagreement | soft ban → quarantine |
| Carousel child missing | field absent | index what exists |
| Caption empty and OCR unusable | post-enrichment check | **drop the document** |
| Hydration blob shape changed | parse failure | alert; demote to another path |

## 8. Performance

| Metric | Budget |
|:---|:---|
| Posts/hour/identity | ~250 |
| Post → media fetched | ≤ 30 min p95 |
| Bandwidth per post (embedded JSON) | ≤ 80 KB |
| Bandwidth per post (logged-in GraphQL) | ≤ 250 KB |
| Identity lifespan | ≥ 90 days median |

## 9. Observability

`xustive_fetch_total{source_type="instagram",outcome,access_path}`,
`xustive_ig_access_path_total{path}`, `xustive_ig_media_url_expired_total`,
`xustive_ig_login_wall_total`, `xustive_ig_empty_caption_total`,
`xustive_ig_media_fetch_lag_seconds`, `xustive_ig_challenge_total`.

`xustive_ig_media_fetch_lag_seconds` is the one to watch — when it climbs past the CDN expiry window,
image collection silently starts failing and Instagram documents lose most of their content.

## 10. Security and Obligations

Unchanged from [[ADR-0009 - Direct Collection for Social Platforms]] §"What Does Not Change":

- No person-centric profiling, no author-history view, no follower-graph reconstruction.
- Upstream deletions honoured within 24 h; takedowns permanent and blocklisted.
- **No face recognition** on any collected imagery, and EXIF/GPS stripped — Instagram content is
  personal-data-rich, and this exclusion is permanent ([[Image Pipeline]] §10).
- Law 18-07 obligations apply regardless of collection method ([[Legal and Compliance]] §5).

## 11. Testing

- Recorded fixtures per path: single image, carousel, video, empty caption, expired media URL, login
  wall, checkpoint, hydration blob, **empty-but-200 feed**.
- Path-ladder test: anonymous → login wall → logged-in escalation, with `access_path` recorded.
- Media-lag test: assert enrichment priority keeps the post→image gap inside the deadline.
- Expiry test: 403 on media → one parent re-fetch → success; second failure keeps the document.
- Drop rule: empty caption + unusable OCR → dropped; empty caption + usable OCR → indexed with
  `body_source = "ocr"`.
- Pacing and jitter over a simulated 24 h.
- **No live-platform requests in CI.**

## 12. Open Questions

- [ ] How much of the target corpus is reachable via the embedded-JSON path alone? If it is most of
      it for daily-frequency sources, identity risk drops sharply — worth measuring early.
- [ ] Mobile private API needs full device emulation (device id, install id, app version signing).
      Is the extra yield worth a second profile family in [[Fingerprint Engine]]?
- [ ] Is hashtag collection worth its noise and cost, or should spend go entirely to curated profiles?
- [ ] Do we fetch media through a different egress than pages, and is that split correlatable?

## Related

[[ADR-0009 - Direct Collection for Social Platforms]] · [[Session Manager]] · [[Proxy Manager]] ·
[[Signature Service]] · [[Image Pipeline]] · [[Enrichment Pipeline]] · [[Data Sources Registry]] ·
[[Social Connector - Facebook]] · [[Social Connector - TikTok]] · [[Legal and Compliance]]
