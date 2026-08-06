---
tags:
  - component
  - serving
component-id: C01
binary: xustive-api
status: specified
updated: 2026-08-06
---

# API Gateway

> **ID** C01 · **Binary** `xustive-api` · **Upstream** browser · **Downstream** [[Query Pipeline]], [[Autocomplete Service]], [[Speech to Text]], [[Image Pipeline]], [[Admin and Source Submission]]

## 1. Purpose

The single HTTP surface of Xustive. It owns transport concerns — routing, validation, limits,
headers, timeouts, error shaping — so that no downstream component ever deals with HTTP. It is also
the **privacy chokepoint**: the raw query exists here and nowhere else in durable form
([[Security and Privacy]] P1).

## 2. Responsibilities

**In scope**: routing per [[API Contract]]; request validation; rate limiting; body size limits;
CORS; security headers; request-id assignment; timeout enforcement; error → HTTP mapping; SSE
transport for summaries; static file serving for the UI; `/healthz`, `/readyz`, `/metrics`.

**Out of scope**: TLS termination (Caddy), any search logic (→ [[Query Pipeline]]), ranking, model
inference, business rules.

## 3. Interface

Axum router:

```rust
Router::new()
  .route("/api/v1/search",         get(search))
  .route("/api/v1/search/summary", get(summary_sse))
  .route("/api/v1/suggest",        get(suggest))
  .route("/api/v1/search/voice",   post(voice))
  .route("/api/v1/search/image",   post(image))
  .route("/api/v1/sources",        post(submit_source))
  .nest("/api/v1/admin",           admin_router().layer(RequireApiKey))
  .route("/healthz", get(health)).route("/readyz", get(ready)).route("/metrics", get(metrics))
  .fallback_service(ServeDir::new("web/dist"))
  .layer(middleware_stack())
```

Middleware stack, **outermost first** (order is load-bearing):

| # | Layer | Notes |
|:--|:---|:---|
| 1 | `CatchPanicLayer` | a panic returns 500, never kills the worker |
| 2 | `SetRequestIdLayer` + `PropagateRequestIdLayer` | ULID; echoed as `X-Request-Id` |
| 3 | `TraceLayer` | span with `route`/`status` only — **never** query text |
| 4 | `SecurityHeadersLayer` | CSP et al. from [[Security and Privacy]] §3 |
| 5 | `CorsLayer` | same-origin in prod; `localhost:*` in dev |
| 6 | `RateLimitLayer` | per-route buckets, §5 |
| 7 | `RequestBodyLimitLayer` | per-route: 8 KB default, 8 MB image, 10 MB audio |
| 8 | `TimeoutLayer` | per-route, §8 |
| 9 | `CompressionLayer` | gzip/br; **disabled on SSE** |
| 10 | `ConcurrencyLimitLayer` | global in-flight cap, sheds with 503 |

Handlers are thin: parse → validate → call a service trait → serialise. Any handler longer than ~40
lines is a design smell.

## 4. Internal Design

- **State**: `Arc<AppState>` with `SearchService`, `SuggestService`, `MlClient`, `Config`,
  `RateLimiter`, `Metrics`. Handlers are generic over traits so tests inject fakes.
- **Validation**: `serde` + a `Validate` impl per query struct. Reject early with `400` and a
  machine-readable `code` ([[API Contract]] §8). Unknown query params are ignored, not errors
  (forward compatibility); unknown *body* fields are rejected.
- **SSE**: `/search/summary` holds a `summary_token` → an in-memory, single-use, 60 s TTL map of
  `token → (candidate_ids, lang)`. The token, not the query, is what travels — the query never
  reappears in a second request. On client disconnect the generation task is aborted immediately
  (`tokio::select!` on the connection close future) so no CPU is burned for a closed tab.
- **Shed load**: when in-flight ≥ `max_concurrent`, expensive routes (`voice`, `image`, `summary`)
  return 503 before cheap ones. Search never sheds until Meilisearch itself is failing.

## 5. Configuration

| Key | Type | Default | Notes |
|:---|:---|:---|:---|
| `bind_addr` | socket | `0.0.0.0:8080` | |
| `max_concurrent` | int | 512 | global in-flight |
| `body_limit_default` | bytes | 8 KiB | |
| `body_limit_image` | bytes | 8 MiB | |
| `body_limit_audio` | bytes | 10 MiB | |
| `timeout_search_ms` | int | 1500 | |
| `timeout_suggest_ms` | int | 150 | |
| `timeout_media_ms` | int | 10000 | |
| `summary_token_ttl_s` | int | 60 | |
| `cors_origins` | list | `[]` (same-origin) | |
| `rate_limits.*` | table | [[API Contract]] §9 | per-route |
| `admin_key_hashes` | list | — | Argon2id |

## 6. Data

Owns no persistent data. Holds two in-memory maps: rate-limit buckets (60 s TTL) and summary tokens
(60 s TTL). Both are bounded (LRU, 100k entries) so a flood cannot exhaust memory.

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Downstream search timeout | `TimeoutLayer` | 504 `upstream_timeout` |
| Meilisearch unreachable | `readyz` probe + call error | 503 `search_unavailable`, `readyz` fails → LB removes replica |
| `xustive-ml` unreachable | connect error | summary omitted (degradation step 1); voice/image → 503 |
| Handler panic | `CatchPanicLayer` | 500 `internal_error`, `ERROR` log, metric |
| Body too large | `RequestBodyLimitLayer` | 413 `payload_too_large` |
| Rate limit exceeded | limiter | 429 + `Retry-After` |
| Client disconnect mid-SSE | connection future | abort generation, 499 in metrics only |
| Malformed multipart | parser | 400 `unsupported_media_type` |

## 8. Performance

| Metric | Budget |
|:---|:---|
| Middleware overhead | ≤ 2 ms p99 |
| `/search` p95 end-to-end | ≤ 200 ms ([[Performance Budgets]]) |
| `/suggest` p95 | ≤ 40 ms |
| Throughput per replica | ≥ 500 rps on 2 vCPU |
| Memory per replica | ≤ 512 MB steady |

## 9. Observability

Metrics `xustive_http_*`, `xustive_ratelimit_rejected_total`, `xustive_sse_active` (gauge),
`xustive_shed_total{route}`. Log events: `startup`, `config_loaded`, `shed`, `panic`,
`admin_action`. Spans per [[Observability]] §4. **Never** log `q`, `transcript`, or `ocr_text` — CI
lint enforces this.

## 10. Security

The trust boundary. Everything arriving here is hostile until validated: size caps, magic-byte type
checks (not `Content-Type`), URL validation delegated to `SafeUrl` for `/sources`, Argon2id-verified
admin keys with constant-time comparison, and no reflection of user input into error messages beyond
a fixed set of `code`s. See [[Security and Privacy]] §§3, 6, 7.

## 11. Testing

- Unit: validation table (valid/invalid for each param), error mapping, rate-limit windows.
- Integration: `axum::Router` + `tower::ServiceExt::oneshot` against fake services — asserts status,
  headers, and body shape for every row of [[API Contract]] §8.
- SSE: disconnect mid-stream asserts the generation task is aborted.
- Security: CSP header snapshot test; egress test (see [[Security and Privacy]] §1); telemetry lint.
- Load: `oha`/`k6` at 500 rps to confirm §8.

## 12. Open Questions

- [ ] Is the in-memory summary-token map acceptable with 3 replicas behind a round-robin LB? (needs
      sticky routing or a shared Redis map — **currently: sticky by `X-Request-Id` hash**)
- [ ] Add `POST /search` for long OCR-derived queries?

## Related

[[API Contract]] · [[Query Pipeline]] · [[Security and Privacy]] · [[Observability]] ·
[[Performance Budgets]] · [[UI Specification]]
