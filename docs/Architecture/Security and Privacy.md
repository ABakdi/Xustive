---
tags:
  - architecture
  - security
type: architecture
status: specified
updated: 2026-08-06
---

# Security and Privacy

> Xustive's differentiator is a promise: **your searches are not recorded and nothing leaves the
> country**. This note defines how that promise is made *structurally* true rather than
> policy-true. Legal obligations are in [[Legal and Compliance]].

---

## 1. Privacy Invariants

These are testable statements, not aspirations. Each has an enforcement mechanism.

| # | Invariant | Enforced by |
|:---|:---|:---|
| P1 | A query string is never written to durable storage | CI telemetry lint + code review; no query in any `tracing` field ([[Observability]]) |
| P2 | A query string is never sent outside the `core` Docker network | `xustive-api`/`xustive-ml` have **no egress route** ([[Deployment Topology]]) |
| P3 | No cookies, no `localStorage` identifiers, no fingerprinting | UI ships zero third-party JS; CSP blocks external origins ([[UI Specification]]) |
| P4 | Uploaded audio and images are never written to disk | processed from an in-memory buffer, dropped on response ([[Speech to Text]], [[Image Pipeline]]) |
| P5 | Client IPs are not stored | rate limiter keys on `HMAC(ip/24, daily_rotating_salt)`, in-memory, 60 s TTL |
| P6 | Aggregate query stats are k-anonymous (k ≥ 20) | [[Autocomplete Service]] drops any term seen by < 20 distinct buckets/day; the interaction store (below) is refused at startup with k < 20 outside dev ([[Interaction Signals]]) |
| P7 | No third-party analytics, fonts, CDNs, or AI APIs | CSP `default-src 'self'`; dependency review in CI |
| P8 | Interaction signals hold no per-person record | impressions/clicks are bare Redis counters keyed on `(query, doc)` — no IP, session, or user in any key (a key-shape unit test proves the construction has no identifier input); surfaced only above k, decaying out of a sliding window; **off by default** ([[ADR-0015 - Anonymous Interaction Signals for Ranking]], M6) |

### Interaction signals (M6) reconciled against "no query logging"

[[ADR-0008 - No Query Logging]] left one escape hatch: *aggregate counters, k-anonymous, default
off*. The interaction store is that hatch and nothing more. It records how often results are shown
and clicked, as counters keyed by the (already-normalised) query and the document id — the same
[[weak_coverage]] pattern already in use — never by anything that identifies a person. The honest
one-line statement of the guarantee: **no identifiable tracking; anonymous aggregate counts only,
k-anonymous, on your own server, and off unless you turn it on.** The click beacon carries an opaque
token and a document id — never the query text — so even the click request cannot be tied to what
was typed.

### Verifying the promise

- **Egress test** (CI, staging): from inside `xustive-api`, attempt outbound DNS + TCP to a public
  host; the test **passes only if it fails**.
- **Telemetry lint**: grep `tracing::` call sites for query-shaped identifiers → build failure.
- **Log scan** (nightly, staging): run the full query corpus, then grep 24 h of logs for any corpus
  string. One hit = `TelemetryLeak` page.
- **Disk scan**: after a voice/image request, assert no new files under the container's writable
  layer.

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

- TLS 1.3 only (1.2 permitted through 2026 for old Android); HSTS `max-age=63072000; includeSubDomains`.
- Certificates via Let's Encrypt / ACME at the Caddy edge, auto-renewed.
- Security headers on every HTML response:

```
Content-Security-Policy: default-src 'self'; img-src 'self' data: https:; media-src 'self' blob:;
  script-src 'self'; style-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'
Referrer-Policy: no-referrer
X-Content-Type-Options: nosniff
Permissions-Policy: microphone=(self), camera=(self), geolocation=(), interest-cohort=()
Cross-Origin-Opener-Policy: same-origin
```

`img-src https:` is required to show remote thumbnails; every such image gets
`referrerpolicy="no-referrer"` and `loading="lazy"` so the origin host learns nothing about the user
beyond an IP for a thumbnail — see [[UI - Results Page]] for the proxy-thumbnails alternative in
[[#9. Open Questions]].

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
| Size cap | 10 MB | 8 MB |
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
| Admin `X-Api-Key` | hashed (Argon2id) in config; plaintext only at issue | 90 d |
| Proxy credentials | secrets file mounted to `xustive-crawler` only | 90 d |
| Rate-limit salt | generated at boot, rotated daily, memory-only | 24 h |
| Grafana admin | secrets file; UI behind VPN/basic auth | 180 d |

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

- [ ] Proxy remote thumbnails through `xustive-api` so the user's IP never touches Facebook's CDN?
      Costs bandwidth and adds a cache that could be argued to be a log. **Leaning yes for beta.**
- [ ] Do we publish a canary/transparency statement, and who signs it?
- [ ] Is the k ≥ 20 threshold for autocomplete defensible, or should popular-query suggestions be
      dropped entirely in favour of a static curated list?
- [ ] Onion service / mirror for censorship resistance — in scope or a distraction?

## Related

[[Legal and Compliance]] · [[Observability]] · [[Deployment Topology]] · [[API Gateway]] ·
[[Summarizer]] · [[Politeness and Robots]] · [[Decision Log]]
