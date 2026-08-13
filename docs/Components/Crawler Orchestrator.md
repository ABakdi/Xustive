---
tags:
  - component
  - ingestion
component-id: C11
binary: xustive-crawler
status: specified
updated: 2026-08-06
---

# Crawler Orchestrator

> **ID** C11 · **Binary** `xustive-crawler` · **Upstream** [[Data Sources Registry]], [[Admin and Source Submission]] · **Downstream** [[Web Fetcher]], social connectors via [[Task Queue]]

## 0. Concurrency

Workers run concurrently — sixteen by default — and **this costs no politeness at all.**

Crawl-delay is a property of a host. The frontier hands each worker a *different* host and pushes
that host's due-time forward atomically, so sixteen workers means sixteen hosts in flight while
each individual site sees exactly the one-request-at-a-time pacing it saw before.

The bottleneck was never CPU. A fetch is a few hundred milliseconds of waiting on somebody else's
server and a few milliseconds of parsing, so a sequential loop spends almost all its time idle and
one slow host stalls the whole crawl. Measured against the live seed list: **133 s sequential,
13 s with sixteen workers, for the same twenty documents.**

There is nothing here a GPU could accelerate. The work is network I/O and a little HTML parsing;
GPUs matter elsewhere in this system — embeddings and the summariser — and not at all here.

The useful ceiling is the number of **distinct hosts that are due**, not the number of cores. Past
that, extra workers find nothing to claim and idle, which is why the default tracks the seed count
rather than the CPU count. More corpus comes from more hosts, not from asking any one host faster.


## 1. Purpose

Decide **what to fetch next, and when**. It owns the frontier: the prioritised set of pending URLs
and social cursors. Everything about crawl politeness, budget, and freshness policy is decided here,
so that fetchers can be dumb, parallel, and stateless.

## 2. Responsibilities

**In scope**: seeding from the source registry; sitemap discovery; link frontier management;
per-host scheduling and budgets; revisit policy; priority assignment; backpressure response;
social-connector cursor scheduling.

**Out of scope**: actually fetching (→ [[Web Fetcher]]); robots parsing (→ [[Politeness and Robots]],
which it consults); parsing (→ [[Content Parser]]).

## 3. Interface

Not an RPC service — a loop. Inputs and outputs are Redis structures ([[Task Queue]]):

| Structure | Type | Purpose |
|:---|:---|:---|
| `frontier:{host}` | sorted set (score = due-at) | per-host pending URLs |
| `frontier:hosts` | sorted set (score = next-due) | which host to service next |
| `seen:{host}` | Bloom filter | URLs already enqueued (bounded memory) |
| `crawl:{host}` | hash | crawl-delay, last-fetch, breaker state, error counts |
| `cursor:{source_id}` | string | social pagination cursor |
| `q:fetch` | stream | dispatched work |

Admin surface: `POST /admin/recrawl {source_id | url}` injects into the frontier
([[Admin and Source Submission]]).

## 4. Internal Design

### 4.1 Main loop

```
loop {
  if queue_pressure() >= Red { sleep(5s); continue }        // [[Error Handling and Resilience]] §4
  let hosts = zrangebyscore("frontier:hosts", -inf, now, limit = 64);
  for host in hosts {
     if !politeness.may_fetch(host) { reschedule(host); continue }
     if breaker.is_open(host)       { reschedule(host, breaker.cooldown()); continue }
     let batch = pop_due(host, host_batch_size);
     for url in batch { xadd("q:fetch", envelope(url, priority)) }
     schedule_next(host, now + crawl_delay(host));
  }
}
```

One orchestrator instance is leader-elected via a Redis lock; the others idle as hot standbys. The
frontier must have exactly one scheduler or per-host rate limits become meaningless.

### 4.2 Priority

`priority = 0` (highest) … `9`. Computed as:

| Factor | Effect |
|:---|:---|
| Source `trust_tier` A / B / C | −2 / 0 / +1 |
| Source `frequency` realtime/hourly/daily/weekly | −2 / −1 / 0 / +1 |
| Depth from seed | +depth |
| Previously indexed and unchanged on last N visits | +2 |
| Manually requested recrawl | force 0 |
| URL matches a "news-shaped" pattern (`/article/`, dated path) | −1 |

### 4.3 Revisit policy (adaptive)

Each URL carries a `revisit_interval`, initialised from the source's `frequency`, then adapted:

- Content changed since last visit (`content_hash` differs) → `interval = max(min_interval, interval / 2)`
- Unchanged → `interval = min(max_interval, interval × 1.5)`
- `304 Not Modified` (we send `If-None-Match`/`If-Modified-Since`) → unchanged, and it cost us almost
  nothing
- 404/410 → remove from frontier, mark the document `gone` after 2 confirmations

Bounds: `min_interval` 30 min, `max_interval` 30 days. This converges on spending crawl budget where
content actually changes, which matters far more than raw throughput.

### 4.4 Discovery

| Path | Mechanism |
|:---|:---|
| Sitemaps | `robots.txt` `Sitemap:` + `/sitemap.xml`; parse index sitemaps recursively (cap 50k URLs/source) |
| Feeds | RSS/Atom when present — cheapest freshness signal available |
| Link extraction | [[Content Parser]] returns outlinks; orchestrator filters them |
| Social | cursor-based pagination per connector, not link following |

Outlink filters: same registered domain **or** a `.dz` TLD **or** an allowlisted Algerian-diaspora
domain; `depth ≤ depth_limit`; not in `seen`; passes `SafeUrl` ([[Security and Privacy]] §4); not
matching the global exclude patterns (`/login`, `/cart`, `?sessionid=`, calendar traps).

**Crawler traps** are handled by: a per-host URL cap, a max path depth (8), a max query-param count
(6), and a repeated-path-segment detector (`/a/b/a/b/a/b`).

### 4.5 Budgets

| Budget | Default | Scope |
|:---|:---|:---|
| `max_docs_per_run` | from source policy | per source per run |
| `max_urls_per_host_total` | 200 000 | lifetime, prevents one site consuming the index |
| `global_fetch_rate` | 100/min/worker | [[Performance Budgets]] |
| `per_host_concurrency` | 1 | never parallel-hammer one host |

## 5. Configuration

| Key | Default |
|:---|:---|
| `host_batch_size` | 20 |
| `scheduler_tick_ms` | 250 |
| `min_revisit_s` | 1800 |
| `max_revisit_s` | 2592000 |
| `depth_limit` | 3 (source-overridable) |
| `max_path_depth` | 8 |
| `sitemap_url_cap` | 50000 |
| `seen_bloom_fp_rate` | 0.001 |
| `leader_lock_ttl_s` | 30 |

## 6. Data

Reads `Source` records ([[Data Model]] §5). Owns the frontier structures in §3. Writes only to Redis
and `q:fetch`.

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Redis unavailable | command error | stop dispatching; retry with backoff; **never** crawl without the frontier |
| Leader lock lost | TTL expiry | step down immediately, stop dispatching |
| Frontier empty | size check | idle; re-seed from registry on the next cycle |
| Frontier unbounded growth | size metric | enforce `max_urls_per_host_total`, drop lowest priority |
| Queue pressure red/black | depth gauge | halve then stop dispatch ([[Error Handling and Resilience]] §4) |
| Host consistently failing | breaker | exponential cooldown; alert after 24 h |
| Crawler trap detected | pattern rules | blocklist the pattern for that host, `WARN` |
| Sitemap enormous / recursive | cap + depth guard | truncate, `WARN` |

## 8. Performance

| Metric | Budget |
|:---|:---|
| Scheduling decisions | ≥ 5 000 URLs/s (Redis-bound) |
| Dispatch latency (due → enqueued) | ≤ 1 s p95 |
| Memory | ≤ 512 MB (Bloom filters dominate) |
| Frontier size supported | ≥ 50M URLs |

## 9. Observability

`xustive_frontier_size{host_bucket}`, `xustive_frontier_hosts_due`,
`xustive_dispatch_total{source_type,priority}`, `xustive_revisit_interval_seconds` (histogram),
`xustive_trap_detected_total`, `xustive_crawl_budget_exhausted_total{source_id}`,
`xustive_leader` (gauge 0/1). Dashboard: **Ingestion** and **Crawl Politeness**
([[Observability]] §5).

## 10. Security

Every URL entering the frontier passes `SafeUrl` — including outlinks and sitemap entries, which are
attacker-controlled if a crawled site is hostile ([[Security and Privacy]] T3). Source registry
changes require admin auth. The orchestrator never executes content; it only schedules.

## 11. Testing

- Unit: priority computation table; revisit adaptation (changed/unchanged/304 sequences); trap
  detectors; outlink filters.
- Integration: a fake site (local HTTP server) with sitemap, feed, links, traps, and 404s; assert the
  frontier converges and never exceeds budgets.
- Politeness: assert never more than one in-flight request per host, and that `crawl-delay` is
  honoured within ±10 %.
- Chaos: kill the leader mid-cycle; assert a standby takes over within `leader_lock_ttl_s` and no URL
  is dispatched twice within its delay window.

## 12. Open Questions

- [ ] Do we need per-source crawl *time windows* (e.g. avoid a news site's peak hours)?
- [ ] How aggressively should we crawl `.dz` domains we did not seed — open web discovery, or
      registry-only? (Registry-only for v1; open discovery is a v2 decision with legal implications —
      [[Legal and Compliance]].)
- [ ] Should the frontier persist across a full Redis loss, or is re-seeding acceptable?

## Related

[[ADR-0011 - Adaptive Recrawl over Static Crawling]] · [[ADR-0012 - Discovery-Only Aggregation]] ·
[[Politeness and Robots]] · [[Web Fetcher]] · [[Task Queue]] · [[Data Sources Registry]] ·
[[Error Handling and Resilience]] · [[Proxy Manager]]
