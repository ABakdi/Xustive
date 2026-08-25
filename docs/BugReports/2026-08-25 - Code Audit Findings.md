---
tags:
  - bugs
  - audit
date: 2026-08-25
status: in-progress
---
# Code Audit Findings — 2026-08-25

Consolidated from a five-track review (search/summary pipeline, CLI eval tooling, federation
gateway, web frontend, cross-cutting) with each finding verified against the code before landing
here. Ordered by severity; tackled top-down. **User-privacy-class findings are deliberately
excluded from this tracker by operator decision (2026-08-25)** — they are listed at the bottom so
the decision is recorded, not silently forgotten.

Status: `open` → `fixed (commit)` / `wontfix (reason)`.

## High

### BUG-001 — Results page 500s when facets are degraded — **fixed**
`crates/xustive-search/src/client.rs` `facet_distribution: #[serde(default)] Value` defaults to
`Null`, so when the deadline drops the facet stage the API serializes `"facets": null`
(`search.rs`), while `web/lib/api.ts:74` types `facets` as a required record and
`web/components/search/Filters.tsx:67` indexes into it. Any query served while facets are
deadline-degraded throws `TypeError` in the Server Component and the whole results page errors —
the exact path `facets_degraded` exists to soften. Fix both sides: API emits `{}` instead of
`null`; web types `facets` as nullable and guards.

### BUG-002 — Deep pagination serves empty pages while advertising 100 — **fixed**
`search.rs`: the engine query always fetches `candidate_pool` (200) from offset 0 and the page is
`skip(offset)` over that pool, so pages past `pool / hits_per_page` are empty; `total_pages` still
advertises up to 100 and the pagination UI offers the dead pages. Also increments the zero-result
metric for queries that have hits. Fix: grow the pool to cover the requested offset
(`candidate_pool.max(offset + hits_per_page)`, bounded by the engine's `maxTotalHits`), and cap
`total_pages` at what a pool can actually serve.

### BUG-003 — Eval harness measures a pipeline production doesn't run — **fixed**
`crates/xustive-cli/src/eval.rs` `retrieve_with_expansion` claims parity with the API but lacks the
weak-top-score expansion trigger (M7-T01.3), the all-stop-word phrase rescue (M7-T01.5), and the
spam filter the API applies. `make eval` / `eval-ab` therefore score a different retrieval than
production, and A/B verdicts are made under a non-production expansion regime. Fix: port the three
legs into the shared helper.

## Medium

### BUG-004 — `federation.allowlist` is dead config with a false doc comment — **fixed**
`crates/xustive-core/src/config.rs` documents "deny-by-default allowlist"; nothing anywhere reads
it except the admin display. The gateway reaches whatever `SEARXNG_URL`/`EXTERNAL_LLM_URL` say.
Fix: remove the dead field and the false claim; state honestly that gateway egress is bounded by
its two env-configured endpoints (and drop the admin/web display of it).

### BUG-005 — Enabling the external summariser doubles the worst-case summary time — **fixed**
`crates/xustive-api/src/summary.rs`: `generate_external` spends up to `ml.deadline_ms`, then the
local `generate` starts a fresh `deadline_ms` of its own — up to ~2× the budget end to end,
contradicting "turning this on only changes who writes the summary". Fix: one shared deadline —
the local fallback gets what the external attempt left.

### BUG-006 — One circuit breaker couples `/federate` and `/summarise` — **fixed**
`crates/xustive-api/src/federate.rs`: both endpoints share a `SharedBreaker`, so a dead LLM
provider sheds federation and a dead SearXNG sheds external summaries. Fix: one breaker per
endpoint.

### BUG-007 — The same URL can appear as both a local result and a web card — **fixed**
`search.rs` `merge_federated` dedups only by `id_for_url`, but documents discovered by non-
federation channels carry ULID ids: a SearXNG hit for an already-indexed URL passes the id check
and duplicates the result on page 1. Fix: dedup by canonical URL as well as id.

### BUG-008 — Gateway accepts ~2MB unauthenticated prompts from any core container — **fixed**
`crates/xustive-federator/src/lib.rs`: `/summarise` (and `/federate`) have no auth, no rate limit,
and axum's default 2MB body cap; any compromised core-network container gets a free proxy to the
operator's paid LLM key. Fix (minimal): a tight `DefaultBodyLimit` on the gateway router sized to
real prompts; note the residual trust model in the module doc.

### BUG-009 — Dev `devhost` bridge gives the serving plane internet egress; egress test can't see it — **open**
`deploy/docker-compose.dev.yml`: `devhost` is a NAT-ing default bridge, so dev containers joined to
it gain full egress while `scripts/test-egress.sh` probes a *fresh* container on `core` only and
stays green. Fix: probe from inside the real API-plane container(s) where possible, and document
the dev-overlay exception explicitly in the script output.

### BUG-010 — New Darija privacy strings are Moroccan, not Algerian — **fixed**
`web/lib/i18n/messages.ts` (`ary`): `كتبحث`/`ما كيبانش` (Moroccan ka-/ki- imperfective) and
`ديالك` (Moroccan possessive) contradict the same catalogue's own `تاعك`/`تاع` forms. Fix: align
with the catalogue's Algerian conventions (still flagged for native review, B7).

### BUG-011 — Interaction-uplift replay doesn't match the runtime signal — **fixed**
`crates/xustive-cli/src/eval.rs` replay uses global per-doc `clicks/(imp+5)`; runtime
(`xustive-ingest/src/interaction.rs`) serves per-(query,doc) Wilson lower bounds with fallback.
The printed M6 uplift number doesn't predict the live feature. Fix: replay with the same Wilson
shape, keyed per query.

### BUG-012 — Miner's federated pseudo-domain breaks the domain floor both ways — **fixed**
`crates/xustive-cli/src/mine.rs`: all federated evidence shares one pseudo-domain, so (a) a
federated-only pair can never clear `MIN_DOMAINS=2` — the T07.3 channel cannot propose alone — and
(b) one template site plus its own appearance in a SearXNG capture counts as two "domains",
re-opening the boilerplate flood the floor exists to stop. Fix: capture per-hit source domains
alongside titles and attribute federated evidence to its real domain.

### BUG-013 — Weak-top expansion trigger misfires under explicit sort — **open**
`search.rs` `top_result_is_weak` reads the first hit in engine order; under `?sort=recency` that is
the newest document, not the best match, so the expansion leg fires on nearly every recency-sorted
search. Fix: apply the weak-score check only when no explicit sort is set.

### BUG-014 — Nested anchors on the privacy page — **open**
`web/app/[lang]/privacy/page.tsx` wraps `Wordmark` (already a `next/link`) in a second `Link` —
invalid HTML, React DOM-nesting warning, hydration risk. Fix: use the bare `Wordmark`.

### BUG-015 — Gateway-client budget bounds only the request headers — **open**
`crates/xustive-api/src/federate.rs`: `tokio::time::timeout` wraps `send()` but `resp.json()` runs
outside it, bounded only by the 30s socket timeout — a trickled body holds the response far past
the budget. Fix: put the full send-and-decode future inside the timeout.

## Low

### BUG-016 — eval-ab: restore failure shadows the original error — **open**
A variant failure followed by a restore failure reports only the restore error; which variant
broke and why is dropped. Fix: chain/report both.

### BUG-017 — eval-ab: a late variant failure discards all completed scores — **open**
Scores for variants 1..N−1 survive only as scrolled stdout; no report, no delta table. Fix: print
the table and write the report for whatever completed before surfacing the error.

### BUG-018 — eval-ab: "wins" label and noise band are both wrong — **open**
Any `d > 0.0` is stamped "← wins" against a caption saying noise isn't a win, and the printed
`±0.010` treats the *relative* `NDCG_TOLERANCE` (1% of baseline) as absolute. Fix: stamp wins only
past the tolerance, and compute the band from the baseline.

### BUG-019 — eval regression gate can never fail on a malformed baseline — **open**
`eval.rs`: `baseline["ndcg_at_10"].as_f64().unwrap_or(0.0)` — a wrong-shaped `--baseline` file
reads as 0.0 and the gate goes permanently green. Fix: error on a baseline without the field.

### BUG-020 — Miner: token cap applies before token cleaning — **open**
`mine.rs`: `.take(24)` on raw whitespace tokens runs before `clean_token`, so stop-word/punctuation
heavy titles (and long federated query+title strings) exhaust the budget before content tokens.
Fix: cap after cleaning.

### BUG-021 — Miner: same-day rerun silently clobbers a half-reviewed candidates file — **open**
The `--out` help promises dated files "never overwrite a half-reviewed one", but the date is
day-granular. Fix: refuse to overwrite an existing candidates file (require `--out` or delete).

### BUG-022 — Expansion-leg hits render without highlighting — **open**
`search.rs` `expand_and_merge`'s query omits `.highlight(...)`, so expansion-only cards have no
`<em>` marks while primary-leg cards on the same page do. Fix: request the same highlights.

### BUG-023 — Eager-indexed federated docs get stamped with the query's language — **open**
`search.rs` `ingest_federated` sets `doc.language` from the *query's* detection; a French query
indexing an English page mislabels it until the full crawl. Fix: detect from the hit's own
title+snippet instead.

### BUG-024 — `interaction_tokens` map has no size cap — **open**
`search.rs`: TTL sweep but no `MAX_PENDING`-style cap (unlike `PendingStore`); a request flood
grows it without bound. Fix: cap with oldest-eviction, mirroring `PendingStore`.

### BUG-025 — Dense-recall failures recorded as "reinforce" — **open**
`search.rs`: when `fetch_by_ids` fails for a non-empty missing set the SEMANTIC_FUSED metric
records `kind=reinforce`, hiding failures from the dashboard. Fix: record a distinct outcome.

### BUG-026 — External LLM client follows redirects — **open**
`crates/xustive-federation/src/llm.rs`: default redirect policy; reqwest strips auth cross-host but
re-sends it on a same-host https→http downgrade and replays the prompt on 307/308. Fix:
`redirect::Policy::none()` — a provider API has no business redirecting.

### BUG-027 — Admin integrations page: toggle race + missing loading states — **open**
`web/app/(operator)/admin/integrations/page.tsx`: `busy` clears before the un-awaited `load()`
resolves (rapid re-click toggles on stale state); the external-summariser section renders a bare
heading when the fetch fails, and a stale doc comment contradicts shipped behavior. Fix: await the
reload; add loading fallbacks; drop the stale comment.

### BUG-028 — TS `Sentiment.confidence` doesn't exist in the API output — **open**
`web/lib/api.ts` requires `confidence: number`; Rust `SentimentOut` serializes only
`label`/`score`. Dishonest type — any future use passes tsc and breaks at runtime. Fix: remove it.

### BUG-029 — French instant-answer typo — **open**
`web/lib/i18n/messages.ts`: `'Résultats d examen'` → `'Résultats d'examen'`.

### BUG-030 — Privacy page unreachable from the results page — **open**
Only the home footer links `/{lang}/privacy`; someone landing on a shared results URL has no path
to it. Fix: add the privacy link to the results-page footer area.

### BUG-031 — Federation strip wait ignores the request deadline; page 1 exceeds `hits_per_page` — **wontfix (documented)**
The 1500ms strip wait is by design independent of the search deadline (federation is additive and
the wait is its own explicit budget), and web cards appended beyond `hits_per_page` are the
intended "extra recall on page 1" behavior. Recorded here so the choice is explicit; the
`pagination.hits_per_page` field describes the local page size, not the card count.

### BUG-032 — Miner pair map unbounded before filtering — **open**
`mine.rs` counts every cross-script pair before `min_count`/PMI filters run; a corpus-scale run can
hold millions of entries. Fix: document the `--max-docs` bound as the memory control and prune
sub-`min_count` pairs periodically during the scan.

## Excluded — user-privacy class (operator decision, 2026-08-25, not tracked here)

- Query text leaks into gateway/discover logs via reqwest error URLs (SearXNG/Brave GET `?q=`).
- Prod Redis AOF + off-host backups defeat the windowed/unchainable search-history model.
- `discovery.k_anonymity` claimed clamped to 20, actually `max(1)`, unvalidated; weak-coverage on
  by default; `interaction.guard()` runs only in the API binary.
- `qhash` comment overstates FNV-1a irreversibility; spec'd salted HMAC unimplemented.
- Sliding TTL doubles as a per-term last-event timestamp.
- ADR-0008's "nightly log scan" (scan-logs.sh) is not automated anywhere.
- Privacy page omits the federation egress (queries → SearXNG → upstream engines).
- `EXTERNAL_LLM_KEY` visible in `docker inspect` (compose secret would remove it).
