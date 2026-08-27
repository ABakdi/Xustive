---
tags:
  - component
  - ingestion
  - social
  - collection
component-id: C15
binary: none (would be xustive-cli crawld)
status: not built
updated: 2026-08-27
---

# Social Connector - TikTok

> **ID** C15 · **Binary** none yet (would live in `xustive-cli crawld`) ·
> **Upstream** [[Crawler Orchestrator]] · **Downstream** [[Content Parser]], [[Image Pipeline]]
> **Collection stance:** direct, per [[ADR-0009 - Direct Collection for Social Platforms]].

## 0. Status (2026-08-27)

**Not built.** No connector code exists for TikTok — nothing in `crates/`, `services/`, `web/`
or `config/` fetches, paginates or maps TikTok content, and there is no `xustive-crawler` binary;
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
TikTok URL from an external metasearch engine ([[ADR-0017 - Query-Time Federation with External Metasearch]]), and it is indexed as an ordinary web result, not through this connector.

## 1. Purpose

Index public TikTok video metadata — captions, hashtags, author, engagement, and cover frames — for
Algerian creators and hashtags.

TikTok is the **easiest** of the three to collect at scale (much is reachable anonymously) and the
most valuable linguistically: its captions are dense, current Darija, which makes it the best
available corpus for tuning [[Query Expander]] and [[Language Detector]] lexicons.

## 2. Responsibilities

**In scope**: access-path selection; profile, hashtag, and keyword enumeration; video metadata and
caption retrieval; comment retrieval; cover-frame handoff; mapping; pacing; detection response.

**Out of scope**: `X-Bogus`/`msToken` minting (→ [[Signature Service]]); identities
(→ [[Session Manager]]); **downloading video files** — permanently excluded on bandwidth, storage,
and copyright grounds.

## 3. Interface

_Intended shape (2026-08-27): none of these streams or message kinds exist yet; see §0._

Consumes `q:fetch` with `kind = "tiktok"`:
`{ source_id, query_kind: "user"|"hashtag"|"keyword", value, cursor?, since?, path_hint? }`

Produces `q:parse`: `{ kind: "tiktok", raw_ref, source_id, fetched_at, access_path }`

## 4. Internal Design

### 4.1 Access-path ladder

| # | Path | Auth | Yield | Fragility |
|:--|:---|:---|:---|:---|
| 1 | **Research API** — if approved | token | clean, includes `voice_to_text` | low |
| 2 | **Embedded hydration** — `__UNIVERSAL_DATA_FOR_REHYDRATION__` in profile/video HTML | anonymous | first page of a profile, full video detail | **low** ← most stable |
| 3 | **Web API** — `/api/post/item_list/`, `/api/challenge/item_list/` | anonymous + signatures | full pagination | **high** — signer rotation |
| 4 | **Mobile API** | device profile | richest | high |

Path 2 is the workhorse. It requires no signing, no account, and survives signer rotations entirely —
so a profile checked daily for new uploads costs one plain page fetch. Path 3 is used for deep
backfill and hashtag enumeration, where pagination is needed.

### 4.2 Signature dependency

Path 3 requires `X-Bogus`/`X-Gnarly` and `msToken`, computed by [[Signature Service]] from
TikTok's `webmssdk.js`. This is the single most fragile dependency in the connector: **when TikTok
rotates the signer, path 3 fails completely and immediately.**

Response: the connector demotes to path 2 automatically and continues at reduced yield rather than
halting, while `xustive_signer_failure_rate` pages for re-extraction
([[Signature Service]] §4.5). This graceful demotion is why the path ladder exists.

### 4.3 Windowing

Collection walks fixed date windows (default 1 day) per source rather than following an open cursor.
Each `(source, window)` is an independent work item, which makes backfill and resume trivial: a
failed window is simply retried, and completion markers make replay idempotent
([[Deduplication Service]]).

### 4.4 `voice_to_text`

Where TikTok exposes auto-generated captions, they are gold: a video becomes indexable Darija text
without us running ASR. Always requested, always treated as optional, appended to `body` with
`body_source = "caption+asr"` recorded.

`xustive_tt_voice_to_text_ratio` is tracked as a product metric — if coverage is high, TikTok becomes
a far richer text source than its caption lengths suggest.

### 4.5 Cover frames

Only the cover image is fetched (size-capped) and handed to [[Image Pipeline]] for OCR and CLIP.
TikTok covers frequently carry the actual message as overlay text, so OCR yield here is high.

**No video bytes are ever downloaded.** `download_video` exists in config only to be explicitly
`false`, and a test asserts no code path writes video data.

### 4.6 Mapping to `Document`

| Source field | `Document` field |
|:---|:---|
| video id | `platform_post_id` |
| description / caption | `body`; `title` = first 80 chars |
| createTime | `published_at` (precision `second`) |
| `tiktok.com/@{user}/video/{id}` | `url` |
| author unique id | `author.handle` |
| hashtags | `topics` |
| play / digg / comment / share counts | `engagement.*` |
| `voice_to_text` | appended to `body` |
| cover URL | `media[0]` (`type = "video"`, `thumb_url`) |
| region code | relevance filter, not `geo.wilaya` |
| — | `source_type = "tiktok"` |

### 4.7 Engagement volatility

TikTok view counts move by orders of magnitude within days — 200 to 2M is routine. Since
`engagement_norm` feeds [[Ranking and Relevance]], stale counts distort ranking badly.

- Refresh engagement for videos < 14 days old on a 24 h cycle.
- Normalise against **TikTok's own P95**, never a cross-platform one — TikTok views are not
  comparable to Facebook likes.
- Cap `engagement_norm` at P95 rather than P99 so a single viral video cannot dominate a result page.

### 4.8 Pacing

Anonymous paths are limited per proxy rather than per identity, which is far cheaper:

| Setting | Value |
|:---|:---|
| Requests/hour/proxy (anonymous) | 300 |
| Requests/day/proxy | 2 500 |
| Min gap | 800 ms ±30 % |
| Requests/hour/identity (logged-in) | 200 |
| Concurrent per source | 1 |

## 5. Configuration

_Intended keys (2026-08-27): no `[tiktok]` section exists in `config/*.toml`._

| Key | Default |
|:---|:---|
| `access_path_order` | `["research_api","embedded_hydration","web_api","mobile_api"]` |
| `window_days` | 1 |
| `page_size` | 35 |
| `max_comments_per_video` | 100 |
| `region_filter` | `["DZ"]` |
| `hashtag_list` | from registry |
| `engagement_refresh_days` | 14 |
| `fetch_cover_images` | `true` |
| `download_video` | **`false` (locked)** |
| `pool` | `datacenter`, `geo = DZ` |

TikTok is the one platform where `datacenter` is usually sufficient — anonymous endpoints are far
less IP-sensitive than Meta's. That materially reduces cost ([[Proxy Manager]] §8).

## 6. Data

Raw JSON/HTML → `raw:{trace_id}`; messages → `q:parse`; per-`(source, window)` completion markers in
Redis for idempotent backfill. No video bytes stored, ever.

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| **Signer rotated** | signature failure rate > 30 % | demote to path 2, page for re-extraction |
| `msToken` expired | 4xx | re-mint from [[Signature Service]] cache |
| Rate limited | 429 / captcha page | rotate proxy, back off; anonymous paths make this cheap |
| Hydration blob shape changed | parse failure | alert; demote to path 3 if signatures work |
| Window returns partial data | `has_more` + cursor | continue paging; window stays incomplete until drained |
| `voice_to_text` absent | field null | proceed with caption only |
| Cover URL expired | 403 | re-query the video once |
| Video deleted upstream | absent on refresh | remove within 24 h |
| Engagement wildly inconsistent | sanity check | accept, but cap `engagement_norm` at P95 |
| Empty profile feed on an active creator | canary disagreement | soft ban → rotate proxy |

## 8. Performance

| Metric | Budget |
|:---|:---|
| Videos/hour/proxy (path 2) | ~250 |
| Videos/hour/proxy (path 3) | ~300 |
| Window completion | ≤ 10 min per source-day |
| Bandwidth per video (path 2) | ≤ 120 KB |
| Bandwidth per video (path 3) | ≤ 40 KB |
| Cover OCR | see [[Image Pipeline]] §8 |

## 9. Observability

`xustive_fetch_total{source_type="tiktok",outcome,access_path}`,
`xustive_tt_access_path_total{path}`, `xustive_tt_window_lag_days`,
`xustive_tt_voice_to_text_ratio`, `xustive_tt_signature_failure_total`,
`xustive_tt_empty_feed_total`, `xustive_tt_captcha_total`.

## 10. Security and Obligations

Unchanged from [[ADR-0009 - Direct Collection for Social Platforms]] §"What Does Not Change":

- Captions and public metadata only; no video re-hosting, no video downloads.
- No person-centric profiling, no author-history view.
- Upstream deletions honoured within 24 h; takedowns permanent.
- **Minors are heavily represented on this platform.** We index captions and public metadata, never
  build author profiles, and the NSFW filter is default-on for any surfaced imagery. No face
  recognition ([[Image Pipeline]] §10). This is a design constraint, not a preference.
- Law 18-07 obligations apply regardless of collection method ([[Legal and Compliance]] §5).

## 11. Testing

- Recorded fixtures: video with and without `voice_to_text`, hashtag response, paginated window,
  deleted video, captcha interstitial, hydration blob, **empty-but-200 feed**.
- **Signer-rotation drill**: invalidate the signer; assert automatic demotion to path 2 and continued
  collection at reduced yield, plus the page firing.
- Window idempotency: replay a completed window, assert zero duplicate documents.
- Engagement: simulate a 1 000× view jump, assert `engagement_norm` stays capped.
- Mapping: hashtags → `topics`; `voice_to_text` concatenation and `body_source` marking.
- Assert **no code path writes video bytes** to disk or storage.
- **No live-platform requests in CI.**

## 12. Open Questions

- [ ] What fraction of Algerian TikTok content carries `voice_to_text`? Decides whether TikTok is a
      caption source or a genuine speech corpus.
- [ ] Is path 2 alone sufficient for creator monitoring, reserving path 3 for hashtag backfill? That
      would make TikTok almost signature-independent.
- [ ] Is `datacenter` really sufficient long-term, or will TikTok tighten IP reputation checks?
- [ ] Should TikTok results be visually distinct in the UI given their different content shape?
      ([[UI - Results Page]])

## Related

[[ADR-0009 - Direct Collection for Social Platforms]] · [[Signature Service]] · [[Session Manager]] ·
[[Proxy Manager]] · [[Image Pipeline]] · [[Ranking and Relevance]] · [[Query Expander]] ·
[[Social Connector - Facebook]] · [[Social Connector - Instagram]] · [[Legal and Compliance]]
