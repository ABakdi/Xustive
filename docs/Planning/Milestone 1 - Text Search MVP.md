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
> Parent: [[TODO]] · Previous: [[Milestone 0 - Foundations]] · Next: [[Milestone 2 - Multimodal Input]], [[Milestone 3 - Ingestion at Scale]]

---

## Why This Milestone Exists

This is where the product's actual differentiator gets built: **Darija works**. Everything else here
is standard search engineering; the language work is not, and it is where the time will go.

The corpus is still the M0 fixture set — real crawling comes in M3. That is deliberate: tuning
ranking against a stable corpus is possible, tuning it against a corpus that changes hourly is not.

---

## M1-T01 — `xustive-telemetry`

- [ ] M1-T01.1 `tracing` subscriber, JSON output, per-target level control
- [ ] M1-T01.2 Prometheus registry and the metric names in [[Observability]] §2
- [ ] M1-T01.3 Span helpers that make it *hard* to attach a query string
- [ ] M1-T01.4 `POST /admin/log-level` with 15-minute auto-revert
- [ ] M1-T01.5 Nightly log-scan job against the query corpus ([[Security and Privacy]] §1)

## M1-T02 — [[API Gateway]] middleware stack

- [ ] M1-T02.1 The ten layers in [[API Gateway]] §3, **in order**
- [ ] M1-T02.2 Per-route body limits and timeouts
- [ ] M1-T02.3 Rate limiter with salted, rotating IP-hash keys ([[Security and Privacy]] P5)
- [ ] M1-T02.4 Security headers, CSP snapshot test
- [ ] M1-T02.5 Load shedding: expensive routes shed before cheap ones
- [ ] M1-T02.6 Contract tests for every row of [[API Contract]] §8

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

- [ ] M1-T05.1 Normalisation + operator parsing (`"…"`, `site:`, `-term`)
- [ ] M1-T05.2 Deadline propagation as an absolute `Instant`
- [ ] M1-T05.3 Multi-search request builder (primary / expanded / comments legs)
- [ ] M1-T05.4 Merge, dedupe by id, comment grouping, parent fetch
- [ ] M1-T05.5 Degradation ladder with per-call timeouts ([[Error Handling and Resilience]] §6)
- [ ] M1-T05.6 Summary candidate selection and token handoff
- [ ] M1-T05.7 Fault-injection tests: every optional dependency fails, request still succeeds

## M1-T06 — [[Ranking and Relevance]]

- [x] M1-T06.1 Meilisearch ranking rules and typo-tolerance settings
- [x] M1-T06.2 Stage-2 scoring formula with hot-reloadable weights (`config/ranking.toml`)
- [x] M1-T06.3 Freshness τ by inferred query intent
- [x] M1-T06.4 Diversity: per-domain cap, per-author cap, source-type spread
- [x] M1-T06.5 SimHash collapse at result time
- [~] M1-T06.6 Per-signal `Explain` struct is computed and returned; the `--explain` CLI
      surface is not wired up yet
- [ ] M1-T06.7 Weight tuning against the golden set — blocked on M1-T15.5

## M1-T07 — [[Sentiment Engine]] (lexicon mode)

- [x] M1-T07.1 VADER-style scorer with negation, intensifiers, diminishers, emoji, elongation
- [x] M1-T07.2 Lexicon files ×4 with loader and hot-reload
- [x] M1-T07.3 Confidence from lexicon coverage; force `neutral` below threshold
- [~] M1-T07.4 Darija sentiment lexicon at ~50 terms (target 2 000) ← **blocked on native
      speakers**, two reviewers required; B7
- [ ] M1-T07.5 Labelled set of 1 000 items ← **blocked on annotators**; B7
- [ ] M1-T07.6 Calibration check — blocked on M1-T07.5

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

- [ ] M1-T09.1 Entity FST built nightly, swapped atomically
- [ ] M1-T09.2 Title-prefix leg against Meilisearch
- [ ] M1-T09.3 Transliteration suggestions via [[Query Expander]]
- [ ] M1-T09.4 Curated list (`data/suggest/curated.tsv`)
- [ ] M1-T09.5 Merge, dedupe, prefix-subsumption
- [ ] M1-T09.6 p95 ≤ 40 ms at 2 000 rps
- [ ] M1-T09.7 Degradation: works with Meilisearch down

## M1-T10 — [[Content Parser]]

- [x] M1-T10.1 Extraction cascade: JSON-LD → OG → readability → domain rules → fallback
- [x] M1-T10.2 Boilerplate removal and link-density heuristics
- [x] M1-T10.3 **Date extraction** including Arabic (أوت, جويلية) and French formats, relative forms,
      DD/MM disambiguation, `unknown` precision handling
- [x] M1-T10.4 Derived fields: excerpt, `content_hash`, `simhash`, entities, canonical URL, media
- [ ] M1-T10.5 Per-domain rules format + rules for the top 20 sources, each with a fixture test
- [ ] M1-T10.6 Adversarial DOM suite (depth, node count, encodings, bombs)
- [ ] M1-T10.7 200-page labelled corpus: ≥ 90 % title, ≥ 85 % date, ≥ 0.9 body F1

## M1-T11 — [[Indexer Worker]]

- [ ] M1-T11.1 Batching by size, bytes, and timeout
- [ ] M1-T11.2 Submit → poll task → **ack last**
- [ ] M1-T11.3 Split-on-failure isolation
- [ ] M1-T11.4 Pre-submit validation table
- [ ] M1-T11.5 Deletion path (vectors → comments → document → blocklist)
- [ ] M1-T11.6 Crash-safety test: kill between submit and ack, assert no loss or duplication

## M1-T12 — [[Task Queue]]

- [ ] M1-T12.1 Streams abstraction: produce, consume-group, ack, DLQ
- [ ] M1-T12.2 `XAUTOCLAIM` reclaim loop with a 5-minute idle window
- [ ] M1-T12.3 Trim maintenance task
- [ ] M1-T12.4 `noeviction` config + a test asserting nothing is evicted at `maxmemory`
- [ ] M1-T12.5 Queue-depth and lag metrics
- [ ] M1-T12.6 `make dlq` stats / peek / replay

## M1-T13 — Core UI

- [ ] M1-T13.1 [[UI - Design System]] tokens, light and dark
- [ ] M1-T13.2 [[UI - Component Library]]: SearchBox, SuggestionList, SummaryBlock, ResultCard,
      FilterChip, Pagination, Sheet, Toast, Skeleton, EmptyState
- [ ] M1-T13.3 [[UI - Home Page]]
- [ ] M1-T13.4 [[UI - Results Page]] with the two-request render sequence
- [ ] M1-T13.5 [[UI - Filters and Facets]]
- [ ] M1-T13.6 [[UI - States and Errors]] for every row of its §4
- [ ] M1-T13.7 URL-as-state, back/forward correctness
- [ ] M1-T13.8 Bundle budgets enforced in CI; Lighthouse CI on a throttled profile
- [ ] M1-T13.9 **CLS ≤ 0.05** verified across the streaming sequence

## M1-T14 — [[UI - RTL and Localization]]

- [ ] M1-T14.1 Logical-property CSS throughout + a lint rejecting physical properties
- [ ] M1-T14.2 `dir="auto"` on every content slot; `<bdi>` around URLs and numbers in RTL
- [ ] M1-T14.3 String files for ar / ary / fr / en with `Intl.PluralRules`
- [ ] M1-T14.4 Algerian month names (أوت, جويلية) in formatting **and** parsing
- [ ] M1-T14.5 Directional icon mirroring, logo exclusion
- [ ] M1-T14.6 Visual regression snapshots: 4 languages × 2 directions × 2 themes
- [ ] M1-T14.7 Native-speaker review of `ar` and `ary` strings ← *B7*

## M1-T15 — Evaluation harness

- [ ] M1-T15.1 `make eval` runs the golden set against a frozen index snapshot
- [ ] M1-T15.2 nDCG@10, MRR@10, recall@50, zero-result rate, freshness distribution
- [ ] M1-T15.3 Reports to `eval/reports/{date}.json`, plotted over time
- [ ] M1-T15.4 CI gate: ranking or lexicon changes cannot drop nDCG@10 by > 1 %
- [ ] M1-T15.5 **Golden set v1: 200 queries** — 40 % ar, 25 % ary, 20 % fr, 10 % en, 5 % mixed, judged
      by Algerian-native judges ← *B7*

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
| Ranking tuned to the fixture corpus, not reality | re-run the full evaluation after M3 when real content arrives |
| Golden set judged by non-native speakers | explicitly blocked on B7; do not substitute |

## Related

[[TODO]] · [[Milestone 0 - Foundations]] · [[Ranking and Relevance]] · [[Query Expander]] ·
[[Summarizer]] · [[Testing Strategy]] · [[UI Specification]]
