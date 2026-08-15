---
tags:
  - planning
  - milestone
milestone: 1
status: not-started
updated: 2026-08-06
---

# Milestone 1 - Text Search MVP

> **Goal:** the complete text search product — multilingual query understanding, ranking, sentiment,
> AI summary, and a real UI. If the project stopped here, it would still be useful.
> **Exit gate:** nDCG@10 ≥ 0.60 on the golden set; `/search` p95 ≤ 200 ms; summary faithfulness
> ≥ 95 %.
> Parent: [[TODO]] · Previous: [[Milestone 0 - Foundations]] · Next: [[Milestone 3 - Multimodal Input]], [[Milestone 2 - Ingestion at Scale]]

---

## Why This Milestone Exists

This is where the product's actual differentiator gets built: **Darija works**. Everything else here
is standard search engineering; the language work is not, and it is where the time will go.

The corpus is still the M0 fixture set — real crawling comes in M2. That is deliberate: tuning
ranking against a stable corpus is possible, tuning it against a corpus that changes hourly is not.

---

## M1-T01 — `xustive-telemetry`

- [x] M1-T01.1 `tracing` subscriber, JSON output, per-target level control
- [x] M1-T01.2 Prometheus registry and the metric names in [[Observability]] §2
- [~] M1-T01.3 Span helpers that make it *hard* to attach a query string — the metrics registry
      only accepts `&'static str` label names, `query_len_bucket` is the sanctioned substitute
      for the query itself, and `lint-telemetry.sh` rejects query identifiers inside `tracing::`
      macros. Dedicated span-constructor helpers are not built; the type-level and lint barriers
      are what exist
- [x] M1-T01.4 `POST /admin/log-level` with 15-minute auto-revert
- [x] M1-T01.5 Log-scan job against the query corpus (`scripts/scan-logs.sh`), verified against
      the real server at debug level replaying every probe query — clean. Not yet *scheduled*
      nightly; that arrives with the deployment work

## M1-T02 — [[API Gateway]] middleware stack

- [x] M1-T02.1 The ten layers in [[API Gateway]] §3, **in order**
- [x] M1-T02.2 Per-route timeouts; body limits are global at the default. A single outer timeout
      silently capped every route at the search budget and turned every summary into a 504,
      which is why these are now per group
- [x] M1-T02.3 Rate limiter with salted, rotating IP-hash keys ([[Security and Privacy]] P5)
- [x] M1-T02.4 Security headers, CSP snapshot test — pinned exactly, not checked for presence
- [x] M1-T02.5 Load shedding: global in-flight cap sheds with 503 rather than queueing. Per-route
      shed *ordering* — expensive before cheap — is approximated by summaries having their own,
      much tighter rate limit rather than by a priority-aware shedder
- [x] M1-T02.6 Contract tests for every row of [[API Contract]] §8 that can be provoked without a
      live backend (20 tests). Rows needing voice, image or a live index are not covered

## M1-T03 — [[Language Detector]]

- [x] M1-T03.1 Script detection by Unicode block ratio
- [x] M1-T03.2 Statistical detection restricted to ar/fr/en — **`whatlang`, not `lingua-rs`**
      (see [[Language Detector]] §4.2 for why)
- [x] M1-T03.3 Darija marker lexicon loader (`data/lang/`, hot-reload)
- [x] M1-T03.4 Arabizi marker detection (digit-consonants + token list)
- [x] M1-T03.5 Short-query confidence scaling; `Und` as the safe default
- [~] M1-T03.6 Labelled set at 59 strings (target 1 000); 100 % overall, 100 % `ary`. The
      number is partly self-confirming — the set was written alongside the lexicons. A
      native-speaker set is what would make it meaningful.
- [~] M1-T03.7 Darija marker lexicon at ~200 terms (target 1 500) ← *needs a native
      speaker to review what is there and extend it; blocker B7*

## M1-T04 — [[Query Expander]]

- [x] M1-T04.1 Arabizi ↔ Arabic transducer with a lattice and bigram scoring
- [x] M1-T04.2 Guardrails: min length, French homographs, quoted spans untouched
- [x] M1-T04.3 Lexicon format and loader (`data/expansion/*.tsv`)
- [x] M1-T04.4 Entity lexicon: 58 wilayas, institutions, operators, banks, universities
- [x] M1-T04.5 Domain synonym lexicon: administrative, employment, transport, health
- [x] M1-T04.6 Variant capping and weighting
- [x] M1-T04.7 Meilisearch `synonyms` generated from the same lexicon at deploy time
- [~] M1-T04.8 Recall measured ad hoc, not gated. سونلغاز 551→834 (+51 %), وهران 453→740
      (+63 %). A real gate needs the judged golden set from M1-T15.
- [ ] M1-T04.9 DziriBERT fallback behind a feature flag (optional, default off)

## M1-T05 — [[Query Pipeline]]

- [x] M1-T05.1 Operator parsing — `"phrase"` (ASCII, typographic and French quotes), `site:`
      (accepts a pasted URL), `-term`. Parsed from the **raw** query, before normalisation folds
      the marks that made a phrase a phrase
- [x] M1-T05.2 Deadline propagation as an absolute `Instant`. A duration passed down the chain
      gives every stage the full budget, so four stages each "within budget" take four times it
- [~] M1-T05.3 Multi-search request builder — primary and **expanded** legs are wired and
      measured; the comments leg is not built. Expansion runs only when the primary returns
      fewer than five hits, since a query that already retrieved well gains only weaker matches
- [~] M1-T05.4 Merge and dedupe by id across legs, primary order preserved — a document found
      by both matched the query *as typed*, which is stronger evidence than matching a
      transliteration of it. Comment grouping and parent fetch are not built
- [x] M1-T05.5 Degradation ladder: summary → expansion → facets → re-ranking, each with a
      budget fraction rather than a fixed floor. Retrieval is never dropped — a search returning
      nothing is not degraded, it is indistinguishable from an outage
- [x] M1-T05.6 Summary candidate selection and token handoff
- [x] M1-T05.7 Fault injection: 10 tests with Meilisearch, Redis and the model all unreachable.
      The process starts, liveness holds, readiness fails honestly, suggestions and `/admin` still
      answer

## M1-T06 — [[Ranking and Relevance]]

- [x] M1-T06.1 Meilisearch ranking rules and typo-tolerance settings
- [x] M1-T06.2 Stage-2 scoring formula with hot-reloadable weights (`config/ranking.toml`)
- [x] M1-T06.3 Freshness τ by inferred query intent
- [x] M1-T06.4 Diversity: per-domain cap, per-author cap, source-type spread
- [x] M1-T06.5 SimHash collapse at result time
- [x] M1-T06.6 Per-signal `Explain` struct is computed and returned, and `search --explain` prints
      it. Ranked through the same `rerank` the API uses rather than Meilisearch's own order — the
      question it answers is why a result sits where it does, which is only meaningful about the
      order a user sees. Age carries a not-trusted marker when the date was inferred, because a
      bare `0 days` on a guessed date reads as "published today"
- [ ] M1-T06.7 Weight tuning against the golden set — blocked on M1-T15.5

## M1-T07 — [[Sentiment Engine]] (lexicon mode)

- [x] M1-T07.1 VADER-style scorer with negation, intensifiers, diminishers, emoji, elongation
- [x] M1-T07.2 Lexicon files ×4 with loader and hot-reload
- [x] M1-T07.3 Confidence from lexicon coverage; force `neutral` below threshold
- [~] M1-T07.4 Darija sentiment lexicon at **158 terms** (target 2 000), machine-generated and
      unreviewed. Arabic-script Darija now lives in `ar.tsv` as its header always said it should —
      without those rows a Darija comment written in Arabic scored as unremarkable MSA, so مليح,
      واعر, خايب and حقرة carried no weight at all. The politically-charged rows are flagged for a
      reviewer to check first. ← *still wants native speakers*, two reviewers; B7
- [~] M1-T07.5 Labelled set at **65 items** (target 1 000), machine-generated. Neutral rows are
      over-represented relative to a real corpus on purpose: a lexicon scorer's characteristic
      failure is finding sentiment in ordinary factual text, and a set of clearly-polar sentences
      cannot detect that at all. 86.2 % accuracy, neutral held 100 %, polarity never inverted —
      every remaining miss is polar → neutral, which declines to judge rather than judging wrong.
      ← *still wants annotators*; B7
- [x] M1-T07.6 Calibration check. Asserts **ordering**, not probability — a set this size cannot
      support a probability claim, and asserting one would measure noise. If confidence is no
      higher when right than when wrong, every downstream filter on it is decorative. Currently
      0.436 against 0.248

## M1-T08 — [[Summarizer]]

- [x] M1-T08.1 `llama-cpp-2` integration, model loading, GGUF magic check on download
- [x] M1-T08.2 Passage preparation: match-centred truncation, quality filtering, context cap
- [x] M1-T08.3 Prompt with untrusted-content framing ([[Security and Privacy]] §5)
- [ ] ~~M1-T08.4 Streaming over SSE~~ — **dropped, deliberately.** §4.5 validates *after*
      generation, so streamed tokens would show text that validation then rejects. Replaced by a
      second request after the results render; see [[Summarizer]] §3. Client abort still frees
      the slot, since a closed reply channel is checked before generation starts.
- [x] M1-T08.5 Output validation: `INSUFFICIENT`, URL/email/phone rejection, citation
      requirement, language check, length and sentence caps
- [x] M1-T08.6 Bounded queue; shed rather than queue under load
- [x] M1-T08.7 Injection fixture — one hostile-passage case runs against the real model. Not yet
      a *suite*, and it is skipped when the weights are absent, so it does not gate CI
- [ ] M1-T08.8 **Faithfulness evaluation: 100 cases, ≥ 95 %, human-sampled** — blocked on B7
- [x] M1-T08.9 **Decide B4.** Quality: 3B is enough — grounded, correctly cited MSA on real
      crawled pages, refuses when passages do not answer, resisted an injected instruction.
      Latency: it is not — 27 s on CPU against a 2.5 s budget ([[Summarizer]] §8). The decision
      moves to GPU offload rather than a different model
- [x] M1-T08.10 Runtime device selection with `/admin` page, GPU with CPU fallback that never
      fails to start ([[Deployment Topology]])
      ([[ADR-0005 - Local Quantised LLM for Summaries]])

## M1-T09 — [[Autocomplete Service]]

- [~] M1-T09.1 A sorted-vector prefix index, not an FST, swapped atomically behind an `RwLock`.
      An FST wins decisively at the ~200k strings that design assumed; at the 436 we have, a
      sorted vector answers in the same microseconds with no build step. Rebuilt at startup, not
      nightly — the scheduler arrives with the deployment work
- [x] M1-T09.2 Title-prefix leg against Meilisearch, restricted to `title` and filtered to
      results that actually contain the prefix. Typo tolerance is right for search and wrong
      here: it offered "فيديو" for "وهر"
- [x] M1-T09.3 Transliteration suggestions via [[Query Expander]], gated on the prefix *looking*
      Arabizi. Applied to every Latin prefix it helped Darija typists and harmed French ones —
      "Ora" is Oran, and transliterating it returned unrelated Arabic
- [x] M1-T09.4 Curated list (`data/suggest/curated.tsv`) — 45 entries: wilayas, administrative
      procedures, utilities. These are what people need and what a news corpus never contains
- [x] M1-T09.5 Merge, dedupe on the folded form, prefix-subsumption, stable ordering
- [~] M1-T09.6 Measured 0–16 ms per request against the 40 ms p95 budget, but **not under load**
      — no 2 000 rps test has been run
- [x] M1-T09.7 Degradation: three of four sources are in-memory, and the title leg has its own
      60 ms timeout and is skipped silently. Suggestions survive Meilisearch being down

## M1-T10 — [[Content Parser]]

- [x] M1-T10.1 Extraction cascade: JSON-LD → OG → readability → domain rules → fallback
- [x] M1-T10.2 Boilerplate removal and link-density heuristics
- [x] M1-T10.3 **Date extraction** including Arabic (أوت, جويلية) and French formats, relative forms,
      DD/MM disambiguation, `unknown` precision handling
- [x] M1-T10.4 Derived fields: excerpt, `content_hash`, `simhash`, entities, canonical URL, media
- [~] M1-T10.5 Per-domain rules in `data/parsers/domains.toml` — 12 sources, applied before
      generic extraction, subdomains inheriting their parent. aps.dz dated documents went 13 → 27.
      **Only aps.dz has a saved fixture**; the other date rules are unverified selectors and the
      test prints which, so the gap is known rather than invisible
- [x] M1-T10.6 Adversarial DOM suite — 13 cases. It found a real denial of service: 50 000
      nested divs took **47 seconds** and 20 000 unclosed tags took 18. A pre-parse complexity
      guard (bytes, tag count, nesting depth) takes the whole suite to 0.09 s
- [ ] M1-T10.7 200-page labelled corpus: ≥ 90 % title, ≥ 85 % date, ≥ 0.9 body F1 ← *B7*.
      Labelling 200 pages by hand is annotation work, not engineering; the fixture harness it
      would run against exists

## M1-T11 — [[Indexer Worker]]

- [x] M1-T11.1 Batching by count, **bytes** and timeout — a batch of long articles hits the byte
      ceiling long before the count, and the engine's limit is bytes
- [x] M1-T11.2 Submit → poll task → **ack last**. Acknowledging on submit would be faster and
      wrong: Meilisearch returns a task id before the write happens
- [x] M1-T11.3 Split-on-failure isolation by bisection — one bad document in sixteen is isolated
      in fewer than sixteen submissions and the other fifteen land
- [x] M1-T11.4 Pre-submit validation: missing id, not an object, empty, oversized. Caught here
      because a batch the engine rejects takes the good documents with it
- [ ] M1-T11.5 Deletion path (vectors → comments → document → blocklist) — needs the vector
      index, which is M3
- [x] M1-T11.6 Crash safety: a worker that consumes without acknowledging leaves its job pending
      and recoverable; redelivery overwrites rather than duplicating because writes are keyed by id

## M1-T12 — [[Task Queue]]

- [x] M1-T12.1 Streams abstraction: produce, consume-group, ack, DLQ
- [x] M1-T12.2 `XAUTOCLAIM` reclaim with a 5-minute idle window, run before consuming so a
      crashed worker's jobs do not wait behind a fresh backlog
- [x] M1-T12.3 Approximate trimming on write **and** on each worker pass — a queue that stops
      receiving work stops trimming itself
- [x] M1-T12.4 `noeviction` is set in compose **and asserted**. Under any `allkeys-*` policy Redis
      reclaims memory by deleting keys of its own choosing, and the stream is an ordinary key —
      queued documents would vanish with no error, no dead letter and no visible gap, leaving an
      index quietly smaller than the crawl claims. Skips rather than fails where `CONFIG GET` is
      refused: being unable to read the setting is not evidence it is wrong
- [x] M1-T12.5 `depth()`, `pending()` and `dead_count()` are exposed by `make dlq` **and as
      Prometheus gauges**. Sampled at scrape time rather than pushed from the indexer: the indexer
      is not always running, and a backlog nobody is draining is exactly what wants alerting on, so
      a gauge that only updates while a worker lives goes silent when it matters. Every failure is
      silent — `/metrics` shares a port with the liveness probe
- [x] M1-T12.6 `make dlq` stats / peek / replay. Replay is always deliberate — a queue that
      retries its own poison on a timer will do it at 3am after someone fixed the bug

> **UI work moved to [[Milestone 1B - Frontend and Instant Answers]].** The items below that are
> ticked were delivered there; the rest stay here because they are gates (budgets, visual
> regression) rather than features, and gates belong with the milestone that defines them.

## M1-T13 — Core UI

- [x] M1-T13.1 [[UI - Design Language]] tokens, light and dark — delivered by M1B-T02.1
- [~] M1-T13.2 [[UI - Component Library]]: SearchBox, **SuggestionList**, SummaryBlock, ResultCard
      are built. Remaining:
      FilterChip, Pagination, Sheet, Toast, Skeleton, EmptyState
- [x] M1-T13.3 [[UI - Home Page]] — delivered by M1B-T03.1
- [x] M1-T13.4 [[UI - Results Page]] with the two-request render sequence — delivered by
      M1B-T03.2 and M1B-T03.5
- [x] M1-T13.5 [[UI - Filters and Facets]] — language, source and tone chips, rendered both
      server-side and client-side so narrowing works without JavaScript. Chips toggle, preserve
      each other, and stay visible while active so a filter can always be cleared
- [ ] M1-T13.6 [[UI - States and Errors]] for every row of its §4
- [x] M1-T13.7 URL-as-state — the query string *is* the state; no client store to desync
- [~] M1-T13.8 Bundle budgets enforced by `make ui-gates`, measuring what a browser actually
      downloads. **The budgets had to be raised from 40/90 KB to 185/195** — React and Next are
      ~152 KB gzipped before any of our code, which ADR-0010 failed to state. Lighthouse CI on a
      throttled profile is not set up
- [ ] M1-T13.9 **CLS ≤ 0.05** verified across the streaming sequence

## M1-T14 — [[UI - RTL and Localization]]

- [~] M1-T14.1 Logical-property CSS throughout — the suggestion panel uses `inset-inline` and
      `inset-block-start`, so it aligns to the input's start edge in Arabic and French with no
      direction-specific rule. The lint rejecting physical properties is not built
- [~] M1-T14.2 `dir="auto"` on every content slot including suggestions and the summary;
      `<bdi>` around URLs and numbers in RTL is not done
- [~] M1-T14.3 Strings for ar / fr / en covering the filter and suggestion chrome, keyed off
      `documentElement.lang`, with Darija falling back to Arabic rather than English. Not yet
      extracted to files, and `Intl.PluralRules` is not used
- [x] M1-T14.4 Algerian month names in both directions: parsed by `xustive-ingest::date` and
      rendered by `xustive-tools::datetime`
- [ ] M1-T14.5 Directional icon mirroring, logo exclusion
- [ ] M1-T14.6 Visual regression snapshots: 4 languages × 2 directions × 2 themes
- [ ] M1-T14.7 Native-speaker review of `ar` and `ary` strings ← *B7*

## M1-T15 — Evaluation harness

- [~] M1-T15.1 `make eval` runs the golden set. **Not against a frozen snapshot** — snapshotting
      Meilisearch is not wired up. Instead the golden set records the corpus size its judgements
      were made against and the runner refuses to gate when the live index has drifted, because
      documents added after judging count as irrelevant and look exactly like a ranking
      regression. Detecting the drift is the honest approximation; freezing is still owed
- [x] M1-T15.2 nDCG@10, MRR@10, recall@50, zero-result rate, freshness distribution, plus a
      per-language breakdown the spec did not ask for and which turned out to be the finding
- [x] M1-T15.3 Reports to `eval/reports/{date}.json`. Not yet *plotted*; the series exists
- [x] M1-T15.4 CI gate: `make eval-check` fails when nDCG@10 drops more than 1 % relative to
      `eval/reports/baseline.json`. Verified in both directions
- [~] M1-T15.5 Golden set v1: **187 queries**, not 200 — every query needs at least one document
      graded 2 or better, and the corpus cannot answer more. Mix is 43 % ar / 27 % ary / 21 % fr /
      4 % en / 5 % mixed; English is short because only a handful of English documents are
      indexed. **Machine-judged**, marked as such per query, and reports print what share of the
      score rests on generated labels. Native-speaker judging still owed ← *B7*

---

## Exit Gate

| Check | Threshold |
|:---|:---|
| Relevance | nDCG@10 ≥ 0.60 on the golden set |
| Darija recall | expansion lifts recall@50 by ≥ 15 % on the Darija slice |
| Latency | `/search` p95 ≤ 200 ms at 100 rps on the fixture corpus |
| Suggest | p95 ≤ 40 ms |
| Summary | faithfulness ≥ 95 %; TTFT p95 ≤ 800 ms; injection suite passing |
| Language detection | ≥ 92 % overall, ≥ 85 % `ary` |
| Sentiment | macro-F1 ≥ 0.70, no language below 0.60 |
| UI | CLS ≤ 0.05, LCP ≤ 2.0 s throttled, bundle within budget |
| Degradation | every optional dependency can fail without failing a search |

## Risks

| Risk | Mitigation |
|:---|:---|
| Lexicon work is unglamorous and gets deferred | it is on the critical path for the exit gate, and B5/B7 name it as a people problem |
| ~~3B summaries are not good enough in Arabic~~ | **Resolved.** Quality is adequate; speed is not. The live risk is now latency, tracked in [[Summarizer]] §8 |
| Ranking tuned to the fixture corpus, not reality | re-run the full evaluation after M2 when real content arrives |
| Golden set judged by non-native speakers | explicitly blocked on B7; do not substitute |

## Related

[[TODO]] · [[Milestone 0 - Foundations]] · [[Ranking and Relevance]] · [[Query Expander]] ·
[[Summarizer]] · [[Testing Strategy]] · [[UI Specification]]
