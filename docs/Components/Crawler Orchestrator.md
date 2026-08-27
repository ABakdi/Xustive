---
tags:
  - component
  - ingestion
component-id: C11
binary: xustive-cli crawld
status: built
updated: 2026-08-27
---

# Crawler Orchestrator

> **ID** C11 · **Binary** `xustive-cli crawld` (`crates/xustive-cli/src/crawld.rs`) · **Library**
> `crates/xustive-ingest` (`orchestrator.rs`, `frontier.rs`, `revisit.rs`, `sitemap_poll.rs`) ·
> **Upstream** [[Data Sources Registry]], [[Admin and Source Submission]] · **Downstream**
> [[Web Fetcher]] in-process, [[Indexer Worker]] via `q:index` ([[Task Queue]])

## 0. Concurrency

Workers run concurrently — **sixty-four by default** (`DEFAULT_WORKERS`) — and **this costs no
politeness at all.**

Crawl-delay is a property of a host. The frontier hands each worker a *different* host and pushes
that host's due-time forward atomically, so sixty-four workers means sixty-four hosts in flight
while each individual site sees exactly the one-request-at-a-time pacing it saw before.

The bottleneck was never CPU. A fetch is a few hundred milliseconds of waiting on somebody else's
server and a few milliseconds of parsing, so a sequential loop spends almost all its time idle and
one slow host stalls the whole crawl. Measured against the live seed list: **133 s sequential,
13 s with sixteen workers, for the same twenty documents.** Sixteen was then the ceiling once
discovery had spread the frontier over hundreds of hosts — sixteen hosts against a 1.5 s delay is
~9 fetches/s no matter how much is queued — so the default rose to sixty-four (PROB-002). An idle
worker is a parked async task; overshooting the due-host count costs almost nothing.

There is nothing here a GPU could accelerate. The work is network I/O and a little HTML parsing;
GPUs matter elsewhere in this system — embeddings and the summariser — and not at all here.

## 1. Purpose

Decide **what to fetch next, and when**. It owns the frontier: the prioritised set of pending URLs
in Redis. Everything about crawl politeness, budget, and freshness policy is decided here, so that
the loop itself can be dull: claim a URL, fetch it, parse it, hand the document on, queue the links
it points to, repeat. A crawler runs for months unsupervised; every clever scheduling decision is
one somebody reconstructs at two in the morning from a document count that stopped rising.

## 2. Responsibilities

**In scope**: seeding from the seed TSV and the registry's approved, active sources; link
frontier management; per-host scheduling, budgets and trap rules; revisit policy; sitemap-driven
freshness; hot-document recrawl; query-driven discovery; backpressure and memory backstops.

**Out of scope**: the HTTP itself (→ [[Web Fetcher]]); robots parsing (→ [[Politeness and Robots]],
which it consults); parsing (→ [[Content Parser]]); indexing (→ [[Indexer Worker]]). Social
cursors: **not built** (2026-08-27) — the connectors do not exist.

## 3. Interface

Not an RPC service — a loop, `Orchestrator::step(now_ms) -> Outcome`, run by every worker task.
`Outcome` is `Document(Parsed)`, `Idle` (back off `IDLE_SLEEP` = 500 ms) or `Finished` (the
`--max` budget is spent). The CLI `crawl` command writes documents to Meilisearch directly; `crawld`
puts them on `q:index`, so an index outage costs a backlog rather than a stopped crawl and a set of
pages fetched, politely, and thrown away.

Frontier structures in Redis (namespace `frontier`; the discovery tools use their own namespace so
they cannot wipe each other's state):

| Key | Type | Purpose |
|:---|:---|:---|
| `frontier:hosts` | sorted set (score = due unix ms) | which host may be touched next |
| `frontier:q:{host}` | sorted set (score = priority, low first) | per-host pending URLs |
| `frontier:seen:{gen}` | set of 128-bit URL hashes | URL dedup, generational, expires after two windows |
| `frontier:inflight` | hash url → claim expiry | so a dead worker's work returns |
| `frontier:due` | sorted set (score = revisit due) | pages we hold and have booked a return to |
| `frontier:count`, `frontier:hostpages:{host}`, `frontier:meta` | counters | global ceiling, per-host lifetime budget |

Admin surface: `POST /api/v1/admin/crawler/enqueue` and `/pause` ([[Crawler Console]]).

## 4. Internal Design

### 4.1 The loop and the claim

```
every 30 s   reclaim expired claims; promote pages that came due (≤ 500 per sweep)
every 2 s    guard probe: indexer backlog, operator pause flag, Redis memory
claim(now, host_delay)   ZRANGEBYSCORE hosts ≤ now → pop the best URL of that host,
                         push the host's due-time forward, record the claim
fetch → parse → Document; complete(url); add_batch(best K outlinks)
```

There is no leader election. Politeness is shared state in Redis and claiming is atomic, so any
number of workers — in one process or several — can run the same loop. Claims carry a 120 s TTL
(`CLAIM_TTL`, comfortably above the fetch timeout) and are reclaimed rather than released: a worker
that dies cannot release anything. At-least-once, so fetching must be idempotent — a page fetched
twice costs a request; losing it costs a document forever.

### 4.2 Priority

Lower sorts first; `priority_for(depth, trust, looks_like_article)`:

| Factor | Effect |
|:---|:---|
| Depth from seed | `+1000 × depth` — depth dominates; shallow-first keeps the crawl broad |
| Source trust 0–100 | `−5 × trust` |
| Article-shaped path | `−250`, breaks ties against listing pages |

Revisits use `priority_for_revisit`: the same base, minus a banded credit for how often the page
has been measured to change (400 at ≤ 2 h intervals, 200 at ≤ 1 day, 50 at ≤ 1 week) and one
point per hour overdue, capped at 300. Both caps keep the two signals as adjustments to an
ordering rather than an ordering of their own.

### 4.3 Revisit policy — [[ADR-0011 - Adaptive Recrawl over Static Crawling]]

`revisit.rs`. Each page held has a `Visit` (content hash, interval, validators). "Changed" means
the BLAKE3 hash of the *extracted* body differed, so an APS or Echorouk page that differs
byte-for-byte on every fetch while the article never moves is *unchanged*.

- Changed → `interval = max(floor, interval / 2)`
- Unchanged, or `304 Not Modified` from a conditional request → `interval += floor` (additive; the
  first version multiplied by 1.5 and the freshness evaluation measured it as *worse* than a fixed
  interval — AIMD converges on the largest interval that still catches the change)
- Four consecutive changes at the floor → `Volatile`: left alone, per Cho & Garcia-Molina, because a
  page that changes faster than we can visit can never be kept fresh
- 404/410 → `gone`; the frontier forgets the page

Bounds are tiered by trust (`Bounds::for_trust`): ≥ 80 → 1 h to 3 days; 50–79 → 2 h to 14 days;
below → 6 h to 30 days. Without tiering a large quiet corpus drags every interval to the ceiling
and the sources that matter go stale with the rest.

### 4.4 Discovery and freshness

| Path | Mechanism |
|:---|:---|
| Links | [[Content Parser]] returns outlinks; the **best-scoring K** (`max_outlinks_per_page`, 64) enter the frontier, not the first K |
| Sitemaps | `{scheme}://{host}/sitemap.xml` per seed host, polled every **6 h** (`sitemap_poll.rs`): a page the sitemap says changed is deferred as due now; one it says unchanged takes a free "unchanged" observation — one fetch stands in for hundreds of revisits |
| Hot recrawl | Every 30 min, up to 200 frequently-clicked pages are deferred into the frontier as `source_id: "hot"`; only when `interaction.enabled` ([[Interaction Signals]], M6) |
| Query-driven | Every 5 min, weak-coverage terms are resolved to URLs ([[ADR-0013 - Direct SERP Collection for Discovery]]) |
| Common Crawl | `xustive-cli commoncrawl` bootstraps from a CDX index; Brave/SERP via `xustive-cli discover` |
| Social | **not built** (2026-08-27) |

Outlink filters: same host unless `--discover` is set (the difference between crawling twenty
sources and crawling the web, made explicitly); `depth ≤ 3`; not in `seen`; passes `SafeUrl`;
survives the trap detectors. `canonical()` normalises conservatively — only what provably does not
change the response is removed, because `?page=2` is not tracking.

**Crawler traps** (`detect_trap`): more than 12 path segments, more than 8 query parameters, or
one segment repeated more than 3 times (`/a/b/a/b/a/b`). Every rejection is counted under its
name (`seen`, `off_site`, `not_permitted`, `trap`, `full`, `too_deep`, `host_budget`,
`frontier_backend_error`) so "the crawler is collecting nothing" resolves to *which* rule.

### 4.5 Budgets — the frontier is a working set, not an archive (PROB-001)

| Bound | Default | Enforced where |
|:---|:---|:---|
| `crawl.frontier_max_urls` | 200 000 | at the ceiling the **worst-priority tail is evicted** to admit the new discovery |
| `MAX_PER_HOST` | 10 000 queued at once | new discoveries for that host dropped |
| `crawl.max_pages_per_host` | 20 000 lifetime (0 = unlimited) | counted at `complete()`, checked at `add()`, expires over ~2 seen rotations |
| `crawl.max_outlinks_per_page` | 64 | branching factor |
| `crawl.seen_rotate_days` | 45 | seen sets are generational, so "seen" is bounded by two windows |
| `BACKPRESSURE_AT` | 5 000 on `q:index` | pause claims 10 s at a time until the indexer catches up |
| `MEMORY_HIGH_WATER` | 0.85 of Redis `maxmemory` | the universal backstop — stop producing before every write fails |

The last two and the operator pause are the queue side of the same story; [[Task Queue]] covers
them from the consumer's end.

The predecessor of `FrontierLimits` was a `MAX_TOTAL` constant that was declared, documented and
never read, which is how the frontier grew into a hard Redis memory wall. Every bound above is
read. `Rejected::Backend` is kept distinct from every quota answer because at the wall the old code
read each failure as "already seen" and the crawl died silently.

## 5. Configuration

`[crawl]` in `config/*.toml` (see §4.5) plus `raw_ttl_days`, `ignore_politeness` (testing only,
refused in production by the config guard), `seeds_path`, `registry_path`. CLI: `--workers`,
`--max`, `--discover`, `--reset`, `--registry`. The rest are constants in `crawld.rs` and
`orchestrator.rs`, named above; none has needed a knob yet.

## 6. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Redis unavailable | command error | `crawld` refuses to start; a running worker's adds fail as `Backend` and are counted, never mistaken for "seen" |
| Worker dies mid-fetch | claim TTL | reclaimed within 120 s by the next sweep |
| Index backlog deep | `q:index` length | claims pause 10 s at a time; frontier untouched |
| Redis past high water | `memory_usage()` | crawl pauses until it drains; logged |
| Operator pause | Redis flag | claims stop within ~2 s; in-flight finishes |
| Frontier at ceiling | `count` | worst tail evicted, counted |
| Trap | rules | rejected, counted under `trap` |
| Sitemap unreachable or huge | poll | empty outcome, capped at 5 000 entries; freshness must never stop the crawl |
| `SIGTERM` / Ctrl-C | signal | drain: in-flight fetch finishes, its document is queued, frontier left intact |

## 7. Observability

`Snapshot` counters in Redis feed the [[Crawler Console]] and Prometheus; a heartbeat logs progress
every 60 s because an unattended process that logs nothing is indistinguishable from a stopped
one. Skips are keyed by rule name. Per-source and per-channel yield counters back the sources
health and channels pages.

## 8. Security

Every URL entering the frontier passes `SafeUrl` — including outlinks and sitemap entries, which
are attacker-controlled if a crawled site is hostile ([[Security and Privacy]]). The orchestrator
never executes content; it only schedules. Admin enqueues are seeds, not exceptions.

## 9. Testing

`crates/xustive-ingest/tests`: `frontier_redis.rs` (claims, bounds, eviction, rotation),
`freshness_eval.rs` (the policy comparison that chose AIMD), `fixture_site.rs` (a local site with
links, traps and 404s), `robots_conformance.rs`, `ssrf.rs`, `crawl_stats_redis.rs`; unit tests for
priority, trap detection and canonicalisation in `frontier.rs`.

## 10. Open Questions

- [ ] Per-source crawl *time windows* (avoid a news site's peak hours)?
- [ ] Open-web discovery of `.dz` hosts we did not seed is behind `--discover`; the legal framing is
      [[Legal and Compliance]].
- [ ] Robots-declared `Sitemap:` URLs for the poller (only `/sitemap.xml` is polled today).

## Related

[[ADR-0011 - Adaptive Recrawl over Static Crawling]] · [[ADR-0013 - Direct SERP Collection for Discovery]] ·
[[ADR-0012 - Discovery-Only Aggregation]] · [[Politeness and Robots]] · [[Web Fetcher]] ·
[[Task Queue]] · [[Data Sources Registry]] · [[Crawler Console]] · [[Error Handling and Resilience]] ·
[[Proxy Manager]] · [[Problems]]
