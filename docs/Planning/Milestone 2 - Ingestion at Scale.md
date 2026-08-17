---
tags:
  - planning
  - milestone
milestone: 2
status: not-started
updated: 2026-08-06
---

# Milestone 2 - Ingestion at Scale

> **Goal:** stop using fixtures. Crawl the real Algerian web politely, ingest whatever social access
> we have actually been granted, and get to 1M documents.
> **Exit gate:** 1M documents indexed; politeness verified under load; whatever social connectors are
> authorised are live; takedown path works end to end.
> Parent: [[TODO]] · Previous: [[Milestone 1 - Text Search MVP]] · Next: [[Milestone 4 - Quality and Operations]].

---

## ⚠ The Critical Path Changed

Under [[ADR-0009 - Direct Collection for Social Platforms]], social ingestion is no longer gated on
platform approvals or admin outreach. The critical path is now **the collection layer**
(M2-T01a/b/c): identities, fingerprints, signatures. Nothing social works until those three do.

Two things that have **not** changed:

1. **Identity warm-up takes 10+ days of wall-clock time** ([[Session Manager]] §4.4) and cannot be
   compressed by adding people. Account acquisition and warm-up must start during
   [[Milestone 1 - Text Search MVP]], or the pool will not be `mature` when the connectors are ready.
2. **Open-web crawling stays fully polite.** M2-T02 is unchanged, and it is still the majority of
   traffic ([[Politeness and Robots]] §4.0).

The honest framing: **web crawling is an engineering problem; social collection is an engineering
problem with a permanent maintenance tail.** Platforms will break these paths repeatedly, without
notice. Budget for it as ongoing work, not a project that finishes.

---

## M2-T01a — [[Session Manager]] ★ *start during M1 — warm-up is wall-clock*

- [x] M2-T01a.1 Identity record; encrypted credential/cookie storage (XChaCha20-Poly1305) —
      `session::crypto`, `session::Identity`; nonce-per-seal, plaintext zeroised on drop
- [x] M2-T01a.2 **Pinning invariant** — `SessionLease` carries the pinned proxy+fingerprint with no
      setter, so no code path can rotate within an identity (`session::pool`)
- [x] M2-T01a.3 Lifecycle: fresh → warming → mature → quarantined → burned (`session::lifecycle`)
- [ ] M2-T01a.4 Warm-up scheduler with human-shaped browsing patterns — needs real accounts + browsing
- [x] M2-T01a.5 Per-identity budgets with jitter and diurnal shaping (`session::budget`; window offset
      per identity)
- [ ] M2-T01a.6 Login flows + TOTP; operator queue for anything needing a human — needs real accounts
- [x] M2-T01a.7 Challenge detection (captcha, checkpoint, login wall, rate limit) (`session::detection`)
- [x] M2-T01a.8 **Silent-cloaking detection**: canaries + `consecutive_empty` thresholds — canary
      disagreement separates soft-ban from a platform change (`session::detection`)
- [x] M2-T01a.9 Quarantine/recovery with doubling cooldown; burn after 3 (`session::lifecycle`)
- [x] M2-T01a.10 Fail-closed budget accounting across Redis restarts — `session::BudgetStore`:
      per-period counters, denies on an unreachable Redis, and detects a flush via a durable
      sentinel (absent sentinel → deny until an operator re-initialises). Verified against Redis
- [ ] M2-T01a.11 **Account acquisition and pool sizing** ← procurement; start in M1
- [x] M2-T01a.12 Pool exhaustion halts the platform (never degrade to unpinned identities) —
      `Exhausted` vs `NoneAvailableNow` distinguished (`session::pool`)

*Decision logic done and unit-tested; the open items (.4, .6, .10, .11) need real accounts, live
browsing, or the Redis budget-counter layer, not more pure logic.*

## M2-T01b — [[Fingerprint Engine]]

- [ ] M2-T01b.1 Select the impersonation library (`rquest` vs alternatives); validate JA4 accuracy —
      needs the library + a JA4 echo endpoint
- [~] M2-T01b.2 Profile schema and catalogue — schema done (`fingerprint::Profile`, TOML-loadable);
      4 real coherent profiles seeded (`data/fingerprints/`), ~8–16 more for a human to add to hit 12–20
- [ ] M2-T01b.3 TLS + HTTP/2 + header-order profiles wired into the client — needs the impersonation lib
- [x] M2-T01b.4 **Coherence test suite** — every §4.2 invariant checked mechanically
      (`fingerprint::coherence`), with a catalogue CI test asserting every shipped profile passes
- [ ] M2-T01b.5 Headless CDP patch scripts; real Chrome, persistent per-identity profile — needs Chrome
- [x] M2-T01b.6 WebRTC forced through proxy; leak assertion — `WebRtc` has no `Direct` variant, so a
      profile can only disable or proxy WebRTC; the coherence model forbids a leaking configuration
- [ ] M2-T01b.7 Distribution weights matching the real Algerian browser mix — needs market-share review
- [x] M2-T01b.8 Version ageing and successor migration — `is_retired` + `can_migrate_to` (same
      browser+OS, newer version only)
- [ ] M2-T01b.9 `make fp-verify` self-validation against echo endpoints, nightly — needs live endpoints
- [ ] M2-T01b.10 Dependency pinning so a library bump cannot silently change our fingerprint — with the lib

*Coherence engine + schema + seed catalogue done and CI-tested; the open items need the impersonation
library, a real headless Chrome, or live echo endpoints.*

## M2-T01c — [[Signature Service]]

- [ ] M2-T01c.1 Choose the JS runtime (`deno_core` / `rusty_v8` / `boa`) — prototype against a real
      obfuscated bundle before committing
- [ ] M2-T01c.2 Sandboxed isolate: no net, no fs, 50 ms cap, 32 MB heap, recycling
- [ ] M2-T01c.3 `navigator` / `window` / `screen` shim fed from the identity's fingerprint profile
- [ ] M2-T01c.4 Signer extraction pipeline (AST-based, human-confirmed); snapshots committed to git
- [ ] M2-T01c.5 Session-constant harvesting (`fb_dtsg`, `lsd`, `X-IG-WWW-Claim`, `doc_id`)
- [ ] M2-T01c.6 Token cache with TTLs; `X-IG-WWW-Claim` response propagation
- [ ] M2-T01c.7 Rotation detection → halt platform + page; daily bundle hash diff
- [ ] M2-T01c.8 **Fallback ladder**, including the embedded-JSON path that needs no signing
- [ ] M2-T01c.9 UA-coherence test (signed UA == transmitted UA)

## M2-T01d — Legal and obligations *(reduced scope)*

No longer a blocker on the connectors ([[Legal and Compliance]]), but still real:

- [ ] M2-T01d.1 Establish the legal entity (prerequisite for takedowns and submissions)
- [ ] M2-T01d.2 Clarify Law 18-07 obligations; ANPDP notification if required
- [ ] M2-T01d.3 Record my risk acceptance, dated
      ([[ADR-0009 - Direct Collection for Social Platforms]])
- [ ] M2-T01d.4 Residential proxy provider due diligence — exit-node consent ([[Proxy Manager]] §10)
- [ ] M2-T01d.5 Opportunistic API authorisations where a source offers one — cheaper and more stable
      than collecting, worth taking when it is free

## M2-T02 — [[Politeness and Robots]]

- [x] M2-T02.1 RFC 9309 parser with longest-match, wildcards, `$` — verified by the conformance
      suite below rather than by the unit tests, which were written from the implementation
- [x] M2-T02.2 Fetch-failure semantics: 404 → allow; 403/5xx/timeout → **disallow** — was already
      implemented and is now asserted, as a table and over real HTTP against a server returning
      each status. The failure here is silent: a crawler reading a 403 as "no restrictions"
      behaves impeccably in testing and crawls a site that refused it
- [x] M2-T02.3 24 h cache in Redis; `Sitemap:` extraction handed to the orchestrator — the cache
      stores the **source text**, so a parser fix applies to everything already cached and a human
      can read an entry to see why a host is refused. Fails **open**: a Redis outage means fetching
      `robots.txt` directly, never "no rules cached, therefore disallow". Verified by counting the
      requests the *site* saw — two fetchers, one request
- [x] M2-T02.4 Crawl-delay resolution (max of robots / registry / default / adaptive) — the
      **maximum**, because each source is a floor set by someone with a reason. Taking the minimum
      would let a config change silently undo a `Crawl-delay` the site asked for, and a reduced
      delay is indistinguishable from ignoring it
- [x] M2-T02.5 Adaptive slowdown from latency, 429, 503 signals — asserted rather than assumed:
      a 429 more than doubles the pace, `Retry-After` is obeyed when named, one success does not
      erase the backoff, and forty consecutive 429s stay bounded so a hostile host cannot park a
      worker indefinitely
- [x] M2-T02.6 Meta-robots and `X-Robots-Tag` post-fetch handling — the meta tag was already
      honoured; the **header was not**, which meant honouring the request only on documents that
      have a `<head>` and ignoring it on a PDF or an image, where the header is the site's only
      option. Proven over real HTTP, including a repeated header line and a directive addressed to
      another crawler
- [x] M2-T02.7 Three-tier blocklist (global, takedown, host opt-out) — separate tiers because
      they are answerable to different people: withdrawing a host's opt-out must not lift a legal
      takedown. Subdomain matching, or a blocklist is evadable by the blocked site itself
- [x] M2-T02.8 `/bot` page: user-agent, contact, how to block or rate-limit us — pasteable rules
      rather than prose, served from the API so it answers when the UI is not deployed. A test
      asserts the token on the page is the one the parser actually matches; a page naming the wrong
      token would look correct and block nothing
- [x] M2-T02.9 **Config guard test**: prod has `respect_crawl_delay = true`, `per_host_concurrency = 1`
      — and `ignore_politeness = false`. The guard runs at startup and the binary **refuses to
      boot** rather than warning; a test loads the shipped `prod.toml` and `staging.toml` through
      it, since a guard the deployed config never passes through proves only that it compiles
- [x] M2-T02.11 **Testing bypass** (`crawl.ignore_politeness`, off by default) — ignores robots,
      delays, adaptive slowdown and host opt-outs; never the takedown or global blocklists. Admin
      toggle with a banner, `warn` log naming the peer, and refused outside development. Proven
      against the fixture site in both directions
- [x] M2-T02.10 Conformance suite including BOM, CRLF, duplicate groups, conflicting rules — 27
      cases written against RFC 9309 rather than against the code. Found a **panic**: the 512 KiB
      cap sliced the file at a byte offset, so an Arabic `robots.txt` long enough to reach it
      crashed the parser, and a site could have done that deliberately

## M2-T03 — [[Crawler Orchestrator]]

- [~] M2-T03.1 Frontier structures in Redis; leader election with a lock — frontier done and
      namespaced (two crawls can share one Redis). Claiming is a single Lua script: as separate
      round trips, two workers routinely pick the same host between the read and the write and
      both fetch it. Asserted with 16 concurrent workers, zero duplicate claims. **Leader election
      still open**, though atomic claiming makes it an optimisation rather than a correctness need
- [ ] M2-T03.2 Scheduling loop with per-host due-times
- [~] M2-T03.3 Priority computation — depth dominates; trust and article-shape only break ties, or
      one trusted source swallows the crawl. **Reopened**: `orchestrator.rs` pins `depth = 1`, so
      every URL scores identically and `max_depth` never fires. Superseded by M2-T15.7
- [ ] M2-T03.4 Adaptive revisit intervals (changed / unchanged / 304) → **specified in M2-T15**
- [ ] M2-T03.5 Sitemap and feed discovery with caps — also the highest-yield freshness signal, see
      M2-T15.6
- [ ] M2-T03.6 Outlink filtering and `SafeUrl` validation
- [x] M2-T03.7 Crawler-trap detectors (depth, params, repeating segments) — plus session ids, and
      repeats counted rather than checked adjacently: `/a/b/a/b` never repeats adjacently, which is
      what a naive check looks for. Tested in both directions, since a detector that refuses real
      pages yields a thin index with nothing to explain it
- [ ] M2-T03.8 Budget enforcement per source and per host
- [ ] M2-T03.9 Backpressure response to queue depth
- [ ] M2-T03.10 Leader failover test: kill the leader, assert no double dispatch

## M2-T14 — [[UI - Search Verticals]]

Tabs above the results: All, News, Images, Videos, Short videos, Files, Social. **They appear as
their content does** — five of the seven have nothing behind them today, and a tab that returns
"no results" is indistinguishable from a broken one.

- [ ] M2-T14.1 Tab row, vertical in the URL (`?v=news`) so it is shareable and the back button
      works, `role="tablist"` with arrow-key movement
- [ ] M2-T14.2 **News** — a filter over what is already indexed: web source, has a date,
      article-shaped. Real on day one
- [ ] M2-T14.3 **Files** — accept `application/pdf` in the fetcher (it refuses it today), extract
      text with a hard page cap, and a size limit well below the HTML one. A large share of
      `.gov.dz` PDFs are **scans**, which yield no text at all, so this covers the born-digital
      minority until OCR exists
- [ ] M2-T14.4 An empty vertical names *which* vertical is empty and links back to All
- [ ] M2-T14.5 Social tab, arriving with the connectors
- [ ] M2-T14.6 Images / Videos / Short videos, arriving with [[Milestone 3 - Multimodal Input]]

## M2-T13 — [[Crawler Console]] and [[UI - Admin Console]]

The question this answers, constantly: **is the crawler working, and is it collecting the right
things?** A document count answers that badly — it rises identically whether we are finding
Algerian news or four hundred copies of one calendar page, and it keeps rising for a while after
the crawl has gone wrong. So the console shows *what* is being collected, not only how much.

Server-rendered in the Rust API, not Next.js: this is the tool for diagnosing a broken system, and
a diagnostic that shares a failure domain with the thing it diagnoses is not a diagnostic. It is
also the fastest option — no bundle to download, parse and hydrate.

### Shell and navigation

- [ ] M2-T13.0 Sidebar shell with sections as real URLs, and a status bar carrying crawler state
      and throughput on **every** page — "is it still running" is asked while looking at something
      else
- [x] M2-T13.1 Overview: crawler state, documents, queue depth, usage counts. The document count
      comes from **index stats, not a search** — a search reports at most `maxTotalHits` (2000), so
      watching that number makes a healthy crawl look stalled the moment it passes the cap. That is
      exactly how "the crawler is not indexing" was diagnosed wrongly
- [ ] M2-T13.1b Overview: crawler state, documents today, queue depth, dead letters, tool-data age.
      Unknown values say so rather than showing zero, which is indistinguishable from healthy

### Crawler

- [ ] M2-T13.2 Start / stop / restart. **Stop drains** — in-flight fetches finish and index, and
      the frontier survives
- [ ] M2-T13.3 **Live**: one SSE stream at 1 Hz — counters, a documents-per-minute sparkline,
      per-host activity, a rolling feed of the last ~50 URLs with outcome, and skip reasons broken
      down. The feed is what shows it is collecting articles rather than tag pages; no aggregate can
- [ ] M2-T13.3b Store a real `word_count` on the document. The Documents list currently shows
      excerpt length, which is capped and so says nothing about article versus nav page — the one
      thing the column exists for. The Live feed has the true count because it sees the parsed body
- [ ] M2-T13.4 **Documents**: paged, newest first, searchable over title/URL/body via Meilisearch
      and filterable by host, source, language and date
- [ ] M2-T13.5 Document detail: extracted text, metadata, outlinks, raw fetch record. Rendered as
      **text, never HTML** — a crawled page is untrusted input and rendering it is stored XSS aimed
      at the most privileged account
- [ ] M2-T13.6 **Remove**, per document or selection, confirmed with a count. Removal suppresses the
      URL so the next pass does not re-add it, or the button feels broken
- [ ] M2-T13.7 **Queue**: depth per host, oldest entry, in flight. Enqueue a URL, optionally at the
      front — ordering only, it still passes every check a discovered URL passes
- [ ] M2-T13.8 **Discovered**: off-seed hosts, ranked by inbound links. The answer to "what would it
      find if I let it", and where a new source is promoted from
- [ ] M2-T13.9 **Sources**: seed list with per-source counts, last crawl, error rate, trust tier
- [ ] M2-T13.10 Force **refetch** (go back to the site) and **reindex** (re-run extraction on the
      stored blob). Distinct: a parser fix needs no network, and conflating them spends other
      people's bandwidth on our bug

### Index and system

- [ ] M2-T13.11 Index search as an operator — same ranking, with score and raw document shown
- [ ] M2-T13.12 Index health: counts by language and source, size, settings drift, last migration
- [ ] M2-T13.13 System: existing compute and politeness controls moved into the shell, plus a log
      tail with a level filter and no query text

### Performance and safety

- [ ] M2-T13.14 Budgets enforced: < 200 ms first render, CSS ≤ 8 KB, JS ≤ 10 KB, document list
      < 300 ms at 1M documents. Paged never "all"; one SSE stream per page; absolute values not
      deltas, so a dropped frame costs nothing
- [ ] M2-T13.15 Nothing auto-refreshes under the cursor — new rows queue behind a "12 new" button.
      A list that reorders as you reach for a row is how the wrong document gets deleted
- [ ] M2-T13.16 Crawler exposes `/metrics`, so the console and Prometheus read the same counters
      and cannot disagree
- [ ] M2-T13.17 Tests: frontier survives restart; blocked and private-address URLs refused through
      enqueue; a `<script>` in a crawled body is escaped in the detail view; Redis down shows
      "cannot read state" rather than zeroes
## M2-T04 — [[Web Fetcher]]

- [ ] M2-T04.1 `reqwest` client with timeouts, redirect revalidation, streamed body cap
- [x] M2-T04.2 Conditional requests (`If-None-Match` / `If-Modified-Since`) and 304 short-circuit.
      A 304 is `Ok` with an empty body, not an error — it is the best possible answer. Validators
      are overwritten only when the server sends new ones: a 304 sends none, and clearing them on
      it would make the *next* request unconditional, paying full price precisely because the last
      visit was free. Proven over real HTTP against the fixture server
- [x] M2-T04.3 Charset detection cascade including `windows-1256` — header charset, then the
      `<meta charset>` / `<meta http-equiv>` declared in the document head, then byte sniffing
      (chardetng). The middle step is the browser order and the one that matters for older Algerian
      sites serving windows-1256 with a bare Content-Type; a declared charset is never counted as a
      guess. Both meta forms tested against real windows-1256 Arabic
- [x] M2-T04.4 Honest user-agent; per-host connection cap of 1. The UA is `XustiveBot/1.0
      (+https://xustive.dz/bot; …)`, published at `/bot`. The cap is now real: `reserve` advances the
      host's next-allowed slot under the lock at claim time, so two callers sharing a fetcher cannot
      both fetch one host at once — the second waits a full delay behind the first. Monotonic;
      record_fetch cannot pull a reserved slot back. Tested that reservations stack in order
- [x] M2-T04.5 Outcome classification table — `FetchError::outcome` returns the stable §4.4 label
      (gone, throttled, transient, permanent, timeout, too_large, redirect_loop, …), finer than the
      retry-or-not class, and the orchestrator records it so failures break down by cause instead of
      one 'failed'. `is_gone` (404/410) is called out as the one the orchestrator can act on. A test
      pins the table and that a gone resource is never retryable
- [ ] M2-T04.6 Headless escalation rules + ratio cap; sandboxed browser container
- [ ] M2-T04.10 **Incomplete certificate chains.** Several `.gov.dz` hosts serve a valid Sectigo
      certificate without the intermediate, so every correctly-configured client fails —
      `curl` included. Browsers hide this by chasing the Authority Information Access extension;
      rustls does not. Options are AIA chasing or bundling known intermediates. **Not** disabling
      verification. Until then these hosts are unreachable and the log says why
- [x] M2-T04.7 Raw blob storage with TTL — `RawStore` keeps a fetched body under its URL with an
      expiry, for reindexing without a re-fetch. Opt-in (`raw_ttl_days`, default 0): blanket storage
      would fill the 1 GB Redis the frontier and queue depend on, so it waits for object storage
      (the real home). Bounded by a per-blob cap and the TTL; best-effort. Store/retrieve/forget and
      TTL expiry proven against real Redis
- [x] M2-T04.8 **SSRF suite including redirects to private IPs** — 13 cases covering the bypasses
      that get past a guard checking only literals: IPv4-mapped IPv6, decimal and octal spellings
      of loopback, credentials hiding the real host, non-HTTP schemes, resolved addresses (one bad
      entry in a round-robin sinks the set), and a redirect from a public host to a private one.
      All passed — the guard was already sound
- [~] M2-T04.9 Politeness assertion — the per-host serialisation is now enforced by `reserve`
      (M2-T04.4) and unit-tested (reservations stack, a slot is never shortened). The full
      50-concurrent-worker integration assertion against a real server is still owed

## M2-T05 — [[Deduplication Service]]

- [x] M2-T05.1 URL canonicalisation with tracking-param stripping — `frontier::canonical`: strips
      the utm/fbclid/gclid family, sorts the query so order does not matter, drops the fragment,
      normalises host/scheme case and default ports, and the bare-root trailing slash. Path case is
      kept, since a server may distinguish it
- [~] M2-T05.2 Exact `content_hash` check — a persistent Redis set of indexed hashes, checked
      before queueing, atomic via `SADD`. Catches the same body at two URLs (a syndicated wire
      story) which canonicalisation cannot, and skips re-queuing an unchanged revisit for free. The
      Bloom pre-filter (a latency optimisation for the negative path) is deferred — the exact check
      is one atomic round trip and sufficient
- [x] M2-T05.3 SimHash banding index and distance verdicts — four 16-bit bands, so any two hashes
      within Hamming distance 3 share a band (pigeonhole, no false negatives), confirmed by full
      distance. Fail-open. Pigeonhole guarantee checked across every ≤3-bit flip; near-duplicate
      detection proven end to end on raw hashes and reworded text
- [~] M2-T05.4 Winner selection (trusted-date, then earliest, then trust, then length) + engagement
      aggregation — pure decision logic done and tested; a guessed date cannot beat a real one on
      'earliest'. Remaining: wiring it to collect the candidate set behind a SimHash match
- [ ] M2-T05.5 pHash image dedup and embedding reuse
- [ ] M2-T05.6 Cluster ids for the 4–8 distance band
- [x] M2-T05.7 **Fail-open on Redis unavailability**, with a test. An unreachable dedup store yields
      `is_new = true`, so a Redis wobble lets documents through rather than dropping them —
      indexing a duplicate is a no-op, a lost document is permanent. Proven against a dead Redis
- [x] M2-T05.8 Volatile-page detection (revision loop guard) — the same mechanism as M2-T15.4 from
      the dedup side: a page changing on every fetch would add a fresh content hash to the seen set
      each visit, so parking it at the ceiling caps the flood. Tested in the dedup framing
- [~] M2-T05.9 Quality evaluation — the real classifier over 500 duplicate and 500 distinct
      generated pairs: **precision 1.000, recall 0.866** against the 0.95/0.85 gate. Deterministic
      and generated, so it is a regression guard and a check on the distance threshold, not a
      production claim; the real labelled set the exit gate names is still owed. Surfaced that
      SimHash shingling puts multi-edit rewrites in the 4-8 cluster band, not the duplicate band

## M2-T06 — [[Enrichment Pipeline]]

- [ ] M2-T06.1 `EnrichmentStep` trait and ordered executor
- [ ] M2-T06.2 Required vs optional steps; skip-under-pressure with `enrichment_level = "partial"`
- [x] M2-T06.3 Quality scoring — `quality_score` combines body length, date precision, extraction
      method, a real title, author, media and detected language into a bounded 0-1 signal that
      feeds ranking and spam suppression. Tested for bounds, and that a trusted date raises it
- [x] M2-T06.4 Spam scoring + phrase list; suppression at 0.8 (not deletion) — `spam::spam_score`
      populates `spam_score` from two signals, stronger wins: distinct spam phrases present (a
      data-file list in ar/fr/en, so one phrase repeated is one signal) and keyword stuffing (the
      most common content word's share of the body). Conservative by design — a false positive
      buries a real document. Wired into parse; search already suppresses at 0.8
- [x] M2-T06.5 Geo/wilaya gazetteer hinting — `gazetteer::detect_wilaya` scans title and body for
      the 58 wilaya names (ar + fr), folded and whole-token, and hints the one named most; a lone
      tied mention is left unhinted. The name table is generated from xustive-tools so the two do
      not drift. Wired into parse, populating `geo.wilaya`
- [x] M2-T06.6 Topic labelling — `topics::label` assigns a coarse subject (politics, economy,
      sport, culture, health, education, technology, society) from the document's vocabulary in
      ar/fr/en; two-hit minimum, at most three labels, empty when unclear. Wired into parse
- [ ] M2-T06.7 Comment enrichment with caps
- [ ] M2-T06.8 Per-step watchdog timeouts
- [ ] M2-T06.9 Repass job for partial documents
- [ ] M2-T06.10 Spam evaluation: 300 labelled posts, precision ≥ 0.90

## M2-T15 — Freshness and Adaptive Recrawl ★ *the index is a photograph until this lands*

Governed by [[ADR-0011 - Adaptive Recrawl over Static Crawling]]. A URL is crawled once today and
then forgotten, so a corrected story or a new decree never reaches the index. Two results from the
literature shape this and both are counterintuitive enough to restate: **recrawling in proportion
to change rate is worse than recrawling everything uniformly** (Cho & Garcia-Molina 2003), and the
signal that matters is **whether a change persisted**, not whether bytes differed
(Olston & Pandey 2008).

Order matters here. T15.1 and T15.2 are the substrate; nothing above them works without both.

- [~] M2-T15.1 **Dual content hash.** One over the raw body, one over extracted article text after
      boilerplate stripping. Only the second counts as a change. This is longevity scoring at the
      cost of a second hash, and it is what stops the crawler chasing view counters and "most read"
      sidebars on every Algerian news page it holds. **Half of this already existed**: `content_hash`
      is BLAKE3 over the *extracted, normalised* body, so comparing it across fetches already
      ignores everything outside the article. Remaining: a raw-body hash, useful only to tell
      "nothing moved" apart from "only the furniture moved" when diagnosing a channel
- [x] M2-T15.2 **Change history**: `last_fetched`, `last_modified`, `etag`, the content hash, the
      current interval and the volatility count. **Stored in Redis beside the frontier, not on the
      indexed document** as ADR-0011's consequences assumed — it is written on every fetch
      including unchanged ones, and Meilisearch takes writes as queued tasks, so a million-page
      corpus revisiting daily would enqueue a million bookkeeping tasks a day that change nothing
      searchable. Losing Redis costs intervals, not documents
- [x] M2-T15.3 **Adaptive interval, AIMD.** Changed → halve, unchanged → add one floor-step, clamped
      per trust tier. Chosen over the formal estimators because we never observe *how many* times a
      page changed — only whether it differs from our copy — so the obvious estimator is biased low
      exactly where it costs most. Additive increase rather than multiplicative: the freshness eval
      (M2-T15.10) showed multiplicative growth overshoots and oscillates, measurably worse than a
      fixed interval. Bounds are per trust tier, or a large quiet corpus drags every interval to the
      ceiling and the sources that matter go stale with the rest
- [x] M2-T15.4 **Volatile-page abandonment.** Changes on every visit even at its floor → slow lane.
      The Cho result applied directly: a page that cannot be kept fresh should not be chased.
      Shares its mechanism with M2-T05.8. Parked at the ceiling rather than dropped, so a ticker
      that becomes an archive is eventually noticed; four consecutive changes rather than one, so a
      burst of breaking news is not mistaken for a page that never settles
- [x] M2-T15.5 **Conditional requests wired into scheduling.** The revisit path replays stored
      validators and a 304 reaches the scheduler as a `NotModified` observation — otherwise free
      revisits would look like silence and the interval would stop adapting. Discovery sends no
      validators and is untouched
- [x] M2-T15.6 **Sitemap `lastmod` polling as a freshness signal.** `extract_entries` parses the
      `loc`/`lastmod` pairs, `sitemap_verdict` compares each against our last fetch, and
      `poll_sitemap` acts: a changed page is deferred into the frontier as due-now, an unchanged one
      grows its interval with no request. The daemon runs it as its own 6-hour task beside the
      workers. Proven against the fixture sitemap and the daemon starts it cleanly. One sitemap fetch
      stands in for hundreds of revisits that would each cost a request to learn nothing
- [x] M2-T15.7 **Revisit priority folds in measured change rate and lateness**, alongside the depth
      and trust base. Not the literal `trust × change_probability × age` product: a page held near
      its floor is one we have *measured* as changing, so the converged interval is the signal, and
      both it and overdueness are capped. Uncapped, a page changing every hour would outrank
      everything on its host forever — precisely the page the Cho result says not to chase. Bands
      rather than a curve, because a continuous function is much harder to reason about from a
      document count that stopped rising
- [~] M2-T15.8 **Recrawl and discovery budgets are counted apart, and the split is visible.** The
      crawler tells a revisit from fresh discovery at fetch time (a claim with prior visit state),
      so `revisited` is now counted alongside `fetched`; `fetched - revisited` is discovery. Shown
      in the console and in `/metrics` (`xustive_crawl_fetched_total`, `xustive_crawl_revisited_total`).
      What remains is *enforcement* — a reserved slice so one cannot starve the other — which needs
      the frontier to tag entries by kind at claim time
- [~] M2-T15.9 **Boilerplate-stripping stability tests.** Two fetches of the same article with a
      rotating most-read sidebar and a changed ad slot now assert an identical body **and content
      hash** — the property the freshness scheduler rests on, since a leak that rotates would make
      every revisit read as a change. The common case (external furniture) passes. One documented
      gap remains, left as an ignored test: a relative timestamp rendered *inside* the article
      ("updated 2 hours ago") still leaks, and fixing it needs a per-domain rule rather than a
      heuristic
- [x] M2-T15.10 **Freshness evaluation** — simulates a population of known change periods and runs
      the real scheduler against two fixed intervals and the proportional policy, measuring mean
      staleness and fetches against exact ground truth. It earned its place immediately: it caught
      that the scheduler's multiplicative growth was **worse** than both a fixed interval and
      proportional (45 h staleness vs 14.5 h), which forced the AIMD fix that brought it to 8.8 h.
      Asserts the defensible claims — fresher than the cheap fixed policy, not dominated by
      proportional, abandons the untrackable — not strict domination, which the literature does not
      support

## M2-T16 — Corpus Bootstrap and Discovery Aggregation

Governed by [[ADR-0013 - Direct SERP Collection for Discovery]] (superseding
[[ADR-0012 - Discovery-Only Aggregation]]). External sources are used to learn **which URLs exist**,
never to answer a user's search. Live metasearch stays rejected — that was never the
terms-of-service objection.

Ordered by yield per unit of effort, and the order matters: **a SERP query returns about ten URLs;
Common Crawl returns billions.** SERP collection is the narrow, targeted, last-resort channel for
queries the bulk sources cannot answer. If it ever becomes the main discovery path, something
upstream has failed.

Everything discovered here enters the ordinary frontier under the ordinary rules — robots,
politeness, `SafeUrl`, dedup, trust tiering. **An externally discovered URL gets no privileges.** We
disregard the search engine's terms by my direction; we do not disregard the terms of the sites it
points at.

- [x] M2-T16.1 **Common Crawl index ingestion.** `xustive-cli common-crawl` reads a snapshot's CDX
      index, filters, and seeds URLs into the frontier at discovered-tier trust (channel `cc`).
      Verified live: 14,467 `.dz` URLs queued from two pages of CC-MAIN-2026-30
- [x] M2-T16.2 **Algeria filter**: `.dz` plus known Algerian hosts on generic TLDs (from the
      registry); language dropped only when the index's own `languages` tag says non-ar/fr/en,
      otherwise deferred to the crawl-time detector. `select_urls` filters + dedups per page
- [x] M2-T16.3 **Incremental snapshot tracking**: last-page-per-`(snapshot,pattern)` in Redis,
      written after each page's URLs are queued. Resumable — verified live resuming at page 1
- [x] M2-T16.4 **Query-driven discovery.** Weak searches recorded as k-anonymous (k ≥ 20), windowed,
      off-by-default counters over normalised terms — no query log, nothing surfaced below the floor
      ([[ADR-0008 - No Query Logging]]). `xustive-cli discover` resolves the surfaced terms to URLs
      via Brave (T16.6), seeds them at discovered trust (channel `brave`), and forgets each once
      actioned so a run does not re-pay for it
- [x] M2-T16.5 **Weak-coverage queue in the console**: the "Weak coverage" page (`/admin/weak-coverage`)
      lists the k-anonymous gaps; "disabled" is shown distinctly from "no gaps"
- [x] M2-T16.6 **Brave Search API connector** for the residual — `xustive-ingest::brave`, budgeted
      (`brave_max_queries_per_run`), **off by default**, inert without a key. Pure result-parsing is
      unit-tested; the live call is key-gated. Only URLs are taken, never Brave's titles/snippets
- [x] M2-T16.7 **Provenance on every document**: seed, link, sitemap, Common Crawl, query-driven,
      Brave, or SERP. `DiscoveryChannel` on the frontier `Pending`/`Claim` and stamped onto every
      `Document`; the channels not yet built (cc/query/brave/serp) exist in the enum ready to be set
- [~] M2-T16.8 **Per-channel yield reporting**: URLs discovered, fetched, indexed, and surviving
      dedup, per channel — the funnel + yield/unique rates on the "Discovery yield" console page
      (`/admin/discovery`). **Cost per surviving document** waits on the paid/collected channels
      (T16.6/.9) that have a cost to divide; the volume funnel is in place

### Direct SERP collection *(my direction — [[ADR-0013 - Direct SERP Collection for Discovery]])*

Last in the ladder and deliberately narrow. Reuses the collection layer built for M2-T01a/b/c
rather than growing a second evasion path — a SERP source is just another consumer of the identity,
proxy and fingerprint machinery, bound by the same pinning invariant.

- [ ] M2-T16.9 **SERP source behind the queue, never a call.** The serving plane publishes a
      weak-coverage term to a stream; the ingestion plane consumes it and owns all egress. Keeps
      `scripts/test-egress.sh` green and meaningful — the boundary is ours, not Google's, and
      nothing about T16.9 requires giving it up
- [ ] M2-T16.10 **Wired into [[Proxy Manager]] / [[Session Manager]] / [[Fingerprint Engine]]**, with
      residential egress **required** — datacenter ranges are classified almost immediately. Off by
      default; it costs money to run
- [ ] M2-T16.11 **Endpoint ladder**, lightest first, demoting on failure — the shape of
      [[Signature Service]] §4.6. Most discovery only needs a list of URLs, which the plainest
      endpoint gives at a fraction of the cost and exposure of a rendered browser
- [ ] M2-T16.12 **Human-shaped pacing**: low volume, jitter, diurnal shaping ([[Session Manager]]
      §4.5). The failure mode is never a single request; it is a regular pattern
- [ ] M2-T16.13 **Challenge → quarantine and back off.** CAPTCHA, interstitial or consent wall
      retires the identity per M2-T01a.7/.9. **Challenges are detected, not solved** — a challenge
      means the identity is already classified, and pushing through burns it faster than resting it
- [ ] M2-T16.14 **Canary queries against silent degradation.** Suspected bots are served plausible
      but degraded results rather than an error, so a neutered channel reports itself healthy.
      Known-stable queries checked against expected URLs, borrowed from M2-T01a.8. Without this,
      T16.8's yield figures are measuring a lie
- [ ] M2-T16.15 **Parser fixtures and a rot alarm.** SERP markup changes without notice and this
      will break repeatedly — the maintenance tail [[ADR-0009 - Direct Collection for Social
      Platforms]] names. Fixtures committed, `serp_parse_miss_total` alerting from day one

## M2-T07 — [[Proxy Manager]] *(now required)*

- [x] M2-T07.1 Pool kinds: `direct`, `datacenter`, `residential`, `mobile`; per-source-class policy
      (`PoolKind`, `PoolPolicy` in `xustive-ingest::proxy`)
- [ ] M2-T07.2 Provider selection and contracts ← *decide with M2-T01d.4* — procurement, external
- [x] M2-T07.3 Health EWMA, quarantine, probing, selection weighting (`proxy::health`, `proxy::pool`)
- [x] M2-T07.4 **`acquire_pinned`** honouring the identity pinning invariant — same proxy every time,
      a dead pin errors for reassignment rather than silently swapping
- [x] M2-T07.5 Geo/ASN targeting: ≥ 4 ASNs, ≤ 3 identities per /24 (`proxy::placement`)
- [x] M2-T07.6 Failure attribution (proxy vs host vs identity vs ASN) — host/ASN win over proxy so an
      outage never quarantines the pool (`proxy::attribution`)
- [x] M2-T07.7 Shared circuit breakers in Redis (host, platform, ASN) — doubling cooldown, verified
      shared across replicas (`proxy::breaker`)
- [x] M2-T07.8 **Graded `on_blocked` ladder** — with the test that `open_web` still halts-and-flags
      (`proxy::ladder`)
- [ ] M2-T07.9 Egress-IP assertion; lease-leak detection; credential rotation — needs real proxy egress
- [ ] M2-T07.10 **Bandwidth accounting and cost-per-1k-docs**; 80 % budget alert — needs real transfer
      measurement, wires to T16.8's cost-per-document
- [x] M2-T07.11 Guard test: platform collection **halts** rather than falling back to `direct`
      (`proxy::pool`)

*Decision logic done and unit-/Redis-tested; the three open items (.2, .9, .10) need a real proxy
provider and live egress to build against, not more code.*

## M2-T08 — [[Social Connector - Facebook]] *(gated on M2-T01a/b/c)*

- [ ] M2-T08.1 Access-path ladder: graph → mbasic → m → graphql → public html
- [ ] M2-T08.2 `doc_id` / `fb_dtsg` / `lsd` / `jazoest` handling via [[Signature Service]]
- [ ] M2-T08.3 Page and group enumeration; posts and comments pagination
- [ ] M2-T08.4 Group access: operator-performed joins where `join_required`; never automated
- [ ] M2-T08.5 `since`-window collection with 2 h overlap (feed ordering is unstable)
- [ ] M2-T08.6 Mapping incl. relative-timestamp resolution (`منذ ساعتين`, `hier à 14:03`)
- [ ] M2-T08.7 **Empty-feed / cloaking detection** wired to canaries
- [ ] M2-T08.8 Automatic path demotion on repeated failure, with `access_path` recorded per document
- [ ] M2-T08.9 Engagement refresh; **deletion propagation within 24 h**
- [ ] M2-T08.10 Recorded-fixture tests per path; **no live platform requests in CI**

## M2-T09 — [[Social Connector - Instagram]] *(gated on M2-T01a/b/c)*

- [ ] M2-T09.1 Access-path ladder: graph → **embedded JSON** → web graphql → mobile api → oembed
- [ ] M2-T09.2 Anonymous-first with login-wall escalation; low-value identity tier
- [ ] M2-T09.3 Mapping including carousels and video covers
- [ ] M2-T09.4 **Expiring-CDN-URL handling**; 30 min post→media deadline; enrichment priority boost
- [ ] M2-T09.5 Empty-caption + usable-OCR → indexed with `body_source = "ocr"`; otherwise dropped
- [ ] M2-T09.6 Curated hashtag rotation; hashtag results at `trust_tier` C
- [ ] M2-T09.7 `X-IG-WWW-Claim` propagation test
- [ ] M2-T09.8 Recorded-fixture tests per path
- [ ] M2-T09.9 **Measure**: what fraction of the target corpus the embedded-JSON path alone reaches

## M2-T10 — [[Social Connector - TikTok]] *(gated on M2-T01a/b/c)*

- [ ] M2-T10.1 Access-path ladder: research api → **embedded hydration** → web api → mobile api
- [ ] M2-T10.2 `X-Bogus` / `msToken` via [[Signature Service]]
- [ ] M2-T10.3 **Signer-rotation drill**: assert automatic demotion to the hydration path
- [ ] M2-T10.4 Date-windowed collection with completion markers for idempotent backfill
- [ ] M2-T10.5 Mapping including `voice_to_text` and hashtags
- [ ] M2-T10.6 Cover-frame handoff to [[Image Pipeline]]
- [ ] M2-T10.7 Engagement refresh with P95-capped normalisation
- [ ] M2-T10.8 Assert **no code path downloads video bytes**
- [ ] M2-T10.9 Recorded-fixture tests

## M2-T11 — [[Data Sources Registry]] seeding

- [x] M2-T11.1 Registry schema, storage, and git export on change
- [x] M2-T11.2 `legal_basis` required on every record; auto-disable when it lapses
- [~] M2-T11.3 Seed ~500 web sources across the categories in its §4 — 96 real domains
  seeded (`data/sources/registry.jsonl`); ~400 left for human curation, all as `proposed`
- [~] M2-T11.4 Per-domain parser rules for the top 50 ([[Content Parser]]) — 12 rules shipped +
  `xustive-cli parse-check` to author the rest from real HTML (a rule is only added where generic
  extraction verifiably fails); remaining ~38 are per-site curation against live article pages
- [x] M2-T11.5 Per-source quality dashboards (fetch, extraction, dedup, spam, date precision)
- [x] M2-T11.6 Lifecycle automation: degrade on sustained failure
- [ ] M2-T11.7 Name a curation owner ← *B5*

## M2-T12 — [[Admin and Source Submission]] (admin half)

- [ ] M2-T12.1 Admin endpoints with Argon2id-verified scoped keys
- [ ] M2-T12.2 Registry CRUD and recrawl triggers
- [ ] M2-T12.3 **Takedown: vectors → comments → document → permanent blocklist**
- [ ] M2-T12.4 Fail loudly on partial takedown — never report success
- [ ] M2-T12.5 Immutable audit log
- [ ] M2-T12.6 `xustive-cli` admin commands
- [ ] M2-T12.7 End-to-end takedown test including a re-crawl attempt that must not resurrect

---

## Exit Gate

| Check | Threshold |
|:---|:---|
| Scale | 1M documents indexed from real sources |
| Politeness | ≤ 1 concurrent request per host; crawl-delay honoured within ±10 %; zero robots violations in an audit |
| Freshness | tier-A sources under 6 h staleness; adaptive recrawl beats a fixed-interval baseline on **both** mean staleness and fetches-per-real-change (M2-T15.10) |
| Discovery | every document carries its provenance; per-channel yield reported (M2-T16.7/.8) |
| Dedup | duplicate rate between 5 % and 60 %; precision ≥ 0.95 |
| Social | all three connectors live and collecting; path ladders demote correctly under failure |
| Collection health | median identity lifespan ≥ 60 days; challenge rate < 10 %; canaries green |
| Cloaking | silent-empty detection verified — a cloaked identity is quarantined, not reported healthy |
| Cost | cost per 1 000 documents measured per source; residential bandwidth within budget |
| Takedown | end-to-end removal verified, including re-crawl resistance |
| Obligations | deletion propagation verified on all three platforms |
| Stability | ingestion runs 72 h without manual intervention |

## Risks

| Risk | Mitigation |
|:---|:---|
| **Identity pool burns faster than it can be replaced** | conservative budgets, warm-up discipline, pinning invariant; `IdentityLifespanDrop` is the leading indicator ([[Session Manager]] §9) |
| Warm-up wall-clock not started early enough | M2-T01a.11 explicitly starts in M1 |
| Signer rotation halts a platform | path ladders demote to unsigned paths; [[Signature Service]] §4.6 |
| **Silent cloaking reported as success** | canaries are ground truth; this is an exit-gate item, not a nice-to-have |
| Residential bandwidth cost runs away | per-source cost metric + 80 % budget alert; prefer light paths (`mbasic`, embedded JSON) |
| Platform stance leaks into open-web crawling | crawl profiles are config-driven and CI-asserted ([[Politeness and Robots]] §4.0) |
| A crawler bug harms a real site | unchanged: politeness config guard, per-host concurrency 1, staged rollout, `/bot` contact monitored |
| Parser rules rot as sites redesign | `parser_rule_miss_total` alerting from day one; each rule ships with a fixture |
| Redis memory exhausted by raw blobs | monitor early; the object-storage decision is pre-identified ([[Task Queue]] §12) |
| Real content breaks M1's ranking assumptions | re-run the full relevance evaluation at the end of this milestone |
| **Boilerplate stripping mistakes churn for content** | M2-T15.1 gives bad stripping a second failure mode — the crawler chases furniture forever. M2-T15.9 asserts extracted text is stable across fetches that differ only in furniture |
| Common Crawl bootstrap floods the frontier with dead URLs | discovered-tier trust, ordinary dedup and `SafeUrl`; per-channel yield (M2-T16.8) makes a bad channel visible rather than merely expensive |
| **SERP collection is silently degraded rather than blocked** | canary queries with known-stable results (M2-T16.14). Without them a neutered channel reports itself healthy and T16.8's yield numbers measure nothing |
| SERP channel competes with social for the identity pool | it is last in the ladder, off by default, and M2-T01a.12 already halts rather than degrading to unpinned identities |
| SERP maintenance tail is underestimated | fixtures plus `serp_parse_miss_total` from day one (M2-T16.15); the tail is real and permanent, as with the social connectors |
| Query-driven discovery reintroduces query logging | aggregate counts over normalised terms with a frequency floor, never a stored log ([[ADR-0008 - No Query Logging]]); the privacy constraint is the design constraint |

## Related

[[TODO]] · [[ADR-0009 - Direct Collection for Social Platforms]] ·
[[ADR-0011 - Adaptive Recrawl over Static Crawling]] · [[ADR-0012 - Discovery-Only Aggregation]] ·
[[Session Manager]] ·
[[Fingerprint Engine]] · [[Signature Service]] · [[Proxy Manager]] · [[Crawler Orchestrator]] ·
[[Politeness and Robots]] · [[Data Sources Registry]] · [[Legal and Compliance]] ·
[[Milestone 4 - Quality and Operations]]
