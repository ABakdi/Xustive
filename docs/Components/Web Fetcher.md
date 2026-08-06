---
tags:
  - component
  - ingestion
component-id: C12
binary: xustive-crawler
status: specified
updated: 2026-08-06
---

# Web Fetcher

> **ID** C12 · **Binary** `xustive-crawler` · **Upstream** [[Crawler Orchestrator]] via `q:fetch` · **Downstream** [[Content Parser]] via `q:parse`

## 1. Purpose

Turn a URL into bytes, safely and politely. Two strategies — a cheap static HTTP fetch and an
expensive headless render — with an explicit rule for when the expensive one is justified.

## 2. Responsibilities

**In scope**: HTTP(S) fetching with conditional requests; redirect handling; charset detection;
size/time caps; headless rendering when required; storing the raw blob; emitting `q:parse` messages.

**Out of scope**: scheduling (→ [[Crawler Orchestrator]]); robots decisions (→ [[Politeness and Robots]]);
proxy selection policy (→ [[Proxy Manager]]); HTML parsing (→ [[Content Parser]]).

## 3. Interface

Consumes `q:fetch`: `{ url, kind, depth, etag?, last_modified?, force_headless? }`
Produces `q:parse`: `{ url, final_url, kind, raw_ref, content_type, charset, http_status, headers_subset, fetched_at, fetch_method, redirect_chain }`

`raw_ref` is a Redis key `raw:{trace_id}` with a 7-day TTL holding the compressed body
([[Data Model]] §6, §8).

## 4. Internal Design

### 4.1 Static path (default)

`reqwest` with:

| Setting | Value |
|:---|:---|
| Timeouts | connect 10 s, read 30 s, total 60 s |
| Redirects | max 5, **each re-validated** by `SafeUrl` |
| Body cap | 10 MB, streamed and aborted on exceed |
| Compression | gzip, brotli, deflate |
| HTTP version | prefer h2 |
| Conditional | `If-None-Match`, `If-Modified-Since` from the last visit |
| User-Agent | `XustiveBot/1.0 (+https://xustive.dz/bot)` — identifiable, with a page explaining how to block us |
| Connection pool | per-host cap 1 (politeness), global 200 |

Charset detection: `Content-Type` header → HTML `<meta charset>` → `chardetng` sniff → assume UTF-8.
Algerian sites still ship `windows-1256` and `iso-8859-1`; getting this wrong silently produces
mojibake that survives all the way to the index.

### 4.2 Headless path

`obscura` headless browser, used **only when justified**:

- The source is marked `requires_render` in the registry, **or**
- the static fetch returned < `min_text_bytes` (512) of extractable text while the HTML contains a
  known SPA root (`<div id="root">`, `__NEXT_DATA__`, `ng-app`), **or**
- the parser reported `needs_render` for this URL previously.

Headless settings: JS enabled, images/fonts/media blocked (bandwidth), 1366×768 viewport, wait for
`networkidle` or 8 s, hard cap 25 s. Result is the serialised DOM.

Headless costs roughly **30× a static fetch** in CPU and time. Its use is capped at
`headless_ratio` (default 10 %) of total fetches; over the cap, work is deferred rather than starving
static crawling.

### 4.3 Three fetch modes

Mode follows the crawl profile from [[Politeness and Robots]] §4.0, not the individual request.

| Mode | Client | Used by | Cost |
|:---|:---|:---|:---|
| `plain` | `reqwest`, honest `XustiveBot` UA | **all `open_web` sources** | baseline |
| `impersonated` | `rquest` with a pinned [[Fingerprint Engine]] profile | `platform` sources, HTTP-level paths | +TLS handshake |
| `stealth_headless` | real Chrome + CDP fingerprint patches, persistent per-identity profile | `platform` sources needing JS execution | ~30× plain |

`plain` remains the default and covers the large majority of traffic. In this mode the fetcher
identifies itself honestly, publishes a bot info page, honours `robots.txt`, `Crawl-delay`, and
`429`/`Retry-After`, and does **not** attempt to evade blocking — unchanged by
[[ADR-0009 - Direct Collection for Social Platforms]].

`impersonated` and `stealth_headless` exist for the `platform` profile. They take their fingerprint,
proxy, and identity as a pinned tuple from [[Session Manager]] and cannot mix components
([[Session Manager]] §4.2). A CI check asserts no `web`-kind source can select either mode.

### 4.3b Stealth headless specifics

- Real Chrome with `--headless=new`, not `chromium-headless-shell` — the shell has distinguishing
  behaviour beyond what patching can hide.
- Persistent user-data directory **per identity**, so cookies, storage, and cache age naturally. A
  browser with an empty profile on every visit is itself a signal.
- CDP `Page.addScriptToEvaluateOnNewDocument` applies the JS-surface patches before any page script
  runs ([[Fingerprint Engine]] §4.6).
- WebRTC forced through the proxy or disabled — a leaked local address defeats every other layer.
- Images and fonts blocked unless the page needs them to render content, since bandwidth is billed on
  residential pools.
- Same sandbox as before: own container, no `core` network access, read-only filesystem, dropped
  capabilities, seccomp. Fingerprint patching does not relax any of it.

### 4.4 Outcome classification

| Status | Class | Action |
|:---|:---|:---|
| 200 | ok | emit `q:parse` |
| 304 | unchanged | no parse; update revisit interval |
| 301/302/307/308 | redirect | follow (≤ 5), record chain, canonicalise |
| 400/401/403 | permanent | drop; increment host error count |
| 404/410 | gone | drop; signal orchestrator to remove from frontier |
| 429 | throttled | honour `Retry-After`, open breaker, requeue |
| 5xx | transient | retry (4 attempts, jittered backoff) |
| timeout / reset | transient | retry |
| body > cap | permanent | drop, `WARN` |
| non-allowlisted content type | permanent | drop quietly |

## 5. Configuration

| Key | Default |
|:---|:---|
| `user_agent` | `XustiveBot/1.0 (+https://xustive.dz/bot)` |
| `connect_timeout_s` / `read_timeout_s` / `total_timeout_s` | 10 / 30 / 60 |
| `max_body_bytes` | 10 MiB |
| `max_redirects` | 5 |
| `per_host_concurrency` | 1 |
| `global_concurrency` | 64 |
| `headless_enabled` | `true` |
| `headless_ratio` | 0.10 |
| `headless_timeout_s` | 25 |
| `min_text_bytes` | 512 |
| `raw_ttl_days` | 7 |
| `allowed_content_types` | html, xhtml, xml, json, text |
| `fetch_mode_by_profile` | `open_web → plain`; `platform → impersonated`, escalating to `stealth_headless` |
| `stealth_profile_dir` | `/var/lib/xustive/browser/{identity_id}` |
| `block_images_headless` | `true` |

## 6. Data

Writes `raw:{trace_id}` (zstd-compressed body, 7-day TTL) and `q:parse` messages. Updates
`crawl:{host}` counters. Never writes to the index.

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| DNS failure | resolver error | transient ×2, then mark host suspect |
| TLS error / expired cert | handshake error | permanent for that host; `WARN` (many `.dz` sites have cert issues — do **not** disable verification; record and skip) |
| Connection reset mid-body | stream error | retry once, then DLQ |
| Body exceeds cap | streamed counter | abort connection, drop |
| Charset undetectable | sniff failure | assume UTF-8, flag `charset_guessed` |
| Headless timeout | 25 s cap | fall back to the static body already fetched |
| Headless crash / OOM | process supervision | restart browser pool, requeue once |
| Redirect loop | chain length | permanent, drop |
| Redirect to a private IP | `SafeUrl` re-check | **drop and alert** — SSRF attempt ([[Security and Privacy]] T3) |

## 8. Performance

| Metric | Budget |
|:---|:---|
| Static fetch throughput | ≥ 100 pages/min/worker ([[Performance Budgets]]) |
| Static fetch latency | ≤ 3 s p95 (network-bound) |
| Headless render | ≤ 12 s p95 |
| Memory | ≤ 1 GB (static); browser pool bounded to 2 instances |

## 9. Observability

`xustive_fetch_total{source_type,outcome}`, `xustive_fetch_duration_seconds{method}`,
`xustive_fetch_bytes`, `xustive_headless_total`, `xustive_headless_ratio`,
`xustive_redirect_chain_length`, `xustive_charset_guessed_total`, `xustive_ssrf_blocked_total`.
URLs are corpus data, not user data — logging them is fine and necessary.

## 10. Security

- Every URL and every redirect target passes `SafeUrl` ([[Security and Privacy]] §4). This is the
  system's primary SSRF defence and it is tested explicitly.
- TLS verification is **never** disabled. A broken-cert site is skipped, not trusted.
- Bodies are treated as hostile bytes: size-capped, never executed, never shell-interpolated.
- The headless browser runs with JS enabled — the highest-risk surface in the system. It runs in its
  own container with no access to the `core` network, a read-only filesystem, dropped capabilities,
  and a seccomp profile.

## 11. Testing

- Local fixture server covering: gzip, brotli, chunked, `windows-1256`, redirect chains, redirect to
  `127.0.0.1` (must be blocked), 429 with `Retry-After`, slow-loris, 20 MB body, malformed HTML.
- Conditional requests: assert `If-None-Match` is sent and 304 short-circuits parsing.
- Politeness: assert single in-flight request per host and `Crawl-delay` compliance.
- Headless: an SPA fixture that yields text only after render; assert the escalation rule fires and
  the ratio cap holds.
- Security: SSRF suite (private IPs, DNS rebinding, redirect chains, IPv6 literals, decimal IPs).

## 12. Open Questions

- [ ] Do we support HTTP/3? Marginal benefit, extra surface.
- [ ] Should `raw_ref` blobs go to object storage instead of Redis at scale? (Redis memory grows with
      crawl rate × 7 days — this becomes a real constraint above ~5M docs/week.)
- [ ] Per-host adaptive concurrency: could we safely go to 2 for large, fast hosts that allow it?

## Related

[[Crawler Orchestrator]] · [[Politeness and Robots]] · [[Proxy Manager]] · [[Content Parser]] ·
[[Security and Privacy]] · [[Task Queue]]
