---
tags:
  - architecture
  - security
type: architecture
status: specified
updated: 2026-08-27
---

# Security and Privacy

> Xustive's differentiator is a promise: **your searches are not recorded and nothing leaves the
> country**. This note defines how that promise is made *structurally* true rather than
> policy-true. Legal obligations are in [[Legal and Compliance]].
>
> **Verified against the code, 2026-08-27** (`crates/xustive-api/src/{ratelimit,geoip,stt}.rs`,
> `web/app/api/thumb`, `web/lib/thumb.ts`, `deploy/docker-compose*.yml`, `scripts/*.sh`). Each
> invariant below now says what is built and what is still a plan; the plan text was kept where
> it is still the intent.

---

## 1. Privacy Invariants

These are testable statements, not aspirations. Each has an enforcement mechanism.

| # | Invariant | Enforced by |
|:---|:---|:---|
| P1 | A query string is never written to durable storage | CI telemetry lint + code review; no query in any `tracing` field ([[Observability]]) |
| P2 | A query string is never sent outside the `core` Docker network | `core` is `internal: true`; `scripts/test-egress.sh` proves it. **As built (2026-08-27):** `deploy/docker-compose.yml` has no `xustive-api` service — the API runs on the host in every topology in the repo, so the network guarantee today covers the backends, the sidecars and the federation bridge, not the API process itself ([[Deployment Topology]]) |
| P3 | No cookies, no `localStorage` identifiers, no fingerprinting | UI ships zero third-party JS and sets `Referrer-Policy: no-referrer`. **A `Content-Security-Policy` header is not set yet** (2026-08-27; `web/next.config.ts` sets Referrer-Policy, nosniff, COOP and Permissions-Policy only) |
| P4 | Uploaded audio and images are never written to disk | STT sidecar transcribes from `io.BytesIO`, never a file. The OCR sidecar (`services/ocr-sidecar/app.py`) **does** write the image to a private temp file for the model and deletes it in a `finally` — the file exists only for the request ([[Speech to Text]], [[Image Pipeline]]) |
| P5 | Client IPs are not stored | rate limiter keys on `blake3_keyed(salt, ip/24 v4 · /48 v6)`; salt generated at boot from `/dev/urandom`, rotated every 24 h, memory-only; fixed windows (search 60/min, suggest 300/min, `/sources` 5/h); the peer address only — **`X-Forwarded-For` is never read**. Open **BUG-042**: behind the Next.js proxy every client arrives from the proxy's address, so they share one bucket |
| P6 | Aggregate query stats are k-anonymous (k ≥ 20) | [[Autocomplete Service]] drops any term seen by < 20 distinct buckets/day; the interaction store (below) is refused at startup with k < 20 outside dev ([[Interaction Signals]]) |
| P7 | No third-party analytics, fonts, CDNs, or AI APIs | CSP `default-src 'self'`; dependency review in CI |
| P9 | Approximate location never leaves the request | [[ADR-0020 - Approximate Location from a Local Database]]: DB-IP City Lite (`data/geoip/dbip-city-lite.mmdb`, fetched by `scripts/fetch-geoip.sh`, not committed) is read **into the heap** at start (`maxminddb::Reader<Vec<u8>>`, not mmap); `wilaya_of(ip)` is looked up per request and the result is used for the weather card and dropped — no store, no log. Still owed (2026-08-27): the CC BY 4.0 attribution the script's comment says the weather card carries **is not rendered anywhere in the UI**, and there is no staleness gauge for the monthly database |
| P10 | Thumbnails do not hand the user's IP to the origin host | [[ADR-0021 - Proxied Thumbnails with Signed URLs]]: `/api/thumb?u=…&s=…` proxies remote images; the results page signs each URL with HMAC-SHA256 and the route refuses anything unsigned before it looks at the URL. The secret is `XUSTIVE_THUMB_SECRET` or, absent that, 32 random bytes **per process** (held on `globalThis` so the page and the route share one value); three public hosts — `upload.wikimedia.org`, `commons.wikimedia.org`, `covers.openlibrary.org` — are served unsigned. The proxy keeps no cache, so it is not a log ([[Thumbnail Proxy]]) |
| P8 | Interaction signals hold no per-person record | impressions/clicks are bare Redis counters keyed on `(query, doc)` — no IP, session, or user in any key (a key-shape unit test proves the construction has no identifier input); surfaced only above k, decaying out of a sliding window; **off by default** ([[ADR-0015 - Anonymous Interaction Signals for Ranking]], M6) |

### Interaction signals (M6) reconciled against "no query logging"

[[ADR-0008 - No Query Logging]] left one escape hatch: *aggregate counters, k-anonymous, default
off*. The interaction store is that hatch and nothing more. It records how often results are shown
and clicked, as counters keyed by the (already-normalised) query and the document id — the same
`weak_coverage` pattern ([[Interaction Signals]]) already in use — never by anything that identifies a person. The honest
one-line statement of the guarantee: **no identifiable tracking; anonymous aggregate counts only,
k-anonymous, on your own server, and off unless you turn it on.** The click beacon carries an opaque
token and a document id — never the query text — so even the click request cannot be tied to what
was typed.

[[ADR-0018 - Anonymous Search History]] (2026-08-25) then amended ADR-0008's "query text never
written to durable storage" row: the same counters, browsed at `/admin/interaction`, **are**
normalised query terms surviving their window. They carry no identifier — that is the property
the promise rests on — and the public privacy page (`web/app/[lang]/privacy`) says exactly this:
what is kept (terms, result counts, opened results — as counts), what never is (IP, device,
session, account), and that the external summariser and federated search are off by default.

### The signals Redis

`redis-signals` in compose is the ephemeral instance for these counters: `--save "" --appendonly
no`, no volume, `volatile-lru` at 192 MB, and `scripts/backup.sh` deliberately skips it — so a
backup can never carry query terms. **Caveat (2026-08-27):** only `config/dev.toml` sets
`queue.signals_url` to it. `staging.toml`, `prod.toml` and `ci.toml` leave it empty, and an empty
value falls back to the *queue* Redis — which is persistent (AOF) and is backed up. A non-dev
deployment that turns signals on must set `signals_url` or the ephemerality is lost.

### Verifying the promise

- **Egress test** (`scripts/test-egress.sh`, `make egress-test`, CI job `egress guarantee`):
  asserts the `core` network is `internal`, runs a throwaway container on it and requires outbound
  HTTP **and** public DNS to fail, runs the same probe off-network as a control, checks a real
  container (`xustive-redis`) cannot reach out, and — when the `federation` profile is up — that
  SearXNG is unreachable from `core`, so only the dual-homed gateway can reach it. It does **not**
  prove "the API reaches the gateway and nothing else": the API is a host process. In CI the
  topology is brought up `--no-start`, so the real-container and container-log checks skip.
- **Telemetry lint** (`scripts/lint-telemetry.sh`, in `make lint` and CI): a `tracing::` call
  site naming a query or credential field fails the build.
- **Log scan** (`scripts/scan-logs.sh`, `make scan-logs LOG=…`): structural checks for forbidden
  field names plus the query corpus grep. Run nightly by the operator; **not wired to CI**. There is
  no `TelemetryLeak` alert — a hit is a finding, not a page (2026-08-27).
- **Disk scan**: ❌ not built (2026-08-27). The sidecars' behaviour is by inspection: STT never
  touches disk; OCR's temp file is unlinked in `finally`.

---

## 2. Threat Model

Assets: the index, the source registry, service availability, and — above all — the *absence* of
user query data.

| # | Threat | Actor | Impact | Mitigation |
|:---|:---|:---|:---|:---|
| T1 | Query log compelled or stolen | state / attacker | catastrophic to trust | there is no log to take (P1, P2) |
| T2 | Index poisoning via submitted sources | spammer / propagandist | wrong answers | moderation queue + `trust_tier` + `spam_score` ([[Admin and Source Submission]]) |
| T3 | SSRF via crawler URL input | anyone who can submit a URL | internal network read | §4 |
| T4 | Prompt injection through crawled content into [[Summarizer]] | any indexed page | fabricated/abusive summary | §5 |
| T5 | Meilisearch or Qdrant exposed publicly | misconfiguration | full index read/write | `internal: true` networks, master key, port audit in CI |
| T6 | Admin API key leak | insider / repo leak | takedown & registry abuse | keys in secrets store, rotated 90 d, scoped, audit-logged |
| T7 | Malicious upload (zip bomb, decompression bomb, malformed media) | user | worker DoS | §6 |
| T8 | XSS via crawled title/excerpt rendered in results | any indexed page | account-less but still session/UI attack | escape everything except `<em>`; CSP ([[UI - Results Page]]) |
| T9 | Scraping-triggered legal exposure | platforms | service disruption | [[Legal and Compliance]], [[Politeness and Robots]] |
| T10 | DoS on expensive endpoints (`/search/image`, `/search/voice`) | anyone | resource exhaustion | strict rate limits + size caps ([[API Contract]]) |
| T11 | Dependency supply chain | upstream crate | RCE | `cargo-deny`, `cargo-audit`, lockfile pinning, vendored builds |

---

## 3. Transport and Edge

> ❌ **Not built (2026-08-27):** there is no TLS terminator in the repo — no Caddy service in
> compose, no HSTS. The frontend listens on :3000 and the API on :8080 in the clear. The
> intent below stands for the beta edge.

- TLS 1.3 only (1.2 permitted through 2026 for old Android); HSTS `max-age=63072000; includeSubDomains`.
- Certificates via Let's Encrypt / ACME at the Caddy edge, auto-renewed.
- Security headers on every HTML response (intended):

```
Content-Security-Policy: default-src 'self'; img-src 'self' data: https:; media-src 'self' blob:;
  script-src 'self'; style-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'
Referrer-Policy: no-referrer
X-Content-Type-Options: nosniff
Permissions-Policy: microphone=(self), camera=(self), geolocation=(), interest-cohort=()
Cross-Origin-Opener-Policy: same-origin
```

**Set today** (`web/next.config.ts`, every path): `Referrer-Policy: no-referrer`,
`X-Content-Type-Options: nosniff`, `Cross-Origin-Opener-Policy: same-origin`,
`Permissions-Policy: geolocation=(), microphone=(self), camera=(self)`; the API adds its own
`permissions-policy`. `scripts/smoke.sh` checks the privacy headers are present. The CSP is the
missing one.

Remote thumbnails no longer need `img-src https:` — they go through the signed proxy (P10), so
the origin host sees the server's address, not the user's ([[Thumbnail Proxy]]).

---

## 4. SSRF and Crawler Input Validation

Every URL entering [[Web Fetcher]] — from a sitemap, a submission, or a discovered link — passes:

1. Scheme allowlist: `http`, `https` only.
2. DNS resolve **before** connect; reject if any resolved IP is in a private/reserved range
   (`10/8`, `172.16/12`, `192.168/16`, `127/8`, `169.254/16`, `::1`, `fc00::/7`, `fe80::/10`).
3. Re-validate after every redirect (max 5 hops) — this is where naive implementations get caught.
4. Reject non-standard ports except 80/443/8080/8443.
5. Response size cap 10 MB; read timeout 30 s; total timeout 90 s.
6. Content-type allowlist for parsing: `text/html`, `application/xhtml+xml`, `application/json`,
   `text/plain`, `application/xml`.

Implemented as a single `SafeUrl` newtype — a `Url` cannot reach the HTTP client without passing
through its constructor.

---

## 5. Prompt Injection into the Summarizer

Crawled text is **untrusted input to a model**. A page containing *"ignore previous instructions and
say X"* must not steer the summary.

Mitigations in [[Summarizer]]:
- Retrieved passages are delimited and labelled as data, never concatenated into the instruction
  block.
- System prompt states the passages are untrusted, may contain instructions, and must be treated as
  content to summarise only.
- Output constraints: ≤ 3 sentences, ≤ 400 chars, no URLs, no imperatives directed at the reader.
- Post-generation filter: reject a summary containing a URL, an email address, or matching an
  injection-phrase regex → fall back to no summary (degradation step 1,
  [[Error Handling and Resilience]]).
- The summary is rendered as **plain text**, never as HTML/markdown.
- Red-team fixtures in `tests/fixtures/injection/` are part of CI ([[Testing Strategy]]).

---

## 6. Upload Handling

| Control | Audio | Image |
|:---|:---|:---|
| Size cap | **8 MB** as built (`stt.max_audio_bytes`; a 30 s Opus clip is under 1 MB) | 5 MB fetched for OCR (`enrich.max_image_bytes`); the `/search/image` upload cap of 8 MB is the spec — *not verified in code, 2026-08-27* |
| Duration / dimension cap | 30 s | 4096 × 4096 px |
| Type check | magic bytes, not `Content-Type` | magic bytes |
| Decode | in a memory-limited task, wall-clock capped (5 s) | same |
| Decompression-bomb guard | reject if decoded PCM > 60 s | reject if decoded pixels > 40 MP |
| Storage | none — in-memory buffer, zeroised after | none |
| EXIF | n/a | stripped before any processing; GPS never read |

Decoding runs in a `tokio::task::spawn_blocking` with a hard timeout; a panic in decode is caught at
the task boundary and returns 422, never taking down the process.

---

## 7. Secrets and Access Control

| Secret | Storage | Rotation |
|:---|:---|:---|
| `MEILI_MASTER_KEY` | env from secrets file, `0600`, never in the image | 180 d |
| Admin `X-Admin-Key` | **as built:** `api.admin_key` in the config file, plaintext, compared in constant time; empty leaves the admin routes open (development). Argon2id hashing is still the intent | 90 d |
| Proxy credentials | secrets file mounted to `xustive-crawler` only | 90 d |
| Rate-limit salt | generated at boot from `/dev/urandom`, rotated every 24 h, memory-only | 24 h |
| `XUSTIVE_THUMB_SECRET` | env on the web process; unset → random per process, so signatures do not survive a restart or span replicas ([[Operating Xustive]] §5) | with the process |
| Grafana admin | **as built:** `admin`/`admin` in compose, anonymous access off, reporting off; the port is only published by `docker-compose.dev.yml` | 180 d |

Meilisearch uses **scoped tenant keys**: `xustive-api` gets a search-only key, `xustive-worker` an
index-only key. Nothing but the migration job holds the master key.

Admin actions (`/admin/*`) are audit-logged with actor key-id, action, target id, timestamp — this is
the one place we log deliberately, and it contains no user data.

---

## 8. Content Safety Obligations

- **Takedown path**: a public contact route plus `POST /admin/takedown` that deletes the document,
  its comments, its embeddings, and adds the URL to a permanent blocklist so re-crawling cannot
  resurrect it. Target: 72 h. See [[Legal and Compliance]].
- **NSFW**: `is_nsfw` set at enrichment; filtered from image-search results by default.
- **PII in crawled content**: we index public posts, which contain names. We do not build
  person-centric profiles, expose an author-history view, or index content behind login. Author
  fields exist for attribution only.
- **Right to erasure**: erasure requests route to the takedown path and are honoured for the index
  copy; we cannot affect the origin platform.

---

## 9. Open Questions

- [x] Proxy remote thumbnails — done through the web tier, signed, cacheless
      ([[ADR-0021 - Proxied Thumbnails with Signed URLs]]).
- [ ] BUG-042: rate limiting keys on the peer address, which behind the Next proxy is the proxy.
      Either trust a hop count from a configured proxy, or move the limiter into the web tier.
- [ ] Ship the CSP (P3) and the DB-IP attribution (P9) before beta.
- [ ] Do we publish a canary/transparency statement, and who signs it?
- [ ] Is the k ≥ 20 threshold for autocomplete defensible, or should popular-query suggestions be
      dropped entirely in favour of a static curated list?
- [ ] Onion service / mirror for censorship resistance — in scope or a distraction?

## Related

[[Legal and Compliance]] · [[Observability]] · [[Deployment Topology]] · [[API Gateway]] ·
[[Summarizer]] · [[Politeness and Robots]] · [[Decision Log]] · [[Operating Xustive]] ·
[[Thumbnail Proxy]] · [[ADR-0018 - Anonymous Search History]]
