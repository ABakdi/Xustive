---
tags:
  - component
  - platform
  - ingestion
component-id: C22
binary: xustive-crawler
status: specified
updated: 2026-08-06
---

# Politeness and Robots

> **ID** C22 · **Binary** `xustive-crawler` · **Upstream** [[Crawler Orchestrator]], [[Web Fetcher]] · **Downstream** none

## 0. The testing bypass

A single flag, `crawl.ignore_politeness`, turns all of this off. It exists so the fixture site can
be crawled at full speed without a `robots.txt` round trip per request, and for nothing else.

**Default off.** Enabled in `config/{env}.toml`, or at runtime from `POST /admin/politeness`.

| Bypassed | Not bypassed |
|:---|:---|
| `robots.txt` — not fetched, not consulted | **Global blocklist** — a safety block |
| `Crawl-delay` and the adaptive pacing | **Takedown list** — a legal order |
| Per-host wait between requests | |
| Adaptive slowdown from 429 and 503 | |
| Host opt-out tier | |

The line is drawn there because those two are not politeness. One is a safety block and the other
is a court order, and a testing flag must not be able to lift a court order. Nothing about crawling
a local fixture site needs either of them lifted, so the exclusion costs the intended use nothing.

### Why it is loud

Pointed at the open web this produces exactly the behaviour the rest of this document exists to
prevent, and the damage lands entirely on somebody else's server — there is no symptom in our own
process, and by the time anyone notices we are in an abuse report. So:

- **Production refuses to start** with it enabled. Not a warning: a warning in startup output is a
  warning nobody reads. `CrawlConfig::guard` fails on `prod` and `staging`, and a test loads the
  shipped `config/prod.toml` and `config/staging.toml` through it — a guard the deployed config was
  never run through proves only that the guard compiles.
- The same guard runs again when the admin endpoint is called, rather than trusting a check that
  ran once at startup in a different function.
- Enabling it logs at `warn` with the peer that did it.
- The admin page carries an unmissable banner while it is on, and hides the control entirely in
  environments where it is refused.

`guard` also rejects the quieter routes to the same abuse: `respect_crawl_delay = false` and
`per_host_concurrency > 1`. Neither looks alarming in a diff.


## 1. Purpose

Decide whether we are allowed to fetch a URL, and how fast. This is the component that keeps Xustive
a good citizen of the Algerian web — most of which runs on modest hosting where an aggressive crawler
is indistinguishable from an outage.

Being polite is not only ethics; it is self-interest. Sites that notice us block us, and a blocked
source is a permanent hole in the index.

## 2. Responsibilities

**In scope**: fetching, caching, and evaluating `robots.txt`; `Crawl-delay` enforcement; per-host
rate limiting; adaptive slowdown from host response signals; `X-Robots-Tag` and meta-robots
handling; the takedown/exclusion blocklist.

**Out of scope**: scheduling (→ [[Crawler Orchestrator]], which asks this component for permission);
fetching content (→ [[Web Fetcher]]).

## 3. Interface

```rust
pub trait Politeness: Send + Sync {
    async fn may_fetch(&self, url: &Url) -> Decision;   // Allow | Disallow(reason) | Defer(until)
    async fn crawl_delay(&self, host: &Host) -> Duration;
    fn observe(&self, host: &Host, status: u16, latency: Duration, retry_after: Option<Duration>);
}
```

`observe` closes the adaptive loop — politeness that ignores how the host is actually responding is
just a static rate limit.

## 4. Internal Design

### 4.0 Two crawl profiles ★

[[ADR-0009 - Direct Collection for Social Platforms]] splits collection into two regimes. This
component governs both, with **different rules**:

| | `open_web` profile | `platform` profile |
|:---|:---|:---|
| Applies to | all `web` sources (the large majority of traffic) | Facebook, Instagram, TikTok |
| `robots.txt` | **fetched, parsed, obeyed** — including fail-closed on 5xx | not consulted; these hosts disallow everything, so it carries no per-source signal |
| `Crawl-delay` | honoured | n/a |
| Meta-robots / `X-Robots-Tag` | honoured | n/a |
| Identification | honest `XustiveBot` UA + `/bot` page | browser fingerprint ([[Fingerprint Engine]]) |
| Per-host concurrency | **1**, always | 1 per identity |
| Rate limiting | politeness — protect the host | **self-preservation** — protect the identity ([[Session Manager]] §4.5) |
| On 403 / challenge | halt and flag for review | graded ladder ([[Proxy Manager]] §4.6) |
| Blocklist tiers | all three apply | takedown tier applies |

The `open_web` column is unchanged from before ADR-0009 and is not negotiable in production. Most
Algerian sites run on modest hosting where an aggressive crawler is indistinguishable from an
outage — and being blocked by them is a permanent hole in the index. Changing the collection stance
toward platforms was a deliberate, bounded decision; it is not a general licence to crawl hard.

Rate limits still apply to platforms, for a different reason: there, the constraint is identity
longevity, and the budgets in [[Session Manager]] §4.5 are stricter than politeness would require.

The profile is resolved from the source's `kind` in [[Data Sources Registry]] and cannot be
overridden per request.

### 4.1 `robots.txt` *(`open_web` profile)*

- Fetched on first contact per host, cached in `robots:{host}` for **24 h**.
- Parsed per RFC 9309. `User-agent: XustiveBot` takes precedence over `*`.
- Longest-match rule for `Allow`/`Disallow`; wildcards `*` and `$` supported.
- `Crawl-delay`, though non-standard, is **honoured** when present.
- `Sitemap:` entries are handed to [[Crawler Orchestrator]] for discovery.

Fetch-failure semantics (the part implementations usually get wrong):

| Outcome | Interpretation |
|:---|:---|
| 200 | parse and apply |
| 404 / 410 | no restrictions — crawling allowed |
| 401 / 403 | **treat as full disallow** — the site is refusing us |
| 5xx or timeout | **treat as full disallow**, retry the fetch in 1 h with backoff |
| Unparseable | apply the lines that do parse; ignore the rest |
| > 500 KB | truncate at 500 KB |

Failing *closed* on 5xx is deliberate: an unavailable `robots.txt` is not permission.

### 4.2 Rate limiting

Effective delay per host, taking the maximum of:

| Source | Value |
|:---|:---|
| `robots.txt` `Crawl-delay` | as stated |
| Source registry `crawl_policy.crawl_delay_ms` | as configured |
| Global default | 1 500 ms |
| Adaptive component | §4.3 |

Concurrency per host is **1**, always. Enforced by [[Crawler Orchestrator]]'s per-host scheduling and
[[Web Fetcher]]'s per-host connection cap — belt and braces, because this is the rule that prevents
us from hurting anyone.

### 4.3 Adaptive politeness

`observe` adjusts the delay from real signals:

| Signal | Adjustment |
|:---|:---|
| Response latency > 2 s | `delay = max(delay, latency × 2)` — a slow host is a loaded host |
| 429 with `Retry-After` | honour exactly; open the breaker |
| 429 without `Retry-After` | `delay ×= 4`, minimum 60 s |
| 503 | `delay ×= 2`, breaker after 3 |
| 10 consecutive 200s, latency < 500 ms | `delay ×= 0.9`, floored at the configured minimum |

Delays only decay slowly and increase quickly — asymmetric on purpose.

### 4.4 Meta-robots and `X-Robots-Tag`

Checked **after** fetching (they cannot be known before). `noindex` → the document is fetched but not
indexed, and `robots_indexable = false` is recorded ([[Data Model]]). `nofollow` → outlinks are not
added to the frontier. `noarchive` → we do not retain the raw blob beyond parsing.

A `noindex` page still costs a fetch; after 3 consecutive `noindex` results the URL's revisit
interval is pushed to the maximum.

### 4.5 Exclusion blocklist

Three tiers, all consulted by `may_fetch`:

| Tier | Source | Effect |
|:---|:---|:---|
| Global | `data/crawl/blocklist.txt` | never fetched (login pages, payment endpoints, known trackers) |
| Takedown | Redis set, written by [[Admin and Source Submission]] | never fetched, never re-indexed |
| Host opt-out | a host emailing our bot contact | added to global, source disabled, confirmed by reply |

The bot info page at `/bot` documents our user-agent, our contact address, and how to block or
rate-limit us — publishing that is part of being identifiable rather than evasive
([[Web Fetcher]] §4.3).

## 5. Configuration

| Key | Default |
|:---|:---|
| `robots_cache_ttl_h` | 24 |
| `robots_fetch_timeout_s` | 10 |
| `robots_max_bytes` | 512 KiB |
| `robots_on_5xx` | `disallow` |
| `default_crawl_delay_ms` | 1500 |
| `min_crawl_delay_ms` | 500 |
| `max_crawl_delay_ms` | 300000 |
| `per_host_concurrency` | 1 |
| `respect_crawl_delay` | `true` (locked in prod, `open_web`) |
| `blocklist_path` | `data/crawl/blocklist.txt` |
| `user_agent_token` | `XustiveBot` |
| `profile_by_source_kind` | `web → open_web`; `facebook\|instagram\|tiktok → platform` |

`respect_crawl_delay` and `per_host_concurrency` are configuration in name only for the `open_web`
profile — a CI check asserts their production values, so relaxing them requires a reviewed change,
not an env var. The same check asserts that no `web`-kind source can be assigned the `platform`
profile.

## 6. Data

Redis: `robots:{host}` (parsed rules, 24 h), `crawl:{host}` (delay, last fetch, adaptive state),
`blocklist:takedown` (set). Reads the static blocklist file.

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| `robots.txt` unreachable | timeout/5xx | disallow host, retry in 1 h |
| `robots.txt` enormous | size cap | truncate |
| Malformed rules | parser | apply what parses; `WARN` |
| Redis unavailable | error | **fail closed** — no crawling without politeness state |
| Clock skew breaks delay accounting | monotonic clock | use `Instant`, never wall time |
| Host changes `robots.txt` to disallow | 24 h cache | worst-case 24 h of stale permission; on a 403 we halt immediately anyway |
| Blocklist file missing | startup | **Fatal** in prod |

The Redis fail-closed rule is the opposite of [[Deduplication Service]]'s fail-open rule, and for the
same underlying reason: dedup is a quality optimisation, politeness is a commitment.

## 8. Performance

| Metric | Budget |
|:---|:---|
| `may_fetch` (cached) | ≤ 0.5 ms p95 |
| `robots.txt` fetch + parse | ≤ 1 s p95 (once per host per day) |
| Memory | ≤ 200 MB (rules cache for ~50k hosts) |

## 9. Observability

`xustive_robots_blocked_total{reason}`, `xustive_robots_fetch_total{outcome}`,
`xustive_crawl_delay_seconds` (histogram), `xustive_adaptive_slowdown_total`,
`xustive_429_total{host_bucket}`, `xustive_noindex_total`, `xustive_blocklist_hit_total{tier}`.

A rising `xustive_429_total` is treated as **our** bug, not the host's.

## 10. Security and Compliance

**Open web** (`open_web` profile) — unchanged, and the posture we hold ourselves to:

- We identify ourselves honestly with a documented user-agent and a working contact address.
- We obey `robots.txt`, `Crawl-delay`, meta-robots, and `X-Robots-Tag`.
- We back off from 429/503 rather than routing around them.
- We honour opt-out requests within 72 h and takedowns permanently
  ([[Security and Privacy]] §8, [[Legal and Compliance]]).
- We do not access content behind paywalls.

**Platforms** (`platform` profile) — governed by
[[ADR-0009 - Direct Collection for Social Platforms]], where I accepted the contractual and legal
risk. Collection uses browser fingerprints and authenticated identities
rather than an announced bot. What that ADR did **not** change, and this component still enforces:

- The takedown blocklist applies to platform content identically — a taken-down URL is never
  re-fetched, whatever profile it came from.
- Deletion propagation, no person-centric profiling, no face recognition, and Law 18-07 duties are
  untouched by the collection method ([[Legal and Compliance]] §5).

Keeping the two profiles genuinely separate is the point of §4.0. It would be easy for the platform
stance to leak outward into general crawling; the profile resolution is config-driven, tested, and
cannot be set per request precisely to stop that.

## 11. Testing

- `robots.txt` conformance suite: RFC 9309 test cases plus real-world oddities (BOM, CRLF, duplicate
  groups, wildcards, `$` anchors, conflicting `Allow`/`Disallow`, comments mid-rule).
- Failure semantics: 404 → allow; 403 → disallow; 500 → disallow; timeout → disallow. **Each asserted
  explicitly.**
- Rate limiting: a fixture host with `Crawl-delay: 5`; assert ≥ 5 s between requests over 10 fetches.
- Concurrency: assert never more than one in-flight request per host under 50 concurrent workers.
- Adaptive: simulate 429 with and without `Retry-After`, and a slow host; assert the delay curve.
- Blocklist: a taken-down URL is never fetched again even if re-discovered via a sitemap.
- Config guard: a test asserts `respect_crawl_delay == true` and `per_host_concurrency == 1` in the
  production config file.

## 12. Open Questions

- [ ] Should we publish a public crawl-rate commitment on `/bot` (e.g. "never more than 1 request per
      1.5 s per host")? It is a promise we would then have to keep — which is the point.
- [ ] Do we honour `Crawl-delay` values above 60 s literally, or cap them and reduce crawl frequency
      instead? (A 300 s delay makes a site effectively uncrawlable.)
- [ ] How do we verify a host opt-out request is genuinely from the host owner?

## Related

[[Crawler Orchestrator]] · [[Web Fetcher]] · [[Proxy Manager]] · [[Legal and Compliance]] ·
[[Data Sources Registry]] · [[Admin and Source Submission]] · [[Error Handling and Resilience]]
