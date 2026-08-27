---
tags:
  - component
  - serving
component-id: C05
binary: xustive-api
status: built
updated: 2026-08-27
---

# Autocomplete Service

> **ID** C05 · **Binary** `xustive-api` (`crates/xustive-api/src/suggest.rs`) · **Upstream**
> [[API Gateway]] · **Downstream** [[Search Index]], [[Query Expander]]

## 1. Purpose

Serve as-you-type suggestions for `GET /api/v1/suggest?q=&limit=`. Constrained by an unusual
requirement: we do not log queries ([[Security and Privacy]] P1), so the usual "suggest what other
people searched" source of truth is mostly unavailable. Suggestions therefore come primarily from
**the corpus**, not from users.

## 2. Responsibilities

**In scope**: prefix suggestions from indexed entities and titles; transliteration suggestions
(Arabizi prefix → Arabic candidate); a curated static list; ranking and deduplication of suggestions.

**Out of scope**: personalisation; search history (there is none); full search (→ [[Query Pipeline]]).

## 3. Interface

As built, the handler is a plain function rather than a trait:

```rust
// suggest.rs
pub struct SuggestParams { pub q: Option<String>, pub limit: Option<usize> }
pub struct Suggestion   { pub text: String, pub source: &'static str }
pub struct SuggestResponse { pub suggestions: Vec<Suggestion>, pub took_ms: u64 }
```

`source` is one of `curated`, `prefix`, `title`, `transliteration` — the specified `Kind` enum
became a string so the UI can use it for iconography without a shared type. `limit` defaults to
**8** and is clamped to `1..=20`. Response shape in [[API Contract]] §4.

## 4. Internal Design

Four sources, merged and capped. Each can fail on its own without taking the endpoint with it: a
suggestion list is an aid, and an aid that returns an error is worse than one that returns nothing.

| Source | Weight | Built from |
|:---|:---|:---|
| **Prefix index** | 1.0 | entity and title strings from the corpus, plus the curated terms, in one in-memory `PrefixIndex` |
| **Curated** | 0.9 | `data/suggest/curated.tsv` (61 lines today) — passport renewal, wilayas, utility bills: the things a corpus of headlines never suggests |
| **Title search** | 0.7 | a Meilisearch query on the `documents` index, `limit` hits, only if the in-memory legs left room; its own **60 ms** timeout |
| **Transliteration** | 0.6 | [[Query Expander]] on the prefix when `looks_arabizi()` (Latin script with Arabizi digits/patterns) and fewer than `limit` candidates so far; each variant is looked up in the prefix index |

Merge rules (`merge()`): fold each candidate with `xustive_text::fold`, collapse identical and
orthographic variants keeping the best weight, drop any candidate that is a strict prefix of
another already kept, sort by weight (stable), cap at `limit`. Titles lose their site suffix
(`title_term`/`trim_title`) and very long titles are cut. A one-character prefix cannot walk the
whole index — there is a test for that.

### Why a sorted `Vec`, not an FST (superseded 2026-08-27)

The specification called for a finite-state transducer over ~200k entity strings, rebuilt
nightly and swapped with `ArcSwap`. At the few thousand strings we actually have, a sorted
`Vec<(normalised, display, weight)>` with binary search answers in the same microseconds, has no
build step and cannot go stale between rebuilds. `PrefixIndex::build(curated, corpus)` is shaped
so the swap to an FST is a replacement, not a rewrite, when the corpus earns it.

### Optional aggregate popularity (k-anonymous) — **not built**

If enabled, the gateway would count the *normalised, ≤ 5-token* query in a Redis structure with a
daily key, and a term would become a `Query` suggestion only once its distinct-bucket count ≥
`k_anonymity` on a day. It is deliberately not built: a popularity counter is a query log with a
different name, and the open question in §12 has not been resolved. Note that the *search-history*
feature ([[Interaction Signals]], [[ADR-0018 - Anonymous Search History]]) records query counts for
the operator's admin view under a k-floor — it does **not** feed suggestions.

## 5. Configuration

`[suggest]` in `config/*.toml` (`SuggestConfig` in `crates/xustive-core/src/config.rs`):

| Key | Default | Notes |
|:---|:---|:---|
| `curated_path` | `data/suggest/curated.tsv` | missing is an `info` line, not an error |
| `limit` | 8 | |
| `min_prefix_len` | 2 | |

Fixed in code: `MAX_LIMIT` 20, `MIN_PREFIX_CHARS` 2, `TITLE_LEG_TIMEOUT` 60 ms; the route's
transport timeout is `api.timeout_suggest_ms` (150). Not built: `fst_rebuild_cron`,
`enable_popularity`, `k_anonymity`.

**Rebuild cadence.** `AppState::refresh_suggestions` builds the prefix index from the curated
file plus corpus titles and swaps it under a brief write lock (readers clone an `Arc` and finish
against a consistent snapshot). It is called **once at startup** (`main.rs`); the "could be called
on a timer" is written in the code and not yet done, so titles indexed after boot are reachable
only through the title-search leg until a restart.

## 6. Data

Reads: `documents` (titles into the prefix index at startup; title search per request), the curated TSV (comments and per-line weights allowed). Writes: nothing.

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Meilisearch slow | 60 ms title-leg timeout | in-memory sources only |
| Meilisearch down | call error | in-memory sources only; no log noise at per-keystroke rate |
| Empty result | count | `[]`; the UI shows nothing, not an error |
| Prefix too short | `MIN_PREFIX_CHARS` | `[]` without calling anything |

Suggestion failures are always silent — the search box must never show an error because a suggestion
lookup failed ([[UI - Home Page]]).

## 8. Performance

| Metric | Budget |
|:---|:---|
| p95 latency | **≤ 40 ms** |
| p99 | ≤ 80 ms |
| in-memory lookup | ≤ 1 ms |
| Throughput | ≥ 2 000 rps per replica |

Autocomplete fires on nearly every keystroke (debounced client-side in
`web/components/search/SearchBox.tsx`), so it is the highest-QPS endpoint in the system by an
order of magnitude. Its rate limit is 300/min per client against 60/min for search.

## 9. Observability

Built: `xustive_suggest_total` (by outcome/source) and the generic HTTP metrics. Not built under
those names: `xustive_suggest_duration_seconds`, `xustive_suggest_empty_total`, `xustive_fst_*`.
Prefixes are user input — never logged.

## 10. Security

The prefix passes through `xustive_text::fold`; the search-side length cap (512) applies. The
curated file is git-reviewed. Suggestions are rendered as plain text; the matched prefix is
highlighted client-side by index, not by injecting markup ([[UI - Home Page]]).

Note the abuse angle: without k-anonymity, a popularity-driven autocomplete lets an attacker probe
what others are searching. The not-built stance in §4 exists for exactly this reason.

## 11. Testing

Unit tests in `suggest.rs` cover: prefix match; empty and non-matching prefixes; curated outranks
corpus; strict-prefix subsumption; cross-source and orthographic collapse; stable order; limit;
transliteration fires for Arabizi and **not** for French (`oran` must not become Arabic titles);
title trimming; a missing curated file; and that the real `data/suggest/curated.tsv` parses.

Still specified only: the degradation test with Meilisearch killed, and the 2 000 rps load run.

## 12. Open Questions

- [ ] Enable aggregate popularity at all? If yes, who reviews the k-anonymity proof?
- [ ] Should suggestions be biased toward the user's detected UI language, or stay language-agnostic?
- [ ] Trending queries widget on the home page — attractive, but the same privacy question, larger.
- [ ] At what corpus size does the sorted `Vec` need to become an FST, and does the prefix index
      then need a rebuild schedule instead of a startup build?

## Related

[[API Contract]] · [[Search Index]] · [[Query Expander]] · [[UI - Home Page]] ·
[[Security and Privacy]] · [[Interaction Signals]]
