---
tags:
  - planning
  - milestone
milestone: 3
status: not-started
updated: 2026-08-06
---

# Milestone 3 - Ingestion at Scale

> **Goal:** stop using fixtures. Crawl the real Algerian web politely, ingest whatever social access
> we have actually been granted, and get to 1M documents.
> **Exit gate:** 1M documents indexed; politeness verified under load; whatever social connectors are
> authorised are live; takedown path works end to end.
> Parent: [[TODO]] · Previous: [[Milestone 1 - Text Search MVP]] · Next: [[Milestone 4 - Quality and Operations]]

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

- [ ] M2-T01a.1 Identity record; encrypted credential/cookie storage (XChaCha20-Poly1305)
- [ ] M2-T01a.2 **Pinning invariant** — `account ↔ proxy ↔ fingerprint ↔ device`, with the test
      proving no code path can break it
- [ ] M2-T01a.3 Lifecycle: fresh → warming → mature → quarantined → burned
- [ ] M2-T01a.4 Warm-up scheduler with human-shaped browsing patterns
- [ ] M2-T01a.5 Per-identity budgets with jitter and diurnal shaping
- [ ] M2-T01a.6 Login flows + TOTP; operator queue for anything needing a human
- [ ] M2-T01a.7 Challenge detection (captcha, checkpoint, login wall, rate limit)
- [ ] M2-T01a.8 **Silent-cloaking detection**: canaries + `consecutive_empty` thresholds
- [ ] M2-T01a.9 Quarantine/recovery with doubling cooldown; burn after 3
- [ ] M2-T01a.10 Fail-closed budget accounting across Redis restarts
- [ ] M2-T01a.11 **Account acquisition and pool sizing** ← procurement; start in M1
- [ ] M2-T01a.12 Pool exhaustion halts the platform (never degrade to unpinned identities)

## M2-T01b — [[Fingerprint Engine]]

- [ ] M2-T01b.1 Select the impersonation library (`rquest` vs alternatives); validate JA4 accuracy
- [ ] M2-T01b.2 Profile schema and catalogue (12–20 profiles)
- [ ] M2-T01b.3 TLS + HTTP/2 + header-order profiles wired into the client
- [ ] M2-T01b.4 **Coherence test suite** — every §4.2 invariant, checked mechanically
- [ ] M2-T01b.5 Headless CDP patch scripts; real Chrome, persistent per-identity profile
- [ ] M2-T01b.6 WebRTC forced through proxy; leak assertion
- [ ] M2-T01b.7 Distribution weights matching the real Algerian browser mix
- [ ] M2-T01b.8 Version ageing and successor migration
- [ ] M2-T01b.9 `make fp-verify` self-validation against echo endpoints, nightly
- [ ] M2-T01b.10 Dependency pinning so a library bump cannot silently change our fingerprint

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
- [ ] M2-T01d.3 Record the owner's risk acceptance, dated
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
- [x] M2-T03.3 Priority computation — depth dominates; trust and article-shape only break ties, or one trusted source swallows the crawl
- [ ] M2-T03.4 Adaptive revisit intervals (changed / unchanged / 304)
- [ ] M2-T03.5 Sitemap and feed discovery with caps
- [ ] M2-T03.6 Outlink filtering and `SafeUrl` validation
- [x] M2-T03.7 Crawler-trap detectors (depth, params, repeating segments) — plus session ids, and
      repeats counted rather than checked adjacently: `/a/b/a/b` never repeats adjacently, which is
      what a naive check looks for. Tested in both directions, since a detector that refuses real
      pages yields a thin index with nothing to explain it
- [ ] M2-T03.8 Budget enforcement per source and per host
- [ ] M2-T03.9 Backpressure response to queue depth
- [ ] M2-T03.10 Leader failover test: kill the leader, assert no double dispatch

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
- [ ] M2-T04.2 Conditional requests (`If-None-Match` / `If-Modified-Since`) and 304 short-circuit
- [ ] M2-T04.3 Charset detection cascade including `windows-1256`
- [ ] M2-T04.4 Honest user-agent; per-host connection cap of 1
- [ ] M2-T04.5 Outcome classification table
- [ ] M2-T04.6 Headless escalation rules + ratio cap; sandboxed browser container
- [ ] M2-T04.10 **Incomplete certificate chains.** Several `.gov.dz` hosts serve a valid Sectigo
      certificate without the intermediate, so every correctly-configured client fails —
      `curl` included. Browsers hide this by chasing the Authority Information Access extension;
      rustls does not. Options are AIA chasing or bundling known intermediates. **Not** disabling
      verification. Until then these hosts are unreachable and the log says why
- [ ] M2-T04.7 Raw blob storage with TTL
- [x] M2-T04.8 **SSRF suite including redirects to private IPs** — 13 cases covering the bypasses
      that get past a guard checking only literals: IPv4-mapped IPv6, decimal and octal spellings
      of loopback, credentials hiding the real host, non-HTTP schemes, resolved addresses (one bad
      entry in a round-robin sinks the set), and a redirect from a public host to a private one.
      All passed — the guard was already sound
- [ ] M2-T04.9 Politeness assertion under 50 concurrent workers: one in-flight request per host

## M2-T05 — [[Deduplication Service]]

- [ ] M2-T05.1 URL canonicalisation with tracking-param stripping
- [ ] M2-T05.2 Bloom + exact `content_hash` check
- [ ] M2-T05.3 SimHash banding index and distance verdicts
- [ ] M2-T05.4 Winner selection (earliest, then trust, then length) + engagement aggregation
- [ ] M2-T05.5 pHash image dedup and embedding reuse
- [ ] M2-T05.6 Cluster ids for the 4–8 distance band
- [ ] M2-T05.7 **Fail-open on Redis unavailability** + a test proving it
- [ ] M2-T05.8 Volatile-page detection (revision loop guard)
- [ ] M2-T05.9 Quality evaluation: 500 dup + 500 distinct pairs, precision ≥ 0.95, recall ≥ 0.85

## M2-T06 — [[Enrichment Pipeline]]

- [ ] M2-T06.1 `EnrichmentStep` trait and ordered executor
- [ ] M2-T06.2 Required vs optional steps; skip-under-pressure with `enrichment_level = "partial"`
- [ ] M2-T06.3 Quality scoring
- [ ] M2-T06.4 Spam scoring + phrase list; suppression at 0.8 (not deletion)
- [ ] M2-T06.5 Geo/wilaya gazetteer hinting
- [ ] M2-T06.6 Topic labelling
- [ ] M2-T06.7 Comment enrichment with caps
- [ ] M2-T06.8 Per-step watchdog timeouts
- [ ] M2-T06.9 Repass job for partial documents
- [ ] M2-T06.10 Spam evaluation: 300 labelled posts, precision ≥ 0.90

## M2-T07 — [[Proxy Manager]] *(now required)*

- [ ] M2-T07.1 Pool kinds: `direct`, `datacenter`, `residential`, `mobile`; per-source-class policy
- [ ] M2-T07.2 Provider selection and contracts ← *decide with M2-T01d.4*
- [ ] M2-T07.3 Health EWMA, quarantine, probing, selection weighting
- [ ] M2-T07.4 **`acquire_pinned`** honouring the identity pinning invariant
- [ ] M2-T07.5 Geo/ASN targeting: ≥ 4 ASNs, ≤ 3 identities per /24
- [ ] M2-T07.6 Failure attribution (proxy vs host vs identity vs ASN)
- [ ] M2-T07.7 Shared circuit breakers in Redis (host, platform, ASN)
- [ ] M2-T07.8 **Graded `on_blocked` ladder** — with the test that `open_web` still halts-and-flags
- [ ] M2-T07.9 Egress-IP assertion; lease-leak detection; credential rotation
- [ ] M2-T07.10 **Bandwidth accounting and cost-per-1k-docs**; 80 % budget alert
- [ ] M2-T07.11 Guard test: platform collection **halts** rather than falling back to `direct`

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

- [ ] M2-T11.1 Registry schema, storage, and git export on change
- [ ] M2-T11.2 `legal_basis` required on every record; auto-disable when it lapses
- [ ] M2-T11.3 Seed ~500 web sources across the categories in its §4
- [ ] M2-T11.4 Per-domain parser rules for the top 50 ([[Content Parser]])
- [ ] M2-T11.5 Per-source quality dashboards (fetch, extraction, dedup, spam, date precision)
- [ ] M2-T11.6 Lifecycle automation: degrade on sustained failure
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
| Freshness | tier-A sources under 6 h staleness |
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

## Related

[[TODO]] · [[ADR-0009 - Direct Collection for Social Platforms]] · [[Session Manager]] ·
[[Fingerprint Engine]] · [[Signature Service]] · [[Proxy Manager]] · [[Crawler Orchestrator]] ·
[[Politeness and Robots]] · [[Data Sources Registry]] · [[Legal and Compliance]] ·
[[Milestone 4 - Quality and Operations]]
