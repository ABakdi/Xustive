---
tags:
  - component
  - platform
  - ingestion
component-id: C22
binary: xustive-cli
status: built
updated: 2026-08-27
---

# Politeness and Robots

> **ID** C22 · **Binary** `xustive-cli` (`crawld` subcommand; the design called it `xustive-crawler`) · **Upstream** [[Crawler Orchestrator]], [[Web Fetcher]] · **Downstream** none

## 00. Where it lives today

Audited against code 2026-08-27. The open-web profile is built and exercised on every crawl; the
`platform` profile and the blocklist tiers are not wired (see the notes in each section).

| Piece | Where |
|:---|:---|
| `robots.txt` parser (RFC 9309), `Politeness` per-host state, delay resolution, adaptive pacing | `crates/xustive-ingest/src/robots.rs` |
| Shared parsed-rules cache in Redis, `robots:v1:{authority}`, 24 h | `crates/xustive-ingest/src/robots_cache.rs` |
| Where it is applied: `Fetcher` holds `Arc<Mutex<Politeness>>` and checks before every request | `crates/xustive-ingest/src/fetch.rs` (`ensure_robots`, `wait_turn`, `observe`) |
| `X-Robots-Tag` / meta-robots / blocklist tiers (types) | `crates/xustive-ingest/src/exclusion.rs`, `parse.rs` (`ParseError::NoIndex`) |
| `CrawlConfig { respect_crawl_delay, per_host_concurrency, ignore_politeness }` + `guard` | `crates/xustive-core/src/config.rs` |
| Runtime bypass toggle `POST /api/v1/admin/politeness` | `crates/xustive-api/src/admin.rs` (`set_politeness`), banner on the [[Crawler Console]] |
| Public bot page (mirrors the crate constants) | `web/app/(operator)/bot/page.tsx`, served at `/bot` |
| Tests | `crates/xustive-ingest/tests/{robots_conformance,robots_sharing,fixture_site}.rs`; the config test that loads `config/{prod,staging,ci}.toml` through `guard` |

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

As designed, a trait with `may_fetch`/`crawl_delay`/`observe`. As built (2026-08-27) it is a plain
struct the fetcher owns behind a mutex — one lock, held across the wait, is what makes per-host
concurrency exactly one:

```rust
pub struct Politeness { … }                       // crates/xustive-ingest/src/robots.rs
impl Politeness {
    pub fn with_bypass(ignore: bool) -> Self;      // the §0 flag lives here, checked in one place
    pub fn allows(&self, host: &str, path: &str) -> bool;
    pub fn wait_for(&self, host: &str) -> Duration;   // how long until this host may be hit
    pub fn reserve(&mut self, host: &str) -> Duration; // claim the next slot
    pub fn observe(&mut self, host: &str, status: u16, retry_after: Option<Duration>);
    pub fn record_fetch(&mut self, host: &str);
    pub fn rules_stale(&self, host: &str, ttl: Duration) -> bool;
}
pub fn resolve_delay(robots: Option<Duration>, registry: Option<Duration>,
                     adaptive: Option<Duration>) -> Duration;   // the max, capped at 60 s
```

`observe` closes the adaptive loop — politeness that ignores how the host is actually responding is
just a static rate limit. Note it takes the status and `Retry-After` only; the design's latency
signal (§4.3) was not implemented. Hosts are keyed by **authority**, not bare host, so
`example.dz:8080` does not inherit `example.dz`'s rules.

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

**Status 2026-08-27:** only the `open_web` column exists in code. There is no profile resolution
from source `kind`, no `platform` path in the fetcher, and no social connector to use one — the
platform column stays the design for when [[ADR-0009 - Direct Collection for Social Platforms]]
collection is built.

The `open_web` column is unchanged from before ADR-0009 and is not negotiable in production. Most
Algerian sites run on modest hosting where an aggressive crawler is indistinguishable from an
outage — and being blocked by them is a permanent hole in the index. Changing the collection stance
toward platforms was a deliberate, bounded decision; it is not a general licence to crawl hard.

Rate limits still apply to platforms, for a different reason: there, the constraint is identity
longevity, and the budgets in [[Session Manager]] §4.5 are stricter than politeness would require.

The profile is resolved from the source's `kind` in [[Data Sources Registry]] and cannot be
overridden per request.

### 4.1 `robots.txt` *(`open_web` profile)*

- Fetched on first contact per host. Cached twice: in-process on `Politeness`, and shared in
  Redis as `robots:v1:{authority}` for **24 h** — the *source text* plus status, not the parsed
  tree, so a parser fix applies to everything already cached and a human can read why a host is
  refused. Another worker's fetch is reused rather than repeated, because fifty workers each
  requesting `robots.txt` looks, to the site, exactly like misbehaviour.
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
| > 512 KiB | truncate (`MAX_ROBOTS_BYTES`) |

Failing *closed* on 5xx is deliberate: an unavailable `robots.txt` is not permission.

### 4.2 Rate limiting

Effective delay per host, taking the maximum of:

| Source | Value |
|:---|:---|
| `robots.txt` `Crawl-delay` | as stated |
| Source registry `crawl_policy.crawl_delay_ms` | as configured |
| Global default | 1 500 ms (`DEFAULT_CRAWL_DELAY`) |
| Adaptive component | §4.3 |

The result is capped at **60 s** (`MAX_CRAWL_DELAY`): past that a site is effectively uncrawlable,
and the honest response is to visit it rarely rather than park a worker (this answers the open
question in §12). The registry column is `resolve_delay`'s second argument; whether a per-source
`crawl_delay_ms` is actually passed from [[Data Sources Registry]] could not be confirmed on
2026-08-27 — see §12.

Concurrency per host is **1**, always. Enforced by [[Crawler Orchestrator]]'s per-host scheduling and
[[Web Fetcher]]'s per-host connection cap — belt and braces, because this is the rule that prevents
us from hurting anyone.

### 4.3 Adaptive politeness

`observe` adjusts the delay from real signals:

| Signal | Adjustment (as built) |
|:---|:---|
| 429 with `Retry-After` | honour it, clamped to 60 s … 600 s; streak reset |
| 429 without `Retry-After` | `delay ×= 4`, clamped to 60 s … 600 s; streak reset |
| any 5xx | `delay ×= 2`, ceiling 300 s; streak reset |
| 2xx / 3xx | `delay ×= 0.9` down to a floor: the declared `Crawl-delay` if any; else **1 000 ms once 20 consecutive clean responses have been earned** (`EARNED_MIN_DELAY`, `HEALTHY_STREAK`, PROB-002); else the 1 500 ms default |

Delays only decay slowly and increase quickly — asymmetric on purpose. *Superseded 2026-08-27:* the
design's latency-doubling rule and the "breaker after 3" on 503 were not implemented — a single
error resets the earned streak, which is the same intent with less machinery. The earned floor was
added in [[PROB-002 - Crawl and Index Throughput]]: a robots-silent host that behaves may be paced at
one request per second, squarely within accepted practice, and re-proves it continuously.

### 4.4 Meta-robots and `X-Robots-Tag`

Checked **after** fetching (they cannot be known before). `X-Robots-Tag` is read in the fetcher
(`exclusion::from_header`, honouring both `*` and a `xustivebot:` prefix — the header matters as much
as the tag because a PDF or JSON endpoint has no `<head>`); the meta tag surfaces from the parser as
`ParseError::NoIndex`. Either way the orchestrator counts the skip (`robots` / `x_robots_tag`) and
the document is not indexed. `nofollow` → outlinks are not added to the frontier. `noarchive` → we do
not retain the raw blob beyond parsing.

A `noindex` page still costs a fetch; after 3 consecutive `noindex` results the URL's revisit
interval is pushed to the maximum.

### 4.5 Exclusion blocklist

Three tiers, designed to be consulted before every fetch. **Status 2026-08-27: the tiers exist as
a type (`exclusion::Blocklist` with `Tier::{Global, Takedown, HostOptOut}`) but nothing constructs
or consults one on the crawl path, and there is no `data/crawl/blocklist.txt`.** Today a takedown is
`xustive-cli takedown --domain <d>` (deletes indexed documents) plus `registry disable`, per
[[Runbooks]] — which stops re-crawl only through the source registry, not from another discovery
channel. Wiring a persisted takedown tier into the fetcher is the outstanding follow-up.

| Tier | Source | Effect |
|:---|:---|:---|
| Global | `data/crawl/blocklist.txt` | never fetched (login pages, payment endpoints, known trackers) |
| Takedown | Redis set, written by [[Admin and Source Submission]] | never fetched, never re-indexed |
| Host opt-out | a host emailing our bot contact | added to global, source disabled, confirmed by reply |

The bot info page at `/bot` documents our user-agent, our contact address, and how to block or
rate-limit us — publishing that is part of being identifiable rather than evasive
([[Web Fetcher]] §4.3).

## 5. Configuration

What is actually configurable (`[crawl]` in `config/*.toml`, `CrawlConfig`):

| Key | Default |
|:---|:---|
| `respect_crawl_delay` | `true` (locked in prod/staging by `guard`) |
| `per_host_concurrency` | 1 (locked in prod/staging by `guard`) |
| `ignore_politeness` | `false` (refused in prod/staging by `guard`) |

Everything else the design listed as a key is a **constant** in `robots.rs`, changed by a reviewed
code change rather than an env var — which is the stricter of the two and the intent of the CI check
below: `TTL` 24 h, `MAX_ROBOTS_BYTES` 512 KiB, `DEFAULT_CRAWL_DELAY` 1 500 ms, `EARNED_MIN_DELAY`
1 000 ms, `MAX_CRAWL_DELAY` 60 s, `USER_AGENT` `XustiveBot/1.0 (+https://xustive.dz/bot; Algerian
search engine)`, `UA_TOKEN` `xustivebot`, and the fetcher's `politeness_margin` 200 ms. No
`blocklist_path`, `robots_on_5xx` or `profile_by_source_kind` exists (§4.0, §4.5).

`respect_crawl_delay` and `per_host_concurrency` are configuration in name only for the `open_web`
profile — a test in `config.rs` loads the shipped `config/prod.toml`, `staging.toml` and `ci.toml`
through `CrawlConfig::guard`, so relaxing them requires a reviewed change, not an env var. (The
profile-assignment half of that check has nothing to assert yet — §4.0.)

## 6. Data

Redis: `robots:v1:{authority}` (source text + status, 24 h). Per-host delay, last-fetch and
adaptive state are **in-process** on each worker's `Politeness`, not in Redis — so an adaptive
slowdown learned by one worker is not seen by another. No `blocklist:takedown` set and no static
blocklist file exist (§4.5).

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| `robots.txt` unreachable | timeout/5xx | disallow host, retry in 1 h |
| `robots.txt` enormous | size cap | truncate |
| Malformed rules | parser | apply what parses; `WARN` |
| Redis unavailable | error | *superseded 2026-08-27:* the shared cache **falls through to fetching `robots.txt` directly** — fail-open on the cache, never on the rules. "No rules cached, therefore disallow everything" would turn a cache outage into a silent halt that looks like the crawler having nothing to do. The safety property survives because the fallback is fetching properly, not skipping |
| Clock skew breaks delay accounting | monotonic clock | use `Instant`, never wall time |
| Host changes `robots.txt` to disallow | 24 h cache | worst-case 24 h of stale permission; on a 403 we halt immediately anyway |
| Blocklist file missing | startup | **Fatal** in prod — *not built; no blocklist file exists (§4.5)* |

The original note said the opposite of what shipped — Redis-down fail-closed. The reasoning that
won: the cache is an optimisation, the rules are the commitment, and falling through *to the rules*
keeps the commitment. (Compare [[Deduplication Service]], which fails open for a weaker reason.)

## 8. Performance

| Metric | Budget |
|:---|:---|
| `may_fetch` (cached) | ≤ 0.5 ms p95 |
| `robots.txt` fetch + parse | ≤ 1 s p95 (once per host per day) |
| Memory | ≤ 200 MB (rules cache for ~50k hosts) |

## 9. Observability

As built, the crawler publishes **skip counters** through `crawl_stats` to Redis for the
[[Crawler Console]] — `robots`, `x_robots_tag`, and the rest of the skip vocabulary — plus a
"recent" feed naming the URL and reason; the bypass state is shown as a banner. None of the
Prometheus series the design named (`xustive_robots_blocked_total`, `xustive_429_total`, …) are
emitted. A rising count of 429s in the recent feed is still to be read as **our** bug, not the host's.

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

- `robots.txt` conformance suite — `tests/robots_conformance.rs`, written against RFC 9309 rather
  than against the implementation: BOM, CRLF, duplicate groups, wildcards, `$` anchors, conflicting
  `Allow`/`Disallow`, comments mid-rule. Sharing across workers: `tests/robots_sharing.rs`.
- Failure semantics: 404 → allow; 403 → disallow; 500 → disallow; timeout → disallow. **Each asserted
  explicitly.** (`fixture_site.rs` runs the whole crawl against the fixture site with the bypass.)
- Rate limiting: a fixture host with `Crawl-delay: 5`; assert ≥ 5 s between requests over 10 fetches.
- Concurrency: assert never more than one in-flight request per host under 50 concurrent workers.
- Adaptive: simulate 429 with and without `Retry-After`, and a slow host; assert the delay curve.
- Blocklist: a taken-down URL is never fetched again even if re-discovered via a sitemap.
- Config guard: a test asserts `respect_crawl_delay == true` and `per_host_concurrency == 1` in the
  production config file.

## 12. Open Questions

- [ ] Should we publish a public crawl-rate commitment on `/bot` (e.g. "never more than 1 request per
      1.5 s per host")? It is a promise we would then have to keep — which is the point.
- [x] ~~Do we honour `Crawl-delay` values above 60 s literally?~~ Capped at 60 s (`MAX_CRAWL_DELAY`);
      the `/bot` page says so.
- [ ] Wire `exclusion::Blocklist` (takedown + host opt-out tiers, persisted) into the fetcher (§4.5).
- [ ] Confirm the registry's per-source delay reaches `resolve_delay`'s `registry` argument.
- [ ] How do we verify a host opt-out request is genuinely from the host owner?

## Related

[[Crawler Orchestrator]] · [[Web Fetcher]] · [[Proxy Manager]] · [[Legal and Compliance]] ·
[[Data Sources Registry]] · [[Admin and Source Submission]] · [[Error Handling and Resilience]] ·
[[Crawler Console]] · [[Runbooks]] · [[PROB-002 - Crawl and Index Throughput]]
