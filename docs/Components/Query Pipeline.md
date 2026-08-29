---
tags:
  - component
  - serving
component-id: C02
binary: xustive-api
status: built
updated: 2026-08-27
---

# Query Pipeline

> **ID** C02 · **Binary** `xustive-api` (`search.rs` handler, `deadline.rs`; ranking in
> `xustive-search::rank`) · **Upstream** [[API Gateway]] · **Downstream** [[Instant Answers]],
> [[Language Detector]], [[Query Expander]], [[Search Index]], [[Vector Index]],
> [[Federation Gateway]], [[Interaction Signals]], [[Summarizer]]

## 1. Purpose

Orchestrate one search request from a raw string to a ranked, faceted result set. It is the only
component that knows the *order* of search operations, and the only place the composition of
tools → detection → retrieval → expansion → fusion → re-ranking → federation lives.

## 2. Responsibilities

**In scope**: validation; operators (`"…"`, `site:`, `-term`); instant-answer matching;
normalisation; detection; typed filters and verticals; the deadline ladder; primary retrieval
with narrowing under load; the stop-word rescue; the conditional expanded leg; dense fusion;
anonymous CTR lookup; stage-2 ranking; the live-web strip; facets; the summary handoff; related
searches; interaction capture.

**Out of scope**: HTTP framing and rate limits (→ [[API Gateway]]); index settings
(→ [[Search Index]]); summary generation (→ [[Summarizer]]); lexicon content
(→ [[Query Expander]]).

## 3. Interface

`GET /api/v1/search` — one `async fn handler` in `crates/xustive-api/src/search.rs`, no trait.

```
q, page, hits_per_page, lang (filters *and* overrides detection; "auto" = detect),
source, sentiment, from, to, sort (relevance | recency), v (all | news | files | images | videos),
ui (interface language: tool labels, summary language, the ui_language ranking signal)
```

Response: `query_info { raw, normalized, language, language_confidence, expanded_terms,
corrected }`, `results: [ResultCard]`, `facets`, `pagination { …, estimated }`, `instant`,
`summary_token`, `is_question`, `related`, `interaction_token`, `took_ms`, degradation flags.
`corrected` is always `None` — there is no spell corrector (2026-08-27).

The budget is one absolute `Deadline` created at the top from `api.timeout_search_ms`; passing a
duration down the chain would let every stage believe it had the whole thing.

## 4. Internal Design

### 4.1 Before retrieval

1. **Question and instant answers** read the *raw* query: normalisation folds the `?` and the
   characters an expression like `45*1.19` is made of. `is_question` decides where the summary
   goes; `xustive_tools::best_in` (pure matchers), then currency, then weather
   ([[Instant Answers]]) — cache-backed tools only when nothing pure matched, so a search that is
   not about weather never pays a Redis round trip.
2. **Operators** come off the raw query too — a `"phrase"` parsed after normalisation is no
   longer a phrase. Three operators only, no boolean grammar. `site:` becomes the `domain`
   filter, so the URL a user shares carries what they typed.
3. **Normalise** the engine query with `xustive_text::normalize` — the same function the
   detector, scorer and expander use ([[Content Parser]] §4.5 for the index side).
4. **Detect** (`detect_normalized`); `?lang=` short-circuits with confidence 1.0.
5. **Filters** are typed (`xustive_search::filter::Filters`): source types, sentiment labels,
   languages, date range (a reversed range is swapped, and any range excludes guessed dates),
   `exclude_spam` at `SPAM_THRESHOLD` 0.8. Verticals are saved filters over the one index: `news`
   = web with a known date, `files` = PDFs, `images`/`videos` = pages carrying that media kind.

### 4.2 The deadline ladder (`deadline.rs`)

Stages are given up in this order as the budget runs out, each dropped whole rather than rushed:

| Stage | What is lost |
|:---|:---|
| `Summary` | already a separate request, nearly free to abandon |
| `Expansion` | the second leg *and* the dense leg — costs Arabizi queries their results |
| `Facets` | the filter chips |
| `Rerank` | the engine's own order is returned |
| `Retrieval` | never dropped — but see narrowing |

Each drop increments `xustive_degraded_total{stage}`.

### 4.3 Retrieval

One `documents` query for a **candidate pool** (`candidate_pool` 200, deepened to reach the
requested page, capped by `MAX_TOTAL_HITS` 2000): `_rankingScore` on, highlights on `excerpt`
and `title`, facets `source_type`, `sentiment.label`, `language` if the ladder allows. Paging
happens *after* re-ranking by skipping into the ranked pool — paging at the engine would freeze
the engine's order into the result (BUG-002).

**Narrow rather than fail (BUG-041).** On `SearchError::Timeout`, retry once with a page's worth
of hits, no facets, no highlights. While Meilisearch is also indexing a crawl backlog the
200-candidate query takes several hundred milliseconds; a bare page answers in ~30 ms even then.
Worse ranking and no chips — but results, and the reader cannot tell an empty search from an
outage. Flagged in the response.

**Stop-word rescue (M7-T01.5).** A query made entirely of stop words (`the and`, `من في`) is
stripped to nothing by the tokeniser. If the pool is empty and `is_all_stop_words` (the *same*
list the index is configured with, ≤ 6 tokens), re-issue it quoted: Meilisearch keeps stop words
inside a phrase.

### 4.3.1 "Did you mean" — spelling correction (`spell.rs`, 2026-08-29)

Meilisearch tolerates typos *inside* retrieval; what it cannot do is tell the reader that another
spelling would have found better pages, or show them those pages when what they typed found
nothing. Two parts, and the second is the one that decides:

**The vocabulary.** Word → number of *documents* it appears in, built in the background
(`AppState::refresh_spelling`, at startup and hourly) from the titles and excerpts of up to
200 000 documents sampled across the whole index, plus the wilaya names as a seed, plus the
queries readers ran that found something ([[ADR-0030]]). Three rules that were each paid for by
a wrong correction:

- **Counted once per document.** A word repeated down a page's navigation is one page's opinion;
  counting every occurrence let a single misspelled site outvote the correct spelling.
- **Past queries reinforce, never introduce.** A misspelled query returns results — that is what
  typo tolerance is for — so feeding query words in blind teaches the vocabulary every typo
  anyone typed. Seven test searches for `tlemcan` gave it a frequency above the real `tlemcen`.
- **The wilayas are seeded above every threshold.** The corpus holds more of the English and
  French web than of the Algerian one; without the seed, `alger` looks like a misspelling of
  `aller`.

**The corrector.** Each token is compared on its *shape* — Arabic orthographic variants folded
([[Language Detector]]'s `fold`) and Latin accents removed — so `algerien` and `algérien` are one
word, and a reader typing without accents is never "corrected" into a different language. A token
is replaced by the most frequent word within Damerau–Levenshtein distance 1 (2 for words over six
shape-characters) that satisfies:

| Guard | Value | Because |
|:---|:---|:---|
| First letter never changes | — | `wehran` (Oran, in Arabizi) is one edit from `tehran` |
| Unknown token (< 4 documents) | candidate ≥ 4 documents | the token is noise, not a word |
| Known token (≥ 4 documents) | candidate ≥ **20×** the token | `hopital` (3) vs `hospital` (13), `universite` (21) vs `university` (138) — frequency alone cannot tell a rare French word from a typo; twenty times can |
| Very common token (≥ 200 documents) | never corrected | at that point it is simply a word |

**The search decides.** The corrector only proposes; a second retrieval of the proposal settles
it, on page one, with no explicit sort, inside the deadline ladder, and skipped entirely for
`exact=1`:

- **Offered** ("did you mean…") when the corrected query does no worse — both queries usually hit
  the engine's 2 000-hit cap, so the count alone says nothing.
- **Applied** (the page shows the corrected query's results, with *search instead for* pointing
  back at `exact=1`) only when what was typed found nothing or a weak top result and the
  correction found more. A correction is never shown unverified.

`query.corrected` and `query.corrected_applied` carry it in the response;
`xustive_spelling_total{applied}` counts it; `GET /admin/spelling?q=…` shows the vocabulary size
and, per token, its shape, its document frequency, the candidates weighed and the one chosen —
so "why was this not corrected?" is answerable without a rebuild.

### 4.4 Expanded leg — conditional

Runs when the primary leg found fewer than `EXPANSION_THRESHOLD` (5) hits, or its top
`_rankingScore` is below `WEAK_TOP_SCORE` 0.6 (relevance order only — under `sort=recency` the
first hit is merely the newest, and the leg fired on nearly every sorted search, BUG-013).
Conditional because expansion costs a round trip and, for a query that already retrieved well,
adds only weaker matches the re-ranker has to push back down.

**One** query carrying up to 12 variant terms, not one per variant: Meilisearch treats extra
terms as optional, and the re-ranker sorts out which mattered. Same filters, sort and highlights
as the primary leg (BUG-022). Merge keeps the primary order first — a document found by both
legs belongs where the primary put it. Failures are swallowed: this leg is an improvement on a
result, never a precondition for it.

### 4.5 Dense fusion (M7-T02)

If a text-embedding engine is configured ([[Vector Index]]) and the ladder allows `Expansion`:
embed the query, k-NN Qdrant, fetch the ids the lexical pool lacks (`id IN […]`, one query), and
**reciprocal-rank-fuse** into the pool. Fail-open. Outcomes are counted three ways
(`recall` / `reinforce` / `fetch_failed`) because labelling an empty id-fetch "reinforce" hid
dense-recall outages behind the label that means "everything worked" (BUG-025).

### 4.6 Re-rank (`xustive_search::rank::rerank`)

Signals the engine does not have, applied as tie-breakers among documents that already match:

```
score = 0.55·relevance + 0.10·freshness + 0.06·trust + 0.09·authority + 0.05·quality
      + 0.07·interaction + 0.10·ui_language − 0.15·spam
```

- `relevance = exp(−pos / 10)`: a 0.05 gap between neighbours, 0.48 across twenty positions —
  small enough locally for freshness to matter, large enough globally that nothing climbs on
  side signals alone. The additive side weights sum to 0.47, deliberately under that 0.48.
- `freshness = exp(−age / τ)`, τ from the inferred **intent**: news 3 days, evergreen 90,
  default 30 (marker words, or ≥ 40 % of candidates under a week old). A guessed date halves it.
- `trust` from the source registry tier; `authority` from `authority.tsv` keyed on host with
  the `.dz` home floor ([[Ranking and Relevance]]); `quality`/`spam` from the document.
- `interaction`: anonymous smoothed CTR above the k-floor ([[Interaction Signals]]), read from
  the signals store before ranking; absent is a neutral 0, never a penalty.
- `ui_language`: 1 when the document is in the reader's nav-bar language (Darija and Arabic count
  as each other), else 0 — a French reader still sees the best Arabic page, just after the
  French pages that are as good.

Then near-duplicates collapse by SimHash Hamming ≤ 3 into the best-scoring copy (shown as
"+N similar"), and domains are capped at 3 with the overflow deferred to the tail, not dropped.
Weights load from `config/ranking.toml` at startup when present. Every card carries an `Explain`.

### 4.7 Federation strip (M7-T05, page 1, runtime switch)

The SearXNG fetch is **detached** at the top of the request with its own budget; on results it
eager-indexes them and feeds the crawler. The response waits only up to the strip budget, less
`SHAPE_RESERVE_MS`, and mixes in whatever arrived that is not already a local result, flagged
`from_web`. Whatever missed the wait still becomes a normal result on the next search. Images and
Videos federate in their own SearXNG category ([[Federation Gateway]], [[Media Extraction]]).

### 4.8 After ranking

- **Summary handoff**: on page 1 with summaries on, the top `MAX_PASSAGES` (8) re-ranked hits
  become passages registered under the normalised query; the response carries a `summary_token`
  and the reader's `ui` language for the output ([[Summarizer]]). Never blocks.
- **Related searches (M7-T03)**: `entities`/`topics` recurring across the top 20 ranked hits,
  minus the query and its sub/superstrings — no graph, no extra round trip.
- **Facets** are never `null`: skipped-by-deadline is distinguished from empty.
- **Interaction capture** ([[Interaction Signals]]): a bounded category by vertical, a result
  count bucket, and an `interaction_token` — best-effort, gated on the runtime switch.
- Comments: `matched_comments` is always empty. The comment leg, per-author cap and
  "did you mean" are **not built** (2026-08-27).

## 5. Configuration

| Key (`config/*.toml`) | Dev value | Notes |
|:---|:---|:---|
| `api.timeout_search_ms` | 2500 | the whole deadline |
| `search.candidate_pool` | 200 | hits pulled before re-rank |
| `search.default_hits_per_page` / `max_hits_per_page` | 20 / 50 | |
| `search.timeout_ms` | 1200 | per Meilisearch call; a timeout triggers narrowing |
| `vector.text_search_limit` | 50 | dense candidates fetched from Qdrant for fusion |
| `federation.fetch_budget_ms`, strip budget | see [[Federation Gateway]] | |
| `config/ranking.toml` | optional | `Weights` incl. `per_domain_cap`, `simhash_collapse_distance` |

`MAX_QUERY_CHARS` 512, `EXPANSION_THRESHOLD` 5, `MAX_EXPANDED_TERMS` 12, `WEAK_TOP_SCORE` 0.6
are constants shared with the eval harness so it scores the same retrieval production runs
(BUG-003).

## 6. Data

Stateless per request apart from the pending-summary registry (keyed by normalised query, short
TTL) and the CTR read. No query-keyed result cache — a query-keyed cache is a query log
([[Security and Privacy]]).

## 7. Failure Modes

| Failure | Response |
|:---|:---|
| Empty / over-long query | 400 `MissingQuery` / `QueryTooLong` |
| Unknown `source`, `sentiment`, `sort` | 400 `InvalidParam`; unknown `v` falls back to All |
| Meilisearch timeout on the pool | narrowed retry; if that also fails, the error propagates |
| Meilisearch 5xx / down | `ApiError::from(SearchError)` → 503-class response |
| Expanded, dense, federation, CTR, summary, capture failing | silently skipped |
| Budget gone before facets / re-rank | chips omitted / engine order, counted as degraded |

## 8. Performance

`xustive_search_duration_seconds{stage}` for `detect`, `retrieve` (incl. the expanded leg),
`rerank`, and the total. No p95 is asserted by a test; the load generator ([[Load Generator]])
is the yardstick.

## 9. Observability

`xustive_search_duration_seconds{stage}`, `xustive_search_results_total{bucket,lang}`,
`xustive_search_zero_results_total{lang}`, `xustive_lang_detected_total{lang,script}`,
`xustive_query_expansion_total{lang}`, `xustive_degraded_total{stage}`,
`xustive_semantic_fused_total{kind}`, `xustive_instant_answers_total{tool}`, federation
duration/searches/fed counters. Spans carry counts and labels — **no query text**.

## 10. Security

Operators become typed filters; free-text values (`site:`) are escaped by the builder, never
string-interpolated. Ids in the dense fetch are quoted. Result strings pass through untouched
except Meilisearch's `<em>` highlight markers, the only markup the client may render.

## 11. Testing

- Unit in `search.rs`: card shaping, `display_url`, federated mixing, media tiles.
- `xustive-search`: operators, filter expressions, `rerank` (relevance stays dominant, unknown
  dates, collapse, domain cap), `is_all_stop_words`, `top_result_is_weak`.
- Offline: `xustive eval` (golden queries, nDCG) and `xustive ab`; `eval-serp` against Google.

## 12. Open Questions

- [ ] Comment leg: nothing indexes comments today, so the `comments` index is empty.
- [ ] Should the expanded leg also fire when the dense leg found nothing new?
- [ ] "Did you mean" — Meilisearch typo tolerance may make a corrector redundant.

## Related

[[ADR-0012 - Discovery-Only Aggregation]] · [[Ranking and Relevance]] · [[Search Index]] ·
[[Query Expander]] · [[Language Detector]] · [[Instant Answers]] · [[Vector Index]] ·
[[Federation Gateway]] · [[Interaction Signals]] · [[Summarizer]] · [[API Contract]] ·
[[Error Handling and Resilience]]
