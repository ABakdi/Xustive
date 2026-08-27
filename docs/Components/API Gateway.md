---
tags:
  - component
  - serving
component-id: C01
binary: xustive-api
status: built
updated: 2026-08-27
---

# API Gateway

> **ID** C01 · **Binary** `xustive-api` (`crates/xustive-api`) · **Upstream** the Next.js server in
> `web/` (which proxies `/api/v1/*`) · **Downstream** [[Query Pipeline]], [[Autocomplete Service]],
> [[Speech to Text]], [[Image Pipeline]], [[Summarizer]], [[Instant Answers]], [[Knowledge Store]],
> [[Admin and Source Submission]]

## 1. Purpose

The single HTTP surface of Xustive. It owns transport concerns — routing, validation, limits,
headers, timeouts, error shaping — so that no downstream component ever deals with HTTP. It is also
the **privacy chokepoint**: the raw query exists here and nowhere else in durable form
([[Security and Privacy]] P1).

## 2. Responsibilities

**In scope**: routing per [[API Contract]]; request validation; rate limiting; body size limits;
CORS; security headers; request-id assignment; timeout enforcement; error → HTTP mapping; the
summary-token handoff; `/healthz`, `/readyz`, `/metrics`.

**Out of scope**: TLS termination (Caddy), any search logic (→ [[Query Pipeline]]), ranking, model
inference, business rules, **and HTML**. *Superseded 2026-08-27:* "static file serving for the UI"
and "SSE transport for summaries". This process is JSON only; search *and* admin pages are the
Next.js app in `web/` ([[ADR-0010 - Next.js for the Frontend]]), and the summary is a plain JSON
`POST` rather than a stream (§4). `api.static_dir` survives in the config struct but nothing serves
it.

## 3. Interface

### 3.1 Routes — `crates/xustive-api/src/lib.rs::app`

Routes are grouped so each group gets its own rate limit, timeout and body limit. A single outer
timeout was the first version, and it capped every route at the search budget — which turned every
summary into a 504 the day it was wired up.

| Group | Routes | Rate limit (per client, per 60 s) | Timeout | Body |
|:---|:---|:---|:---|:---|
| search | `GET /api/v1/search` | 60 | `timeout_search_ms` + grace | 8 KB |
| suggest | `GET /suggest` · `GET /tools` · `GET /languages` | 300 | `timeout_suggest_ms` | 8 KB |
| interaction | `POST /interaction` (click beacon, always 204) | 300 (shares suggest) | `timeout_suggest_ms` | 8 KB |
| summary | `POST /summary` | 20 | `ml.deadline_ms` + 10 s | 8 KB |
| translate | `POST /translate` | 10 | **none** — it streams; the engine's own deadline bounds it | 8 KB |
| knowledge | `GET /knowledge` | 90 | `timeout_search_ms` | 8 KB |
| knowledge (large) | `POST /knowledge/render` · `POST /knowledge/resolve-live` | 90 | `timeout_search_ms` | 32 MB (a Wikidata document) |
| media | `POST /ocr` · `POST /search/image` | 10 | `media.sidecar.timeout_ms` + 10 s | `media.max_image_bytes` |
| stt | `POST /transcribe` | 10 (shares media) | `stt.timeout_ms` + 10 s | `stt.max_audio_bytes` |
| admin | `/api/v1/admin/*` (§3.3 of [[Admin and Source Submission]]) | — | search budget | 8 KB |
| ops | `/healthz` · `/readyz` · `/metrics` | — | — | — |

Route names differ from the first draft of this note (`/search/summary`, `/search/voice`,
`POST /sources`): the real names are above. `POST /api/v1/sources` — public submission — does
not exist; its rate-limit bucket (`SOURCES`, 5 per hour) is defined and unused.

The rate-limit constants live in `ratelimit.rs` (`SEARCH`, `SUGGEST`, `SUMMARY`, `TRANSLATE`,
`MEDIA`, `KNOWLEDGE`, `SOURCES`), not in configuration.

### 3.2 Middleware stack, **outermost first** (order is load-bearing)

| # | Layer | Notes |
|:--|:---|:---|
| 1 | `CatchPanicLayer` | a panic returns 500, never kills the worker |
| 2 | `ConcurrencyLimit` + `LoadShed` + `HandleError` | global in-flight cap `api.max_concurrent`; sheds with 503 `overloaded` + `Retry-After: 1` rather than queueing. Drop `LoadShed` and requests hang instead of failing |
| 3 | `strip_client_request_id` | a client-supplied `X-Request-Id` is discarded, so nobody can choose the id we echo back and group our log lines under |
| 4 | `SetRequestIdLayer` (ULID) | sorts by time, so grepping near an incident narrows the window |
| 5 | `CorsLayer` | `cors_origins` empty = same-origin; otherwise the listed origins, `GET` only |
| 6 | `CompressionLayer` | |
| 7 | `security_headers` | CSP `default-src 'self'; img-src 'self' data: https:; media-src 'self' blob:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'`, `Referrer-Policy: no-referrer`, `nosniff`, `Permissions-Policy: microphone=(self), camera=(self), geolocation=()`, COOP `same-origin` |
| 8 | `observe` | `xustive_http_requests_total{route,status}` and `xustive_http_duration_seconds{route}` keyed on the *matched path*, never the query |
| 9 | `PropagateRequestIdLayer` | echoes `X-Request-Id` |
| 10 | per-group: rate limit → `TimeoutLayer` → `RequestBodyLimitLayer` | §3.1 |

The 8 KB default body limit is applied to the `core` router; the large-body groups (`/ocr`,
`/search/image`, `/transcribe`, `/knowledge/render`) are merged *outside* it, because a limit
applied outside a route binds it regardless of any looser inner one.

There is no `TraceLayer`; `observe` and per-handler `tracing` calls do that job, and the
telemetry lint (`scripts/lint-telemetry.sh`) keeps `q` out of them.

## 4. Internal Design

- **State**: `AppState` (`state.rs`) — config, Meilisearch client, the detector and expander,
  the rate limiter, metrics registry, the pending-summary map, optional engines (text/image
  vector search, STT, federation client, interaction store, knowledge index). Handlers are plain
  functions over `State<AppState>`, not trait objects; tests build a state with fakes.
- **Validation**: `serde` query structs; unknown query params are ignored. Errors map to a fixed
  set of codes in `error.rs` — `invalid_query`, `query_too_long`, `invalid_filter`,
  `search_unavailable`, `upstream_timeout`, `internal_error`, plus the translate/media ones — with
  a human message the UI shows verbatim ([[API Contract]] §8).
- **Summary handoff** (`summary.rs`): `/search` registers the top re-ranked passages under a
  fresh ULID token in an in-memory map (TTL **120 s**, cap **4096** entries, one use per token)
  and returns `summary_token`. The browser then `POST`s `/summary {token}` and gets JSON. The
  token, not the query, is what travels — and it binds the summary to a search we actually ran,
  so the endpoint cannot be used as a free text generator ([[ADR-0004 - Stream Summary Separately from Results]] describes the separation; the "stream" half was not built — the answer is small
  enough that one JSON response was simpler).
- **Deadline**: `/search` builds one absolute `Deadline` from `timeout_search_ms` and hands it
  down; the transport timeout sits `SEARCH_GRACE_MS` *above* it so the pipeline's degradation
  ladder can shape and send a narrowed page before the layer cuts the connection (BUG-041).
- **Shed load**: the global cap sheds *any* route with 503 once `max_concurrent` is reached. The
  specified "expensive routes shed first" priority is not implemented; the per-route rate limits
  (10/min for media, 20/min for summary) are what keep them from crowding search.

## 5. Configuration

As built — `[api]` in `config/*.toml` (`ApiConfig`, env overrides in brackets):

| Key | Default | Notes |
|:---|:---|:---|
| `bind_addr` | `0.0.0.0:8080` | [`XUSTIVE_BIND_ADDR`] |
| `max_concurrent` | 512 (dev 64) | global in-flight |
| `body_limit_default` | 8 KiB | |
| `timeout_search_ms` | 1500 (dev 2500) | the search deadline; BUG-041 raised dev |
| `timeout_suggest_ms` | 150 | |
| `cors_origins` | `[]` (same-origin) | dev: `http://localhost:3000` |
| `static_dir` | `web/public` | vestigial, unused |
| `admin_key` | `""` | [`XUSTIVE_ADMIN_KEY`]; empty = loopback-only admin |

Not built as keys: `body_limit_image`/`audio` (they are `media.max_image_bytes` and
`stt.max_audio_bytes`), `timeout_media_ms` (derived from the sidecar timeouts),
`summary_token_ttl_s` (a constant), `rate_limits.*` (constants), `admin_key_hashes` (one plain key).

## 6. Data

Owns no persistent data. Two in-memory maps: rate-limit buckets keyed on `HMAC(boot-salt,
ip/24)` with a per-route window and a size cap (so an attacker spraying source addresses cannot
grow it without bound), and the pending-summary map (120 s, 4096). Neither is a log of who asked
what: the salt is generated at boot and never written down.

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Meilisearch timeout on the candidate query | `SearchError::Timeout` | **not** a 504: the pipeline re-queries narrower (BUG-041); only a second failure is `upstream_timeout` |
| Meilisearch unreachable | `readyz` probe + call error | 503 `search_unavailable`; `readyz` fails |
| Summary engine / sidecars unreachable | connect error | summary omitted or `model_unavailable`; media → the route's own error |
| Handler panic | `CatchPanicLayer` | 500 `internal_error` |
| Body too large | `RequestBodyLimitLayer` | 413 |
| Rate limit exceeded | limiter | 429 `rate_limited` + `Retry-After`, `xustive_rate_limited_total{route}` |
| Too many in flight | `LoadShed` | 503 `overloaded` + `Retry-After: 1` |
| Transport timeout | `TimeoutLayer` | 504 with an empty body — the backstop for a genuinely hung request |

## 8. Performance

| Metric | Budget |
|:---|:---|
| Middleware overhead | ≤ 2 ms p99 |
| `/search` p95 end-to-end | ≤ 200 ms ([[Performance Budgets]]) |
| `/suggest` p95 | ≤ 40 ms |
| Throughput per replica | ≥ 500 rps on 2 vCPU |
| Memory per replica | ≤ 512 MB steady |

`crates/xustive-loadgen` ([[Load Generator]]) is the tool that checks these.

## 9. Observability

Built: `xustive_http_requests_total{route,status}`, `xustive_http_duration_seconds{route}`,
`xustive_rate_limited_total{route}`, `xustive_build_info`, plus the per-feature metrics listed in
`metrics.rs`. Not built under those names: `xustive_sse_active`, `xustive_shed_total`. **Never** log
`q`, `transcript`, or `ocr_text` — `scripts/lint-telemetry.sh` and the startup banner (written
without a literal query string on purpose) enforce this.

## 10. Security

The trust boundary. Everything arriving here is hostile until validated: size caps, magic-byte type
checks (not `Content-Type`) for images and audio, `SafeUrl` for every operator-entered URL, a
constant-time admin-key comparison, and no reflection of user input into error messages beyond a
fixed set of `code`s. See [[Security and Privacy]] §§3, 6, 7. Egress is enforced by
`scripts/test-egress.sh`: the only outbound targets are internal services (Meilisearch, Redis,
Qdrant, the sidecars, the federator).

## 11. Testing

- Unit: rate-limit windows and resets (`ratelimit.rs`), error mapping (`error.rs`), deadline ladder
  (`deadline.rs`), suggest merge rules.
- Integration: `tower::ServiceExt::oneshot` against the router with fakes for the status/header/body
  rows of [[API Contract]] §8.
- Security: `scripts/test-egress.sh`, `scripts/lint-telemetry.sh`, `scripts/scan-logs.sh`.
- Load: `xustive-loadgen`.

## 12. Open Questions

- [ ] The pending-summary map is per process. With more than one API replica behind a round-robin
      balancer the token has to reach the replica that minted it — sticky routing or a shared map.
      Single replica today, so unresolved by construction.
- [ ] Add `POST /search` for long OCR-derived queries?
- [ ] Should the global shed prefer to drop media/summary before search, as first specified?

## Related

[[API Contract]] · [[Query Pipeline]] · [[Security and Privacy]] · [[Observability]] ·
[[Performance Budgets]] · [[UI - Frontend Architecture]] · [[Web Upstream Client]] ·
[[ADR-0010 - Next.js for the Frontend]]
