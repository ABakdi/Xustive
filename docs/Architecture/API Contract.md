---
tags:
  - architecture
  - api
type: contract
status: specified
updated: 2026-08-06
---

# API Contract

> The complete HTTP surface of `xustive-api`. Owned by [[API Gateway]]; shapes come from [[Data Model]].
> Consumed by [[UI Specification]].

Base URL: `https://xustive.dz/api/v1` · Content type: `application/json; charset=utf-8` ·
All timestamps are **unix seconds, UTC**.

---

## 1. Conventions

- **Versioning** — path-versioned (`/api/v1`). Breaking changes mint `/api/v2`; both run in parallel
  for one release cycle.
- **Request id** — clients may send `X-Request-Id`; otherwise the server generates a ULID. Always
  echoed in the response header. It is *not* correlated with the user or the query text.
- **Pagination** — `page` (1-based) + `hits_per_page` (default 20, max 50).
- **Errors** — every non-2xx returns the [[#8. Error Object]].
- **No auth** for search endpoints. Write endpoints require `X-Api-Key`.

---

## 2. `GET /search`

The main endpoint. Returns results **without** the AI summary.

### Query parameters

| Param | Type | Default | Notes |
|:---|:---|:---|:---|
| `q` | string (1…512) | — | **required**; the raw user query |
| `page` | int ≥ 1 | 1 | |
| `hits_per_page` | int 1…50 | 20 | |
| `lang` | `ar\|ary\|fr\|en\|auto` | `auto` | overrides [[Language Detector]] |
| `source` | csv of `web,facebook,instagram,tiktok` | all | facet filter |
| `sentiment` | csv of `positive,neutral,negative` | all | facet filter |
| `from` | unix seconds | — | `published_at >= from` |
| `to` | unix seconds | — | `published_at <= to` |
| `sort` | `relevance\|recency` | `relevance` | |
| `expand` | bool | `true` | disable [[Query Expander]] for debugging |
| `include_comments` | bool | `true` | fold comment matches into cards |

### 200 Response

```jsonc
{
  "query": { "raw": "…", "normalized": "…", "language": "ary", "language_confidence": 0.86,
             "expanded_terms": ["…"], "corrected": null },
  "summary_token": "01J8ZK…",        // pass to /search/summary (SSE). null if summary unavailable
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
      "sentiment": { "label": "negative", "score": -0.42 },
      "engagement": { "likes": 340, "comments": 88, "shares": 12, "views": 0 },
      "language": "ary",
      "thumbnail_url": "https://…",
      "matched_comments": [
        { "id": "…", "excerpt": "…", "sentiment": { "label": "positive", "score": 0.5 },
          "published_at": 1754439000 }
      ],
      "score": 0.8123
    }
  ],
  "facets": {
    "source_type": { "web": 900, "facebook": 700, "instagram": 150, "tiktok": 84 },
    "sentiment":   { "positive": 401, "neutral": 1100, "negative": 333 },
    "date_histogram": [ { "bucket": 1754352000, "count": 120 } ]
  }
}
```

Highlighting uses `<em>…</em>` only. The client must escape all other HTML — see
[[UI - Results Page]].

---

## 3. `GET /search/summary` (SSE)

```
GET /api/v1/search/summary?token=01J8ZK…
Accept: text/event-stream
```

The `token` is single-use and expires after 60 s. Events:

| Event | Data | Meaning |
|:---|:---|:---|
| `status` | `{"state":"queued"\|"generating"}` | shown as a shimmer in the UI |
| `delta` | `{"text":"…"}` | append token(s) |
| `done` | `{"citations":[{"result_id":"…","n":1}],"took_ms":1840}` | finished |
| `error` | `{"code":"summary_unavailable"}` | client hides the summary block silently |

Rationale in [[ADR-0004 - Stream Summary Separately from Results]]. Behaviour and skeleton states in
[[UI - Results Page]].

---

## 4. `GET /suggest`

| Param | Type | Notes |
|:---|:---|:---|
| `q` | string 1…64 | prefix |
| `limit` | int 1…10, default 8 | |

```jsonc
{ "suggestions": [
    { "text": "سونلغاز فاتورة", "type": "query", "score": 0.91 },
    { "text": "sonelgaz facture", "type": "transliteration", "score": 0.77 },
    { "text": "Sonelgaz",       "type": "entity", "score": 0.70 }
] }
```

Budget: **p95 ≤ 40 ms**. Served by [[Autocomplete Service]].

---

## 5. `POST /search/voice`

`multipart/form-data`

| Part | Type | Limit |
|:---|:---|:---|
| `audio` | `audio/webm;codecs=opus`, `audio/wav`, `audio/ogg` | 10 MB / 30 s |
| `hint_lang` | text, optional | `ar\|ary\|fr\|en` |

```jsonc
{ "transcript": "وين نلقى خدمة في وهران",
  "language": "ary", "confidence": 0.82, "duration_ms": 4200, "took_ms": 900 }
```

The server **transcribes only** — it does not search. The client puts the transcript in the search
box and issues a normal `/search`. Audio is processed in memory and never written to disk. See
[[Speech to Text]] and [[UI - Voice Search]].

---

## 6. `POST /search/image`

`multipart/form-data`

| Part | Type | Limit |
|:---|:---|:---|
| `image` | `image/jpeg\|png\|webp` | 8 MB, ≤ 4096 px |
| `mode` | `auto\|ocr\|similar` | default `auto` |

```jsonc
{
  "mode_used": "ocr",
  "ocr": { "text": "…", "language": "ar", "confidence": 0.79 },
  "similar": [ { "document_id": "…", "media_url": "…", "score": 0.88, "title": "…",
                 "source_type": "instagram", "published_at": 1754438400 } ],
  "took_ms": 380
}
```

`auto` runs OCR first; if OCR yields ≥ 8 usable characters the client is told to search that text,
otherwise vector similarity results are returned. See [[Image Pipeline]] and [[UI - Image Search]].

---

## 7. Operational and write endpoints

| Method | Path | Auth | Purpose |
|:---|:---|:---|:---|
| `GET` | `/healthz` | none | liveness, always 200 if the process is up |
| `GET` | `/readyz` | none | 200 only if Meilisearch + Redis reachable |
| `GET` | `/metrics` | internal net | Prometheus exposition ([[Observability]]) |
| `POST` | `/sources` | none + captcha | public "submit a source" ([[Admin and Source Submission]]) |
| `GET` | `/admin/sources` | `X-Api-Key` | list/inspect registry |
| `PATCH` | `/admin/sources/{id}` | `X-Api-Key` | enable/disable, change policy |
| `POST` | `/admin/recrawl` | `X-Api-Key` | force a source into the frontier |
| `POST` | `/admin/takedown` | `X-Api-Key` | remove a document + block its URL ([[Legal and Compliance]]) |
| `GET` | `/admin` | `X-Admin-Key` | operator page: compute device, model inventory |
| `GET` | `/admin/status` | `X-Admin-Key` | device resolution, GPU detection, model inventory, ranking weights |
| `POST` | `/admin/device` | `X-Admin-Key` | change device preference and GPU layer count |

### Admin authentication

The admin surface has its own key, separate from the `X-Api-Key` used by source-registry
endpoints, because it changes how the process runs rather than what it holds.

Two modes, and the default is the restrictive one:

- **`XUSTIVE_ADMIN_KEY` set** — callers must present it in `X-Admin-Key`. Compared in constant
  time. This is how any deployment reachable from a network must run.
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

## 8. Error Object

```jsonc
{ "error": { "code": "query_too_long", "message": "q must be ≤ 512 characters",
             "request_id": "01J8ZK…", "retry_after": null } }
```

| HTTP | `code` | When |
|:---|:---|:---|
| 400 | `invalid_query`, `query_too_long`, `invalid_filter`, `unsupported_media_type`, `invalid_device`, `invalid_gpu_layers` | validation |
| 413 | `payload_too_large` | upload over limit |
| 415 | `unsupported_media_type` | bad codec/format |
| 422 | `no_speech_detected`, `image_unreadable` | media understood but unusable |
| 403 | `admin_key_required`, `admin_local_only` | admin surface, wrong or missing credentials |
| 429 | `rate_limited` | + `Retry-After` header ([[API Gateway]]) |
| 499 | — | client disconnect (logged, not returned) |
| 500 | `internal_error` | unexpected; never leaks internals |
| 503 | `search_unavailable`, `summary_unavailable` | dependency down / shed load |
| 504 | `upstream_timeout` | index or model exceeded its budget |

**Rule:** `message` is safe to display to end users and is localised by the client, not the server —
the client keys off `code`. See [[UI - States and Errors]].

---

## 9. Rate Limits (default)

| Endpoint | Limit | Window | Key |
|:---|:---|:---|:---|
| `/search` | 60 | 1 min | IP /24 |
| `/suggest` | 300 | 1 min | IP /24 |
| `/search/voice` | 10 | 1 min | IP /24 |
| `/search/image` | 10 | 1 min | IP /24 |
| `/sources` | 5 | 1 hour | IP /24 |

Buckets are keyed on a **truncated, salted** IP hash with a rotating daily salt so the limiter cannot
be used to reconstruct browsing history ([[Security and Privacy]]).

---

## 10. Open Questions

- [ ] Should `/search` support `POST` for very long queries (image OCR output can exceed 512 chars)?
- [ ] Do we expose `score` publicly? It invites gaming; useful for debugging.
- [ ] Cursor pagination instead of `page` for deep result sets?

## Related

[[API Gateway]] · [[Query Pipeline]] · [[UI Specification]] · [[Data Model]] · [[Performance Budgets]]
