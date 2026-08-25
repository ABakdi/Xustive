---
tags:
  - problems
  - capacity
  - performance
date: 2026-08-25
status: open
---
# Problems Register

Structural and capacity problems — things that are working as coded but will not survive growth.
Distinct from the bug tracker: a bug is code doing the wrong thing; a problem is the design meeting
its limits. Each entry carries the evidence, the mechanics, and researched recommendations ranked
by effort. Sources: live Redis forensics on the OOM'd dev instance, a full code walk of the
frontier/queue/indexer, and the crawler literature (Mercator, Heritrix, Nutch/Common Crawl,
URLFrontier/StormCrawler, Frontera, IRLbot) plus Meilisearch's indexing guidance.

## PROB-001 — The crawl frontier grows super-linearly and the queue Redis is a hard 1GB wall

**Status: SOLVED 2026-08-25 — see [[PROB-001 - Bounded Frontier and Queue|the solution document]]
([Solutions/PROB-001 - Bounded Frontier and Queue.md](<Solutions/PROB-001 - Bounded Frontier and Queue.md>))
for the full design, knobs, and operator runbook.** Every growing structure is now bounded or
self-expiring, branching is linear (best-64 outlinks), the crawler pauses itself at 85% Redis
memory, failure is loud, and the admin Queue page carries the capacity alarm. The analysis below
is preserved as the record of what was wrong.

**Severity at discovery: high (the dev instance was at the wall — every write OOMing)**

### What actually filled the 1GB (live forensics, 2026-08-25)

| What | Size | Nature |
|---|---|---|
| `q:index` stream | **866 MB** (92,491 entries) | Each entry is a **full document JSON** (body up to 200KB). The stream *is* capped — `XADD MAXLEN ~100_000` — but the cap counts **entries while the cost is bytes**: 100k × ~9KB ≈ 900MB, i.e. the cap is ~10× too large for a 1GB instance. Nearly all entries were already consumed (`entries-read` ≈ `XLEN`, lag 4): this is dead weight awaiting displacement that stopped coming when production stalled. |
| `frontier:q:{host}` | **64 MB** across 357 host queues | The actual frontier — modest *today* (one host held 1,761 pending URLs), but unbounded tomorrow: this is the family the exponential-growth concern is about. |
| `linkgraph:*`, robots cache, interaction remnants | ~2 MB | Noise. |

So **today's OOM ≈ 86% mis-sized stream cap + 6% frontier** — but the structural concern is real
and separate: the frontier families grow without bound and would fill any ceiling given time.

### The growth mechanics (code walk, with references)

- **Branching**: up to **200 outlinks per page** (`ParseConfig::max_outlinks`, parse.rs:72) ×
  **depth ≤ 3** (orchestrator.rs:93) ≈ 8M candidate URLs per seed before dedup. Thin pages also
  enqueue their outlinks (deliberately — listing pages are walked through).
- **Per-host cap `MAX_PER_HOST = 10_000`** (frontier.rs:47) caps *queued-at-once*, not total-ever:
  claims free slots that refill, and the `defer`/`promote_due` path **bypasses the cap entirely**
  (frontier.rs:736, 804-809).
- **The global cap is dead code**: `MAX_TOTAL = 5_000_000` (frontier.rs:50) is declared and never
  referenced. There is **no global bound on the frontier**.
- **Four key families grow forever with no TTL and no sweep**: `frontier:seen` (full URL strings,
  ~150-250B each, permanent — the #1 long-run suspect), `frontier:visits` (per fetched URL),
  `frontier:due` (every fetched page is re-inserted for revisit — claim+defer is net zero, the
  frontier never shrinks under the daemon), `frontier:seen_hashes` + `linkgraph:*`.
- **Under `--discover`**, every off-site link opens a new host with its own 10k budget: growth is
  effectively unbounded. Federation injects new hosts per user search.
- **At the wall, failure is silent**: `SADD frontier:seen` OOM is `unwrap_or(0)` → indistinguishable
  from "already seen" → every discovery silently dropped and miscounted (frontier.rs:639-644); the
  claim script (contains writes) is refused → workers idle-loop 500ms forever — **the crawl
  freezes while looking merely idle**; fetched documents are warn-logged and dropped at `XADD`.

### Recommendations (ranked; the literature's consensus is "no production crawler keeps its full frontier in RAM")

**Tier 0 — unblock the stuck instance now (operator choice, minutes):**
`XTRIM q:index MAXLEN ~5000` reclaims ~800MB instantly (the entries are already consumed), or
raise `maxmemory`; then restart the worker to drain the small remainder.

**Tier 1 — cheap bounds that convert super-linear to linear (hours of work):**
1. **Byte-aware stream cap**: shrink `q:index` `MAXLEN` from 100k to ~10-20k entries *and* lower
   the crawler's `BACKPRESSURE_AT` (5,000) so stream residency stays ~50-100MB. The stream is a
   hand-off buffer, not storage.
2. **Wire the dead `MAX_TOTAL`**: on add, when the global frontier exceeds the cap, **evict the
   lowest-priority tail (`ZREMRANGEBYRANK`) instead of refusing new URLs** — bounding by dropping
   the worst, never the newest (the Heritrix/Nutch behavior).
3. **Top-K outlinks per page**: Nutch ships `db.max.outlinks.per.page = 100` as its default
   defense; cut 200 → ~64 *best* outlinks (see prioritization below) — this alone changes the
   branching curve.
4. **Per-host lifetime budget**: a `queue-total-budget` analog (Heritrix) — after N pages ever
   crawled from a host, retire it. Caps any single site's total footprint.
5. **Make OOM loud**: distinguish `SADD` error from duplicate; a metric + error log when Redis
   refuses writes, and an admin-console alert at 80% `maxmemory`. The current silence is the worst
   part of the failure mode.

**Tier 2 — memory-shape fixes (a day or two):**
6. **Hash the `seen` set**: store 16-byte URL hashes instead of full URL strings (~10× smaller),
   or adopt a **Bloom filter** (`BF.RESERVE`: ~12-18MB per 10M URLs at 0.1-1% FP; a false positive
   only means one candidate URL is skipped — harmless, most pages have many in-links). IRLbot's
   DRUM is the disk-based exact answer if FPs ever matter.
7. **Priority scoring at enqueue** instead of FIFO: score = host authority (`authority.tsv` already
   exists) + depth penalty + URL features (penalize query strings, calendar patterns). Makes the
   Tier-1 tail-eviction meaningful and is the proven-better-than-BFS ordering (Baeza-Yates;
   OPIC/AOPIC as the upgrade path). IRLbot's insight: scale per-host budget by host reputation, so
   spam farms get starved automatically.
8. **TTL/sweep the unbounded families**: `visits` entries for hosts gone 404/retired; `due`
   entries beyond a horizon; cap or relocate `linkgraph:*` (it serves offline pagerank, not the
   serving path — it can live on disk).

**Tier 3 — the structural fix (the pattern every production crawler converged on):**
9. **Move the full frontier to an embedded disk store** (RocksDB / sled / SQLite — a
   `frontier(url_hash PK, host, priority, state)` table handles tens of millions of rows on one
   box). Redis keeps only what it is good at: the Bloom seen-filter, the hot per-host ready
   queues + politeness heap (the Mercator front/back-queue design), and the capped crawl→index
   stream. This is Heritrix (BerkeleyDB), URLFrontier (RocksDB), Nutch (CrawlDb on disk,
   `generate -topN` materializes a bounded fetch list per cycle — the simplest model to retrofit),
   and Frontera (HBase) — nobody keeps the frontier in RAM. Capacity becomes disk-bounded
   (~100-1000× RAM) with crash durability for free.

## PROB-002 — Crawl and index throughput is low

**Status: open · Severity: medium (correctness unaffected; time-to-corpus suffers)**

### Where the time actually goes (code walk)

- **The ceiling is host diversity, not workers**: per-host politeness is ~1.5s
  (`DEFAULT_CRAWL_DELAY` 1500ms + 200ms margin), so throughput ≈ `min(64 workers, distinct due
  hosts) / 1.7s`. With ~20 seed hosts that is **~12 pages/s maximum no matter what** — most of the
  64 workers sleep. This matches the classic "idle crawler" diagnosis (Heritrix wiki: few active
  queues → idle threads). For calibration: IRLbot sustained ~1,790 pages/s on one 2008 server *by
  being host-diverse*, politely.
- **Serial Redis chatter per page**: each of up to 200 outlinks costs 4-5 *sequential, un-pipelined*
  round trips in `Frontier::add` (SADD, ZCARD, ZADD, HSET, ZADD) — a link-rich page can spend
  ~1,000 serial round trips (~100-200ms+) and hammers the same Redis that is fighting for memory.
- **Indexer drains in dribbles under trickle**: `XREADGROUP BLOCK 5000 COUNT 1000` returns as soon
  as *anything* exists, so light flow produces tiny batches, each paying a full
  `add_documents` + `wait_task` poll cycle against Meilisearch's single writer.
- Enrichment (OCR/embeddings) is opt-in and off by default — not a factor unless enabled.

### Recommendations (ranked)

1. **Raise distinct ready hosts — the only real lever**: more seeds, and/or discovery on *with the
   PROB-001 budgets* (per-host lifetime budget + priority scoring make `--discover` safe). This is
   the same work as PROB-001 Tier 1-2: **the frontier fix and the throughput fix are the same
   fix.** Mercator's back-queue design (one queue per host, min-heap on next-allowed-time) is the
   shape the frontier already approximates; feeding it more hosts is what scales it.
2. **Pipeline the outlink enqueue**: batch a page's accepted outlinks into one Redis pipeline (or
   one Lua call) instead of 4-5 RTs × 200 links. Order-of-magnitude fewer round trips per page.
3. **Indexer batch dwell**: when lag is high, accumulate toward `MAX_BATCH`/`MAX_BATCH_BYTES`
   before posting (short dwell, e.g. re-poll until batch full or 2s elapsed) so Meilisearch sees
   few large payloads — its documented best practice — instead of many tiny tasks. Keep settings
   applied before documents (already done), attributes minimal (already lean), and run a recent
   Meilisearch (the 2024 indexer is ~2× faster on insertion).
4. **Politeness floor per robots**: keep 1500ms as the default, but where `robots.txt` is silent
   and the host has proven fast/healthy, an adaptive floor of ~1000ms is within accepted practice;
   never parallel per host.
5. **Don't add worker processes for indexing yet**: fetching is the bound for a polite crawler;
   Meilisearch's single writer handles thousands of small docs/s in large batches. Revisit only if
   stream lag grows while fetchers are saturated.

## PROB-003 — The admin console exposes a fraction of what tunes, measures, and controls the system

**Status: open · Severity: medium (visibility/operability debt; nothing malfunctions, but the
operator flies blind on most of the surface)**

Method: one full inventory of everything that controls or measures the system (config fields,
hardcoded tuning constants, runtime switches, metrics, eval artifacts, tuning data files), one full
inventory of everything the 13 admin pages display or control, then the diff. Scale of the gap:
the backend carries **~90 config keys, ~150 tuning constants, ~27 metric families, 8 kinds of
evaluation artifact, and ~12 tuning data files**; the admin UI displays a few dozen values and
offers **12 controls** (5 runtime switches: federation, external summariser, device, GPU layers,
politeness bypass, temporary log level — plus force-crawl, seed add/remove, DLQ replay, takedown).

### A. Whole subsystems with ZERO admin visibility

1. **The evaluation loop** — the quality trajectory is invisible. `eval/reports/*.json` (nDCG/MRR
   history + the regression gate), `ab-*.json` settings A/Bs, `calibration-*.json` weight sweeps,
   `serp-*.json` Google-yardstick agreement, and the miner's `candidates-*.tsv` review files all
   exist only on disk via `make` targets. An operator cannot see "is search getting better" or
   review synonym candidates without a shell.
2. **Capacity / Redis health** — the PROB-001 OOM had **no admin signal whatsoever**: no memory
   used-vs-maxmemory for either Redis instance, no frontier size (`seen`/`due`/per-host queue
   depths), no `q:index` byte size. The queue page shows entry backlog and dead letters only.
3. **Most Prometheus metrics** — the registry exports ~27 families; the console mirrors ~6
   (federation/semantic counters). Invisible in admin: search stage durations, zero-result rate,
   degraded-stage counts, rate-limited counts, summary-withheld reasons, external-summary
   outcomes beyond ok/failed, instant-answer/translate usage, `xustive_data_age_seconds`
   (weather/tool staleness — there IS a staleness gauge, no page shows it), queue depth gauges.
   Grafana exists in dev, but the console's own story stops at the Integrations effectiveness box.
4. **The proxy/SERP layer** (breakers, EWMA health, quarantine, bandwidth alert at 80%) — fully
   built, zero UI.
5. **The expansion lexicon** — `entities.tsv`/`synonyms.tsv` (what the engine considers
   equivalent) and Meilisearch settings drift (`make migrate-check`) have no admin view.
6. **Blocklists** — the three-tier exclusion set (safety/takedown/host-opt-out) is **in-memory
   with no persistent file wired and no UI**; a takedown's "block future crawling" pairing the
   Maintenance page itself recommends cannot actually be done anywhere.

### B. Controls that exist in the backend but cannot be operated from the UI

- **No crawler pause/stop/start anywhere** — state is displayed, never controllable.
- **Registry lifecycle** (`registry.jsonl`: approve/disable, per-source frequency, depth limit,
  max-docs-per-run, crawl delay) — file-edit only; the console shows health but offers no action.
- **`WeakCoverage::forget(term)` exists in code** — no endpoint, no UI (an operator cannot dismiss
  a chased term).
- **Ranking weights** — displayed read-only (7 of 10 fields); editing means writing
  `config/ranking.toml` (a file that does not even exist in the repo) and restarting. No editor,
  no validation surface, despite the relevance-dominance invariant being checkable.
- **DLQ** — replay is all-or-nothing (1000 at a time); no per-item inspect/drop.
- **Federation tuning** (`budget_ms`, `fetch_budget_ms`, `max_hits`, `eager_index`) and every
  `[discovery]`/`[interaction]`/`[vector]`/`[stt]`/`[media]` switch — config-file-only, displayed
  at best as hints.
- **Static tool data with expiry dates** (fuel prices `REVIEW_BY 2027-03-01`, exam calendar) — no
  staleness surfacing, no reminder.

### C. Config that cannot even be *seen* from the console

There is no "effective configuration" page. An operator cannot answer basic questions without
shell access: what is `candidate_pool`? which Meilisearch index/URL is live (returned by
`GET /status` and never rendered)? what are the deadline stage budgets, rate limits, politeness
delay, revisit intervals, indexer batch sizes, k-anonymity window? The ~150 tuning constants
(EXPANSION_THRESHOLD, WEAK_TOP_SCORE, RRF K, MAX_PER_HOST, BACKPRESSURE_AT, prompt caps, …) are
of course code — but the *config* half is one serializer away from visible.

### D. Fifteen fields already returned by admin endpoints and silently dropped by the pages

`GET /status`: the whole `index` block (alias, resolved index, meili_url); `Weights.unknown_date_factor`,
`per_domain_cap`, `simhash_collapse_distance`; models' `actual_mib`; GPU name/VRAM when healthy.
Source health: `approved`, `crawlable`, `counts.failed/thin/duplicate`. Live: `parsed`,
`revisited`, `recent[].host/.at`. Queue: dead-letter `failed_at`. Media: `stt.endpoint`,
`vector.embedder_endpoint`. Enqueue response `added`/`already_known` (client types don't even
match the API shape — the operator can't tell "queued fresh" from "already known").
`add_source.queued` and `remove_source.removed` ignored (UI claims success unconditionally).
Interaction `hot_floor` used server-side, never displayed. Weak-coverage page doesn't show whether
a resolution source (SERP/Brave) is even configured.

### Recommendations (ranked)

1. **Render what already arrives** (pure frontend, hours): the fifteen dropped fields in D, and
   fix the enqueue-response type mismatch.
2. **A read-only Configuration page** (one endpoint serializing the effective `Config` with
   secrets redacted — meili_key, admin_key, brave key, salt): answers "what is this system
   running with" in one view, including env-var overrides.
3. **A capacity card on Overview** (ties to PROB-001): both Redis instances' used/max memory with
   an 80% warning, `q:index` bytes, frontier totals. This is the missing alarm that would have
   caught the OOM days early.
4. **An Evaluation page**: list `eval/reports/*` with the nDCG trend and gate status; render the
   latest calibration/A/B verdicts; a miner-candidates review table (view + copy, promotion stays
   manual per B7).
5. **Close the control gaps in operator-pain order**: crawler pause/resume; registry lifecycle
   editor (approve/disable/frequency); weak-term forget button; per-item DLQ actions; a
   ranking-weights editor that enforces the relevance-dominance bound server-side; blocklist
   manager (requires wiring blocklist persistence first — a prerequisite worth its own line).
6. **Longer term**: config *editing* for the safe subset through the existing `Config::validate()`
   so the guards (k-floors, politeness, salt) apply to UI changes exactly as to file changes.

### Sources (PROB-001/002)

Mercator (SRC-173; IR-book ch.20) · Heritrix 3 frontier budgets (`queue-total-budget`,
BdbFrontier) · Nutch CrawlDb / `generate -topN` / `db.max.outlinks.per.page=100` ·
URLFrontier (RocksDB) + StormCrawler at Common Crawl · Frontera architecture · IRLbot (budget by
in-link reputation; DRUM; 1,790 pages/s single-box) · Baeza-Yates "Crawling a Country" · OPIC
(WWW'03) · RedisBloom sizing (~9.6-14.4 bits/URL) · Redis Streams `XADD MAXLEN ~` · Meilisearch
indexing best practices (settings-first, few large batches, minimal attributes).
