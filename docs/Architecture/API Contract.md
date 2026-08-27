---
tags:
  - architecture
  - api
type: contract
status: built
updated: 2026-08-27
---

# API Contract

> The complete HTTP surface of `xustive-api`, plus the handful of routes the web tier serves
> itself. Owned by [[API Gateway]]; shapes come from [[Data Model]]. Consumed by
> [[UI Specification]].

Base URL: `https://xustive.dz/api/v1` (dev: `http://0.0.0.0:8080/api/v1`, `api.bind_addr`) ·
Content type: `application/json; charset=utf-8` · All timestamps are **unix seconds, UTC**.

Audited against `crates/xustive-api/src/lib.rs` (the router) on 2026-08-27. Where the built
surface differs from the 2026-08-06 design, the design is kept below with a note saying what
replaced it and why.

---

## 1. Conventions

- **Versioning** — path-versioned (`/api/v1`). Breaking changes mint `/api/v2`; both run in parallel
  for one release cycle. (Only `v1` exists.)
- **Request id** — the server generates a ULID and echoes it in `X-Request-Id`. A client-supplied
  `X-Request-Id` is **stripped** before the id layer runs: tower-http would otherwise keep it, and
  a public API must not let a caller choose the id its log lines are grouped under. It is *not*
  correlated with the user or the query text.
- **Pagination** — `page` (1-based) + `hits_per_page` (default 20, max 50; `search.*` config).
- **Errors** — every non-2xx returns the [[#9. Error Object]], with three exceptions noted there.
- **No auth** for public endpoints. The admin surface needs `X-Admin-Key` ([[#8. Admin surface]]).
  The `X-Api-Key` source-registry key from the original design was never built: source
  submission moved into the admin console, so there is one operator credential, not two.
- **Rate limits** are per route group, keyed on a salted hash of the client's /24 ([[#10. Rate Limits]]). A `429` carries `Retry-After`; every limited response carries
  `x-ratelimit-remaining`.
- **Load shedding** — `api.max_concurrent` (dev 64, prod 512) requests in flight; the next one is
  refused immediately with `503 overloaded` and `Retry-After: 1` rather than queued
  ([[Error Handling and Resilience]]).
- **Body limit** — `api.body_limit_default` (8 KB) on JSON routes; the media and knowledge routes
  raise it per route (see each). Over the limit is a bare `413` from the body-limit layer, not the
  error object.
- **Security headers** on every response, JSON included: the CSP, `Referrer-Policy: no-referrer`,
  `X-Content-Type-Options: nosniff`, `Permissions-Policy`, `Cross-Origin-Opener-Policy`
  ([[Security and Privacy]] §3).

---

## 2. `GET /search`

The main endpoint. Returns results **without** the AI summary.

### Query parameters

| Param | Type | Default | Notes |
|:---|:---|:---|:---|
| `q` | string (1…512 chars) | — | **required**; the raw user query |
| `page` | int ≥ 1 | 1 | |
| `hits_per_page` | int 1…50 | 20 | |
| `lang` | `ar\|ary\|fr\|en\|auto` | `auto` | filters results; overrides [[Language Detector]] |
| `source` | csv of `web,facebook,instagram,tiktok` | all | facet filter |
| `sentiment` | csv of `positive,neutral,negative` | all | facet filter |
| `from` | unix seconds | — | `published_at >= from` |
| `to` | unix seconds | — | `published_at <= to` |
| `sort` | `relevance\|recency` | `relevance` | |
| `v` | `all\|news\|images\|videos` | `all` | search vertical — a saved filter over the one index ([[UI - Search Verticals]]) |
| `ui` | `ar\|ary\|fr\|en` | — | interface language, for instant-answer labels; distinct from `lang` |

The `expand` and `include_comments` debugging switches from the 2026-08-06 design were not built.
Expansion is decided per query by the pipeline (a second expanded leg runs only when the primary
leg is thin), and comment folding does not exist yet — see `matched_comments` below.

Verticals: `news` is web documents whose `published_at_precision` is not `unknown`; `images` and
`videos` filter on `media.type` and, when federation is on, ask SearXNG's matching category
([[Federation Gateway]], M9). An unknown `v` falls back to `all`.

### 200 Response

```jsonc
{
  "query": { "raw": "…", "normalized": "…", "language": "ary", "language_confidence": 0.86,
             "expanded_terms": ["…"], "corrected": null },
  "summary_token": "01J8ZK…",        // pass to POST /summary. null when there is nothing to
                                      // summarise, summaries are off, or page > 1
  "interaction_token": "01J8ZK…",    // opaque; returned by the click beacon (§7). Absent when
                                      // interaction signals are off. Never logged.
  "is_question": false,               // where the summary goes (above vs beside results), not
                                      // whether to fetch it
  "instant": { "tool": "calculator", "confidence": 0.98, "interpretation": "2 + 2",
               "value": "4", "detail": null, "as_of": null },   // absent almost always
  "pagination": { "page": 1, "hits_per_page": 20, "total_hits": 1834, "total_pages": 92,
                  "estimated": true },
  "took_ms": 63,
  "results": [
    {
      "id": "01J8ZK4Q…",
      "title": "…",
      "url": "https://…",
      "display_url": "elkhabar.com › economie",
      "excerpt": "…",                       // with <em> highlight markers
      "source_type": "facebook",
      "source_name": "Groupe Emploi Alger",
      "author": { "name": "…", "handle": "…", "url": "…" },
      "published_at": 1754438400,
      "published_at_precision": "day",
      "sentiment": { "label": "negative", "score": -0.42 },   // null when not scored
      "engagement": { "likes": 340, "comments": 88, "shares": 12, "views": 0 },
      "language": "ary",
      "thumbnail_url": "https://…",
      "matched_comments": [],               // always empty today — see below
      "score": 0.8123,
      "similar_count": 3,                   // near-duplicates folded in ("+N similar"); omitted at 0
      "from_web": true,                     // live federation hit not yet in the index; omitted
                                            // when false (M7)
      "media": [                            // Images/Videos tabs (M9); omitted when empty
        { "kind": "image", "url": "https://…", "thumb_url": "https://…",
          "provider": "youtube", "width": 1280, "height": 720 }
      ]
    }
  ],
  "facets": {
    "source_type": { "web": 900, "facebook": 700, "instagram": 150, "tiktok": 84 },
    "sentiment":   { "positive": 401, "neutral": 1100, "negative": 333 },
    "date_histogram": [ { "bucket": 1754352000, "count": 120 } ]
  },
  "facets_degraded": true,            // facets dropped under time pressure, not genuinely empty;
                                      // omitted when false
  "related": ["…"]                    // related searches (M7-T03); omitted when empty or page > 1
}
```

`instant` is a `xustive_tools::Answer` ([[Instant Answers]]); the client picks a renderer from
`tool`. Media `url`/`thumb_url` are upstream URLs — the web tier signs and proxies them through
`/api/thumb` before they reach a browser ([[ADR-0021 - Proxied Thumbnails with Signed URLs]]).

**`matched_comments`** is in the shape and always empty. The `comments` index exists and has
settings ([[Data Model]] §3) but the query pipeline runs no multi-search against it — the social
connectors that would fill it are not built. The field stays so the card shape does not change
when they are.

Highlighting uses `<em>…</em>` only. The client must escape all other HTML — see
[[UI - Results Page]].

---

## 3. `POST /summary`

```
POST /api/v1/summary
{ "token": "01J8ZK…" }
```

The `token` from the search response. Single-use, expires after **120 s**, at most 4096
outstanding (a flood of searches cannot grow the map without bound). Consuming it re-runs nothing:
one summary per search.

**Always 200**, even with no summary — a missing summary is a normal outcome, not an error the
reader can act on ([[Error Handling and Resilience]] §6):

```jsonc
{ "summary": "…",                     // null when there is none
  "citations": [ { "result_id": "…", "n": 1 } ],   // omitted when empty
  "reason": "unknown_token",          // diagnostic, omitted when there is a summary
  "took_ms": 1840 }
```

An unknown token gets the same `200` with `reason: "unknown_token"` — probing for valid tokens
learns nothing from the response.

**Superseded design (2026-08-06 → M1):** `GET /search/summary` as an SSE stream with
`status`/`delta`/`done`/`error` events. The separation of summary from results that
[[ADR-0004 - Stream Summary Separately from Results]] argues for is kept — the page paints, then
asks — but the summary is short enough (≤ 400 chars, [[Summarizer]]) that a single JSON reply
arrives about when the first streamed token would, with none of the stream's reconnect and
buffering edge cases. Streaming is used where the output is long: `/translate`.

---

## 4. `GET /suggest`

| Param | Type | Notes |
|:---|:---|:---|
| `q` | string, ≥ 2 chars | prefix; shorter returns an empty list |
| `limit` | int 1…20, default 8 | |

```jsonc
{ "suggestions": [
    { "text": "سونلغاز فاتورة", "source": "index" },
    { "text": "sonelgaz facture", "source": "transliteration" },
    { "text": "Sonelgaz",       "source": "curated" }
], "took_ms": 3 }
```

`source` is one of `index` (the prefix index built from the corpus), `curated`
(`suggest.curated_path`), `title` (a 60 ms document-title leg, run only when the prefix index is
thin) and `transliteration` (Arabizi variants). Scores are internal weights and are not exposed.
Budget: **p95 ≤ 40 ms**. Served by [[Autocomplete Service]].

### `GET /tools` and `GET /languages`

Two small discovery endpoints on the suggest limiter.

- `/tools` → `{ "tools": [ { "id": "calculator", "keyword": "calc" }, … ] }` — every instant-answer
  tool, its `Answer.tool` id (what a per-tool opt-out in Settings keys on) and its explicit
  `!keyword`. See [[Instant Answers]].
- `/languages` → `{ "languages": [ { "code": "ar", "name_ar": "…", "name_fr": "…", "name_en": "…",
  "approximate": false }, … ] }` — the translator's target languages: `ar ary fr en es de it tr`.
  `approximate` marks a language the local model handles weakly (Darija).

---

## 5. `POST /translate` (SSE)

```
POST /api/v1/translate
{ "text": "…", "from": "fr", "to": "ar" }      // from is optional (detected)
Accept: text/event-stream
```

Streams frames as `data:` lines, keep-alive every 15 s:

| Frame | Data | Meaning |
|:---|:---|:---|
| `delta` | `{"type":"delta","text":"…"}` | append |
| `done` | `{"type":"done","truncated":false,"took_ms":900}` | terminal; `truncated` means a limit cut it |
| `error` | `{"type":"error","reason":"…"}` | terminal |

Rejected before streaming with `400 untranslatable` or `400 unknown_language` — the body never
echoes the text, which is the most sensitive field this service handles. `503 model_unavailable`
when the local model is not loaded or every slot is busy. Served by [[Summarizer]]'s model
([[Instant Answers]] translation card).

---

## 6. Media: `POST /transcribe`, `POST /ocr`, `POST /search/image`

All three take the **raw bytes as the body** — not multipart. Exactly one file is sent and a
form wrapper would only add a parser and a place for a second file to hide. All three are on the
media limiter (10/min) and never write the upload to disk (P4, [[Security and Privacy]]).

### `POST /transcribe?lang=ar&partial=1`

| Param | Notes |
|:---|:---|
| body | audio, ≤ `stt.max_audio_bytes` (8 MB) |
| `lang` | optional hint from a whitelist (`ar ary fr en`); anything else is ignored, so a stray value cannot reach the model |
| `partial=1` | a reading of the words so far while the person is still speaking — the sidecar answers with its fast model and a greedy decode; the final call without it gets the careful model |

```jsonc
{ "text": "وين نلقى خدمة في وهران", "language": "ar" }
```

`text` may be empty for silence — the client shows an empty state, not an error. The server
**transcribes only**; the client puts the transcript in the box and issues a normal `/search`.
`503 stt_unavailable` when `stt.enabled` is off or the sidecar is down; `422 empty_audio` /
`audio_too_large`. See [[Speech to Text]] and [[UI - Voice Search]].

**Superseded design:** `POST /search/voice` as multipart with `hint_lang`. Same contract, renamed
when it became a query parameter on a raw-body route.

### `POST /ocr`

Body: an image, ≤ `media.max_image_bytes` (5 MB), ≤ 40 MP decoded.

```jsonc
{ "text": "…", "usable": true, "confidence": 0.79, "backend": "tesseract" }
```

`usable` is the 8-character rule: below it the client should not search the text.
`backend` is `tesseract` or `sidecar` ([[ADR-0016 - Two OCR Engines with an Optional Unlimited-OCR Sidecar]]).
Errors: `422 empty_image` / `image_too_large` / `undecodable_image`; `503 ocr_unavailable`.

### `POST /search/image`

Body: an image, same caps as `/ocr`. Runs CLIP similarity against the `image_clip` Qdrant
collection ([[Vector Index]]) and resolves the matching points back to documents.

```jsonc
{ "results": [ /* ResultCard[] as in /search */ ], "matched_images": 12 }
```

`matched_images` counts points before collapsing by document, so the UI can say "12 similar
images across 5 pages" honestly. `503 image_search_unavailable` when `vector.enabled` is off.

**Superseded design:** one endpoint with `mode=auto|ocr|similar` that ran OCR first. The client
now decides — the OCR tool page calls `/ocr`, the image-search affordance calls `/search/image` —
because "auto" hid which of two very different things happened. See [[Image Pipeline]] and
[[UI - Image Search]].

---

## 7. Knowledge and interaction

### `GET /knowledge?q=&lang=&kind=`

The entity panel ([[ADR-0019 - The Knowledge Layer]]). `200` with the rendered panel, or **`204`**
when the query is not entity-shaped or nothing resolves — "no panel" is a different thing from
"an empty panel" for the client and every cache in between. `kind=film,series` restricts what the
entity may be (the relation row asks for the film's own panel this way).

```jsonc
{ "id": "Q83495", "kind": "film", "title": "The Matrix", "description": "1999 film",
  "lang": "en", "facts": [ { "key": "director", "…": "…" } ],
  "images": [ { "url": "…", "licence": "…", "source": "…" } ],
  "authorities": [ { "key": "imdb", "id": "tt0133093", "url": "…" } ],
  "extract": { "text": "…", "url": "…" }, "updated_at": 1756200000, "generated": "…",
  "confidence": 0.9, "also": "Q…" }
```

Facts are rendered from the per-kind template in `xustive-knowledge` (keys such as `director`,
`cast`, `birth_date`, `population`, `taxon_name`).

### `POST /knowledge/render` and `POST /knowledge/resolve-live`

The live fallback ([[ADR-0014 - Knowledge Panel from Wikipedia via the Web Tier]] extended by M8):
when the store has no entity, the **web tier** — the one place with egress — fetches raw Wikidata
documents and hands them here, so the panel is built by the same parser and templates a harvested
entity gets. No second, weaker parser in TypeScript.

- `render` — `{ "doc": <wbgetentities entity>, "lang": "fr", "labels": Q30","United States,
  "extract": { "lang", "text", "url" } }` → the panel as above plus `"live": true` and
  `"unresolved": ["Q…"]` (ids whose labels the caller did not supply).
- `resolve-live` — `{ "query": "zidane", "docs": [ … ], "prefer_kinds": ["person"] }` →
  `{ "id", "kind", "confidence", "also" }`, or `204` when none is a credible match.

Both are on the knowledge limiter (90/min) with a 32 MB body limit — a Wikidata entity for a
country is large. They are internal to the web tier in practice but carry no secret, so they are
not authenticated: rendering a document the caller already holds reveals nothing.

### `POST /interaction`

```jsonc
{ "t": "<interaction_token>", "d": "<document id>" }
```

The click beacon ([[ADR-0015 - Anonymous Interaction Signals for Ranking]]). **Always `204`** —
malformed, missing, expired or unknown tokens are not distinguished from a recorded click, so the
endpoint leaks nothing about what it accepted. Carries the token and a document id, never the
query text. Suggest limiter.

---

## 8. Operational and admin endpoints

| Method | Path | Auth | Purpose |
|:---|:---|:---|:---|
| `GET` | `/healthz` | none | liveness, always 200 if the process is up |
| `GET` | `/readyz` | none | 200 only if Meilisearch answers its health check; Redis is not consulted (search works without it) |
| `GET` | `/metrics` | internal net | Prometheus exposition ([[Observability]]) |

These three are mounted at the root (`/healthz`, not `/api/v1/healthz`).

### Admin surface — `/api/v1/admin/*`, `X-Admin-Key`

Everything the operator console ([[UI - Admin Console]], [[Crawler Console]]) calls. Read
endpoints are `GET`, actions are `POST`; every action is logged at `info`/`warn` with the peer
address, which is the one place logging is deliberate and contains no user data.

| Method | Path | Purpose |
|:---|:---|:---|
| `GET` | `/status` | device resolution, GPU detection, model inventory, ranking weights |
| `GET` | `/config` | the effective config, every secret `REDACTED` (the marker distinguishes "set" from "empty") |
| `GET` | `/media` | OCR / CLIP / STT / text-embed sidecar health — probes them, so slower than `/status` |
| `GET` | `/interaction` | k-anonymous interaction analytics: top queries by category, CTR leaders, hot re-crawl targets |
| `GET` | `/integrations` | federation and external-tool switches; **no live probe** of SearXNG (it is behind the no-egress boundary) |
| `POST` | `/integrations` | flip a runtime switch; enabling federation without an endpoint is refused with a reason |
| `GET` | `/eval` | every report under `eval/reports/`, the gate verdict, synonym-candidate sheets awaiting review |
| `GET` | `/queue` | backlog depth, dead-letter count, a peek at the dead letters ([[Task Queue]]) |
| `POST` | `/queue/replay` | re-enqueue every dead letter |
| `POST` | `/queue/dead/replay` | re-enqueue one dead letter by entry id |
| `POST` | `/queue/dead/drop` | discard one dead letter — the only deliberate discard in the queue; the UI confirms it |
| `GET` | `/crawler/status` | frontier, hosts, counters |
| `GET` | `/crawler/events` | one SSE connection carrying every live number on the console page |
| `GET` | `/crawler/documents` | paged document list backed by the product index, with provenance badges |
| `GET` | `/crawler/channels` | the funnel per discovery channel: discovered → fetched → indexed → survived dedup |
| `GET` | `/crawler/sources` | the registry ([[Data Sources Registry]]) |
| `POST` | `/crawler/sources` | add a source; the URL still passes `SafeUrl` and the trap detectors — the console changes ordering, never permission |
| `POST` | `/crawler/sources/remove` | remove the seed line; does not delete what was crawled |
| `GET` | `/crawler/sources/health` | per-source quality metrics |
| `POST` | `/crawler/enqueue` | push a URL into the frontier now |
| `POST` | `/crawler/registry` | registry mutation (approve / lifecycle) |
| `POST` | `/crawler/pause` | pause/resume the crawler (`crawl:paused` in Redis) |
| `GET` | `/crawler/weak-coverage` | k-anonymous under-served terms; reports "disabled" distinctly from "no gaps" |
| `POST` | `/crawler/weak-coverage/forget` | dismiss a term (a real gap re-accumulates on its own) |
| `POST` | `/politeness` | flip `crawl.ignore_politeness`; logged at `warn`, and prod refuses to start with it on |
| `POST` | `/takedown` | preview or execute a domain takedown: index, embeddings and blocklist in one auditable action ([[Legal and Compliance]]) |
| `POST` | `/device` | change device preference and GPU layer count |
| `POST` | `/log-level` | temporary log-filter override; **expires on its own after 15 minutes** |

**Not built from the 2026-08-06 design:** the public `POST /sources` "submit a source" route with
captcha, `PATCH /admin/sources/{id}` and `POST /admin/recrawl`. Submission goes through the
operator console (`POST /crawler/sources`), and re-crawl is `POST /crawler/enqueue`.

### Admin authentication

The admin surface changes how the process runs rather than what it holds, so it has its own key.
Two modes, and the default is the restrictive one:

- **`api.admin_key` / `XUSTIVE_ADMIN_KEY` set** — callers must present it in `X-Admin-Key`.
  Compared in constant time. This is how any deployment reachable from a network must run.
- **unset** — only loopback callers are admitted, and everyone else gets `403 admin_local_only`.
  A peer address the server cannot determine counts as remote. This keeps a local `make web`
  usable in a browser without setup, without silently exposing device settings on a box whose
  default bind address is `0.0.0.0`.

### `POST /admin/device`

```jsonc
{ "preference": "auto" | "gpu" | "cpu",   // optional
  "gpu_layers": 0 | 24 | null }           // optional; null = decide from free memory
```

Changes take effect on the **next model load**, not immediately: tearing down a model mid-request
would fail whatever generation is in flight and buys nothing. The response echoes the resolved
state, which is not always what was asked for — a GPU request on a CPU-only build still lands on
CPU, with `fell_back: true` and a `reason` string saying why. See [[Summarizer]] and
[[Deployment Topology]].

---

## 9. Error Object

```jsonc
{ "error": { "code": "query_too_long", "message": "Search is limited to 512 characters." } }
```

| HTTP | `code` | When |
|:---|:---|:---|
| 400 | `invalid_query`, `query_too_long`, `invalid_filter` | validation |
| 400 | `untranslatable`, `unknown_language` | `/translate` input; the text is never echoed |
| 403 | `admin_key_required`, `admin_local_only` | admin surface, wrong or missing credentials |
| 413 | — | body over the route's limit; bare status from the body-limit layer |
| 422 | `empty_audio`, `audio_too_large`, `empty_image`, `image_too_large`, `undecodable_image` | media understood but unusable |
| 429 | `rate_limited` | + `Retry-After` header ([[#10. Rate Limits]]) |
| 499 | — | client disconnect (logged, not returned) |
| 500 | `internal_error` | unexpected; never leaks internals |
| 503 | `search_unavailable` | Meilisearch down or unreachable |
| 503 | `model_unavailable`, `stt_unavailable`, `ocr_unavailable`, `image_search_unavailable` | a model or sidecar is off, not loaded, or every slot is busy |
| 503 | `overloaded` | load shed at `api.max_concurrent`; `Retry-After: 1` |
| 504 | `upstream_timeout` | index or model exceeded its budget |

`request_id` and `retry_after` are **not** in the body (the 2026-08-06 shape had both): the id is
in the `X-Request-Id` header and the delay in `Retry-After`, and duplicating them in JSON gave the
client two places to disagree.

**Rule:** `message` is safe to display and never restates the caller's input; the client
localises off `code`. See [[UI - States and Errors]].

---

## 10. Rate Limits (default)

Constants in `ratelimit.rs`; not configurable.

| Group | Routes | Limit | Window |
|:---|:---|:---|:---|
| search | `/search` | 60 | 1 min |
| suggest | `/suggest`, `/tools`, `/languages`, `/interaction` | 300 | 1 min |
| summary | `/summary` | 20 | 1 min |
| translate | `/translate` | 10 | 1 min |
| media | `/transcribe`, `/ocr`, `/search/image` | 10 | 1 min |
| knowledge | `/knowledge`, `/knowledge/render`, `/knowledge/resolve-live` | 90 | 1 min |
| sources | *(defined, not wired — there is no public `/sources` route)* | 5 | 1 hour |

Suggest is five times search because it fires per keystroke; a limit a normal typist trips only
affects real users. Translate and summary are low because the model has only as many slots as
the card has memory for.

Buckets are keyed on `blake3_keyed(salt, ip/24)` — /48 for IPv6 — with a salt generated at boot
and rotated every 24 h, in memory, at most 200 000 buckets. Possessing the table does not let
anyone test whether a given IP is in it, and a client cannot be recognised across a rotation,
which is the intended trade ([[Security and Privacy]] P5). `X-Forwarded-For` is **not**
consulted; behind a proxy every caller shares one bucket, which is deliberately the strict
failure — a limiter that stops limiting when it cannot identify anyone is not a limiter. A peer
address the server cannot determine also lands in that shared bucket.

---

## 11. Web-tier routes (`/api/*`, Next.js)

The Next.js server ([[UI - Frontend Architecture]]) serves a few routes of its own on the site
origin — not under `/api/v1`, and not part of `xustive-api`. They exist because the web tier is
the one place with egress ([[ADR-0001 - Two-Plane Architecture]], [[ADR-0014 - Knowledge Panel from Wikipedia via the Web Tier]]),
and because the browser must never talk to a crawled host directly.

| Route | What it does |
|:---|:---|
| `GET /api/knowledge?q=` | Wikipedia summary panel (ADR-0014); `204` for anything not entity-shaped. Superseded for most queries by the Rust `/knowledge` panel (M8), kept as the extract source |
| `GET /api/knowledge-live?q=` | live Wikidata lookup when the store has no entity: search, drop disambiguation/name pages, rank by sitelink prominence, then `POST /knowledge/render` |
| `GET /api/knowledge-list?q=` | list answers (M8-T11): the cast of a film, the books of an author — one SPARQL query, cards linking to authorities by identifier, no scraping |
| `GET /api/wiki-image?u=` | image proxy for the panel; host allowlist `upload.wikimedia.org`, `commons.wikimedia.org`; 5 MB, 6 s |
| `GET /api/thumb?u=&s=` | signed thumbnail proxy ([[ADR-0021 - Proxied Thumbnails with Signed URLs]]): HMAC-SHA256 over the upstream URL with `XUSTIVE_THUMB_SECRET` (random per process when unset); https only, no IP literals or private names; 5 MB, 4 s, 4 redirects; a transparent pixel on upstream failure. Wikimedia and `covers.openlibrary.org` are served unsigned because they are public by construction |

These are rate-limited only by the API calls they make downstream. Anything they fetch from the
internet carries the `XustiveKnowledge/0.1` or `XustiveThumb/0.1` user agent and never the
reader's address or referrer.

---

## 12. Open Questions

- [ ] Should `/search` support `POST` for very long queries (image OCR output can exceed 512 chars)?
- [ ] Do we expose `score` publicly? It invites gaming; useful for debugging.
- [ ] Cursor pagination instead of `page` for deep result sets?
- [ ] Should `/knowledge/render` and `/resolve-live` be restricted to the web tier's address once
      there is a proxy in front, or is "reveals nothing" enough?

## Related

[[API Gateway]] · [[Query Pipeline]] · [[UI Specification]] · [[Data Model]] · [[Performance Budgets]]
