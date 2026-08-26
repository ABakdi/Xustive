---
tags:
  - solution
  - capacity
problem: PROB-001
date: 2026-08-25
status: solved
---
# PROB-001 Solution — The Bounded Frontier and Queue

> Problem: [[Problems#PROB-001 — The crawl frontier grows super-linearly and the queue Redis is a hard 1GB wall|Problems register → PROB-001]] ([../Problems.md](../Problems.md))
> Related: [[PROB-002 - Crawl and Index Throughput]] — the shared connection and pipelining here
> are its groundwork; [[PROB-003 - Admin Console Coverage]] — the capacity alarm this design needs
> an operator to see.
> Outcome: **the crawl can no longer fill Redis — every growing structure is bounded or
> self-expiring, the branching is linear, the wall is unreachable by construction, and failure is
> loud.** Verified by four new Redis integration tests plus the full suite, six clean runs.

## The design in one paragraph

The frontier is now a **working set with enforced ceilings**, not an archive with a decorative
one. Growth is cut at the source (best-K outlinks per page), bounded globally (a real ceiling that
evicts the worst rather than refusing the newest), bounded per host (a lifetime page budget), and
every "forever" structure now expires (generational seen-sets, TTL'd revisit records, a capped
due-set, a byte-sane stream cap). Above all of that sits one universal backstop: the crawler
pauses itself when Redis crosses 85% of `maxmemory` — so even a *future* unbounded structure
cannot reach the wall through the crawler. And if anything still fails, it fails **loudly**:
backend errors are distinct from "already seen", and the admin Queue page carries the capacity
alarm that did not exist when the last OOM arrived.

## What was done, mechanism by mechanism

### 1. The stream cap counts what costs: bytes, not entries
`q:index` entries are full documents, so the old `MAXLEN ~100_000` permitted ~900MB on a 1GB
instance — **86% of the actual OOM**. The cap is now `queue.index_stream_max_len` (default
**20,000** ≈ 100–200MB typical), threaded through every producer. The crawler's backpressure
(pause at 5,000 backlog) keeps the working depth far below it; the cap is the runaway backstop.
*(commit `db2afd4`)*

### 2. A real global ceiling with worst-tail eviction
`MAX_TOTAL = 5_000_000` was declared and never read. It is now `crawl.frontier_max_urls`
(default **200,000** ≈ 120–150MB), enforced in `Frontier::add`: at the ceiling, the frontier
**evicts the worst-priority URL from the fattest sampled host queue** and admits the new
discovery — the Heritrix/Nutch behaviour of dropping the least promising, never the newest. An
evicted URL's dedup entry is released, so it stays discoverable when room returns. A global
counter (maintained in the claim script and on add/promote/evict, self-healed by a heartbeat
reconcile) makes the check O(1). *(commit `20475fd`)*

### 3. Best-K outlinks: the exponent becomes linear
Each page contributed up to 200 outlinks in document order. Now every candidate is scored first
(the frontier's own priority: depth, trust, article-shape) and only the best
`crawl.max_outlinks_per_page` (default **64**) are enqueued — Nutch ships 100 as its default
defence for exactly this reason. Dropped links are counted under the `outlink_cap` skip, never
silently. *(commit `20475fd`)*

### 4. Per-host lifetime budgets
`MAX_PER_HOST` only capped *queued-at-once* — a host could push unlimited pages through over
time. `crawl.max_pages_per_host` (default **20,000**, 0 = unlimited) now counts pages actually
crawled per host at `complete()`; a spent host gets `Rejected::HostBudget` for new discoveries —
deliberately **without** marking them seen, so they return when the budget window rotates.
Revisits are unaffected: the budget gates growth, not freshness. *(commit `20475fd`)*

### 5. The seen-set stops being forever
`frontier:seen` held every full URL string ever discovered, permanently — the #1 long-run
suspect. Dedup now lives in **generational sets of 128-bit blake3 hashes**
(`{ns}:seen:{generation}`), where a generation is `crawl.seen_rotate_days` (default **45**) and
each set expires two windows after first write. Membership checks current + previous generation,
so dedup holds across the boundary; memory is bounded by *two windows of discovery*; a URL not
re-encountered within them may be crawled afresh, which the revisit scheduler and content-hash
dedup absorb. The legacy full-URL set is consulted read-only during transition (no mass
re-enqueue on upgrade) and can be deleted once confident: `DEL frontier:seen`. *(commit `20475fd`)*

### 6. Revisit records get a lifetime
`frontier:visits` was one hash growing with every URL ever fetched. Records are now per-URL keys
with a **90-day expiry** — any page still in rotation refreshes long before that (the adaptive
interval caps at 30 days). The legacy hash is read as a fallback and drained on every write, so
deployments migrate themselves. The due-set is likewise capped at the global ceiling, dropping the
farthest-future revisit when full. *(commits `a31f5f6`, `20475fd`)*

### 7. The universal backstop: the memory high-water pause
Every crawl worker checks Redis `used_memory / maxmemory` in its backpressure probe and **pauses
at 85%**, with a warn naming the numbers. This is the guarantee that survives future mistakes: no
matter what new structure starts growing, the crawler stops feeding it before the wall — where,
previously, every write failed silently and the crawl froze looking merely idle. The heartbeat
logs `redis_memory_pct` every minute. *(commit `20475fd`)*

### 8. Failure is loud and distinct
A Redis error in `add` used to be indistinguishable from "already seen" (`SADD … unwrap_or(0)`),
which converted the OOM into silent, miscounted loss. There is now a dedicated
`Rejected::Backend`, counted under `frontier_backend_error` and warned per page by the
orchestrator. *(commit `20475fd`)*

### 9. The admin alarm
The Queue page (`/admin/queue`) now shows: **frontier queued**, **deferred revisits**, and
**Redis memory %**, with a warning banner from 80% that names the numbers and the remedies. This
is the signal whose absence let the last OOM arrive with no warning anywhere in admin.
*(commit `20475fd`)*

### 10. One connection, pipelined
Found while testing: the frontier opened a **fresh TCP connection per operation** and issued 4-5
serial round trips per discovered link. It now holds one shared auto-reconnecting connection and
`add` runs in ~3 pipelined round trips — which also removes a chunk of PROB-002's Redis chatter
and fixed a real intermittent failure (connects failing under pressure surfaced as spurious
`Backend` rejections). *(commit `20475fd`)*

## The knobs (all in `[crawl]` / `[queue]`, validated at startup)

| Key | Default | Meaning |
|---|---|---|
| `crawl.frontier_max_urls` | 200,000 | global ceiling; evicts worst-priority tail at capacity (min 1,000) |
| `crawl.max_pages_per_host` | 20,000 | lifetime crawl budget per host; 0 = unlimited; window-expiring |
| `crawl.max_outlinks_per_page` | 64 | best-scoring K outlinks enqueued per page (1–200) |
| `crawl.seen_rotate_days` | 45 | dedup generation window; sets expire after 2 windows (min 7) |
| `queue.index_stream_max_len` | 20,000 | `q:index` entry cap, sized to bytes (min 1,000) |

Fixed policy (constants, deliberate): memory high-water 0.85 (`crawld`), visit TTL 90d
(`revisit.rs`), per-host queued cap 10,000 (`frontier.rs`).

## Operator runbook

- **The warning banner appears (≥80%)**: run the indexer worker to drain the backlog, or raise
  `maxmemory` in `deploy/docker-compose.yml`. The crawler pauses itself at 85% either way.
- **After upgrading an existing deployment**: the legacy `frontier:seen` and `frontier:visits`
  drain by themselves; delete `frontier:seen` manually once comfortable to reclaim its memory.
- **The 2026-08-25 incident itself** was cleared with `XTRIM q:index MAXLEN 5000` (866MB of
  already-consumed entries) — never needed again now that the cap is byte-sane.

## Deliberately not done (and why that is safe)

The Tier-3 option — a disk-backed frontier (RocksDB/sled/SQLite, the Heritrix/URLFrontier/Nutch
architecture) — remains available if the corpus ambition ever outgrows a ~200k-URL working set.
It is **not needed for the "never again" guarantee**: that guarantee comes from the ceilings plus
the memory backstop, which hold regardless of scale ambitions. Growing ambition now means raising
`frontier_max_urls` alongside `maxmemory` consciously, not silently.

## Verification

- Four new integration tests (`crates/xustive-ingest/tests/frontier_redis.rs`): the ceiling
  evicts the worst and never refuses the newest; an evicted URL is rediscoverable; a spent host
  budget refuses new URLs without poisoning them and leaves other hosts alone; generation dedup
  holds. Full frontier suite green ×6 consecutive runs; whole workspace green.
- Live: the stuck dev instance trimmed 866MB → 32MB, writes verified working; capacity fields
  confirmed on the admin endpoint.
