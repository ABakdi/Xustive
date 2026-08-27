---
tags:
  - component
  - ingestion
component-id: C12
binary: xustive-cli crawld
status: built
updated: 2026-08-27
---

# Web Fetcher

> **ID** C12 · **Module** `crates/xustive-ingest/src/fetch.rs` (`Fetcher`, `FetchConfig`,
> `Fetched`, `FetchError`) with `robots.rs`, `robots_cache.rs`, `exclusion.rs`, `raw_store.rs` ·
> **Upstream** [[Crawler Orchestrator]], in-process · **Downstream** [[Content Parser]], in-process

## 1. Purpose

Turn a URL into bytes, safely and politely. One strategy today: a plain HTTP fetch that
identifies itself honestly. The headless and impersonated paths that earlier drafts specified are
**not built** (2026-08-27); what remains of that design is the seam it would plug into (§4.3).

## 2. Responsibilities

**In scope**: HTTP(S) fetching with conditional requests; manual redirect handling with `SafeUrl`
on every hop; charset detection; size and time caps; `robots.txt` and `X-Robots-Tag`; per-host
pacing; born-digital PDF text; the optional raw-body store.

**Out of scope**: scheduling (→ [[Crawler Orchestrator]]); the robots *policy* (→
[[Politeness and Robots]]); proxy selection (→ [[Proxy Manager]]); HTML parsing (→
[[Content Parser]]).

## 3. Interface

There are no `q:fetch` / `q:parse` streams. The orchestrator holds a `Fetcher` and calls it
directly; the only stream in the crawl path is `q:index` after parsing ([[Task Queue]]).

```rust
Fetcher::new(FetchConfig) -> Fetcher            // .with_shared_cache(RobotsCache) to share robots via Redis
fetcher.get(url)                                // blocks until the host's crawl-delay has elapsed
fetcher.get_conditional(url, Conditional { etag, last_modified })
fetcher.sitemaps_for(host)                      // Sitemap: lines from robots.txt
-> Fetched { url, final_url, status, body, content_type, charset_guessed, exclusion, etag, last_modified }
```

A `304` comes back as `Ok` with `status == 304` and an empty body — not as an error, because it is
the best possible answer: the page is exactly what we hold, learned for a few hundred bytes.
`exclusion` carries what `X-Robots-Tag` asked for; the fetcher reports what the server said and
the caller decides — a `noindex` page is still worth crawling for its links, and the header is the
only way a PDF or an image can refuse indexing.

## 4. Internal Design

### 4.1 The client

`reqwest`, with:

| Setting | Value |
|:---|:---|
| Timeouts | connect **5 s**, total **20 s** — a worker is pinned to one fetch, so a slow host is stolen throughput, not just a slow page |
| Redirects | `Policy::none()`; followed by hand, at most `safe_url::MAX_REDIRECTS` = 5 hops, **each re-validated** by `SafeUrl` |
| Body cap | `max_body_bytes` 10 MiB; PDFs 12 MiB and 200 000 extracted characters |
| Conditional | `If-None-Match` / `If-Modified-Since` from the last `Visit` |
| User-Agent | `XustiveBot/1.0 (+https://xustive.dz/bot; Algerian search engine)`; robots token `xustivebot`; the page is `web/app/(operator)/bot` |
| Per-host concurrency | 1, enforced by the `Politeness` lock held across the wait, plus a 200 ms `politeness_margin` |
| Indexable types | `text/html`, `application/xhtml+xml`, `text/plain`, `application/xml`, `text/xml`, RSS, Atom, `application/pdf` |

Charset: `Content-Type` header → `<meta charset>` / `<meta http-equiv>` in the first 2 KB →
`chardetng` sniff. A declared charset, wherever declared, is not counted as guessed. Algerian sites
still serve `windows-1256` with a bare `Content-Type` header, and getting this wrong silently
produces mojibake that survives all the way to the index.

PDFs go through `pdf-extract` inside `catch_unwind`, because the library panics on some malformed
files and a panicking fetch would take a worker with it. Scanned PDFs yield nothing and fall out as
thin, which is correct until OCR exists ([[Media Extraction]]).

### 4.2 Politeness — the part that keeps us welcome

`robots.rs`. An unreachable `robots.txt` is **not** permission: 5xx, timeout, 401 and 403 all
mean full disallow; only 404 means "no restrictions". Rules are cached 24 h in-process and, when
`RobotsCache` is attached, in Redis so sixty-four workers do not each re-read the file — a site
watching its logs sees a burst of identical requests for the one file that tells us not to. That
cache fails *open* to fetching robots properly, never to "no rules, therefore disallow", which
would turn a cache outage into a silent halt.

The delay a host gets is the **maximum** of its declared `Crawl-delay`, the registry's value, our
adaptive delay and the 1.5 s default, capped at 60 s (`resolve_delay`) — taking the minimum would
let a configuration change silently undo a request the site made. Adaptive pacing grows fast and
shrinks slowly: 429 → `Retry-After` or 4×, clamped to 60–600 s; 5xx → 2× up to 300 s; each clean
response relaxes by 10 % toward a floor. A robots-silent host **earns** a 1 s floor after 20
consecutive clean responses (PROB-002); a single error resets the streak.

`ignore_politeness` is threaded through `FetchConfig`, not a global, so a `Fetcher` built without
asking for it can never acquire it. It skips robots, delays and host opt-outs; it does **not**
lift the global or takedown blocklists (`exclusion::Blocklist`), because a testing flag must not
be able to lift a court order.

### 4.3 What the earlier design specified and where it stands

| Mode | Status (2026-08-27) |
|:---|:---|
| `plain` — honest `XustiveBot` | **Built.** All traffic |
| `impersonated` — browser-accurate TLS/HTTP-2 client with a pinned [[Fingerprint Engine]] profile | **Not built.** The profile catalogue and coherence checker exist; no impersonating client library is wired |
| `stealth_headless` — real Chrome under CDP | **Not built.** No headless browser anywhere in the workspace |

The seam is the [[Proxy Manager]] / [[Session Manager]] lease: a fetch through those would take
proxy, fingerprint and identity as a pinned tuple. Today the only outbound path that accepts a
proxy at all is the SERP client's `discovery.serp_proxy`
([[ADR-0013 - Direct SERP Collection for Discovery]]).
The commitment that does not change with any of it: in `plain` mode the fetcher publishes a bot
page, honours `robots.txt`, `Crawl-delay` and `429`/`Retry-After`, and does **not** evade blocking
([[ADR-0009 - Direct Collection for Social Platforms]] altered the platform stance only).

### 4.4 Outcome classification

`FetchError::outcome()` gives the crawl counters a finer label than retry-or-not, so a spike in
`gone` (sites removing content) reads differently from `throttled` (we are rate-limited) or
`transient` (the network is flaky):

| Result | Outcome | Action |
|:---|:---|:---|
| 200 | ok | parse |
| 304 | unchanged | no parse; the revisit interval grows |
| 3xx | followed (≤ 5, each `SafeUrl`-checked); loop → `redirect_loop` | permanent |
| 404 / 410 | `gone` | the frontier forgets the page |
| 429 | `throttled` | `Retry-After` honoured, adaptive delay raised |
| 5xx, timeout, transport | `transient` | retried on a later visit; delay raised on 5xx |
| robots disallow | `robots` | permanent, counted |
| unsafe URL or hop | `unsafe` | dropped |
| non-indexable type, body over cap | `content_type` / `too_large` | permanent |

### 4.5 Sitemaps

`sitemaps_for(host)` reads `Sitemap:` lines from `robots.txt`, following an `http`→`https` or
apex→`www` redirect on the file itself — those looked like refusals in the first version.

### 4.6 The raw store

`raw_store.rs`. Keeps the fetched body in Redis (`frontier:raw:{url}`) for `crawl.raw_ttl_days`,
per-blob cap 5 MiB, so
extraction can be re-run without a re-fetch — the reason `xustive-cli media-repass` is free to the
sites. **Off by default** (0 days): blanket storage would overwhelm the 1 GB development Redis, and
the real home is object storage. Best-effort throughout: a store that cannot be written is a lost
reindex convenience, never a lost document.

## 5. Configuration

| Key | Default | Where |
|:---|:---|:---|
| `connect_timeout` / `total_timeout` | 5 s / 20 s | `FetchConfig` |
| `max_body_bytes` | 10 MiB | `FetchConfig` |
| `robots_ttl` | 24 h | `FetchConfig`, `robots_cache::TTL` |
| `politeness_margin` | 200 ms | `FetchConfig` |
| `crawl.ignore_politeness` | `false`; refused in production | `config/*.toml` |
| `crawl.raw_ttl_days` | `0` (off) | `config/*.toml` |
| `crawl.respect_crawl_delay`, `crawl.per_host_concurrency` | `true`, `1` | `config/*.toml` |

The headless/proxy keys of earlier drafts (`headless_ratio`, `stealth_profile_dir`, …) do not
exist.

## 6. Failure Modes

| Failure | Response |
|:---|:---|
| Redirect to a private address, any hop | dropped as `unsafe` — the SSRF check that matters, because a public host can 302 to `169.254.169.254` |
| TLS error / expired certificate | permanent for that fetch; verification is **never** disabled, many `.dz` sites have certificate issues and are skipped, not trusted |
| Body over cap | `too_large`, dropped |
| Charset undeclared | sniffed; `charset_guessed = true` on the document |
| Malformed PDF | panic caught; `content_type` error |
| Robots unreachable | disallow, until the TTL passes |
| Redis (robots cache, raw store) down | robots fetched directly; raw body not stored; the crawl continues |

## 7. Security

Every URL and every redirect target passes `SafeUrl` ([[Security and Privacy]]); `tests/ssrf.rs`
covers private ranges, IPv6 literals, decimal IPs and redirect chains. Bodies are hostile bytes:
size-capped, never executed, never shell-interpolated. There is no JS execution surface in the
fetcher today.

## 8. Testing

`tests/fixture_site.rs` (redirect chains, traps, 404s), `tests/ssrf.rs`,
`tests/robots_conformance.rs`, `tests/robots_sharing.rs` (the Redis cache), `tests/raw_store_redis.rs`,
`tests/adversarial.rs`; unit tests in `fetch.rs` for `windows-1256` from header, meta and sniff.

## 9. Open Questions

- [ ] Raw bodies to object storage instead of Redis, so the store can be on by default.
- [ ] Per-host adaptive concurrency above 1 for large hosts that allow it.
- [ ] Whether the impersonated client is ever wanted for the open web, or only ever behind a
      platform lease (the current design says only behind a lease).

## Related

[[ADR-0011 - Adaptive Recrawl over Static Crawling]] · [[Crawler Orchestrator]] ·
[[Politeness and Robots]] · [[Proxy Manager]] · [[Content Parser]] · [[Security and Privacy]] ·
[[Task Queue]] · [[Media Extraction]]
