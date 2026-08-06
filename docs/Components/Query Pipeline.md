---
tags:
  - component
  - serving
component-id: C02
binary: xustive-api
status: specified
updated: 2026-08-06
---

# Query Pipeline

> **ID** C02 · **Binary** `xustive-api` · **Upstream** [[API Gateway]] · **Downstream** [[Language Detector]], [[Query Expander]], [[Search Index]], [[Summarizer]]

## 1. Purpose

Orchestrates one search request from a raw string to a ranked, faceted result set. It is the only
component that knows the *order* of search operations, and the only place the composition of
detection → expansion → retrieval → re-ranking lives.

## 2. Responsibilities

**In scope**: query normalisation; operator parsing (`"…"`, `site:`); calling detection and
expansion; building the Meilisearch multi-search request; merging document and comment hits;
applying [[Ranking and Relevance]] stage 2; diversity capping; facet assembly; excerpt/highlight
selection; preparing the summary candidate set; enforcing the degradation ladder.

**Out of scope**: HTTP (→ [[API Gateway]]), index settings (→ [[Search Index]]), summary generation
(→ [[Summarizer]]), lexicon content (→ [[Query Expander]]).

## 3. Interface

```rust
#[async_trait]
pub trait SearchService: Send + Sync {
    async fn search(&self, req: SearchRequest) -> Result<SearchResponse, SearchError>;
}

pub struct SearchRequest {
    pub q: String, pub page: u32, pub hits_per_page: u32,
    pub lang: LangHint, pub filters: Filters, pub sort: Sort,
    pub expand: bool, pub include_comments: bool, pub deadline: Instant,
}
pub struct SearchResponse {
    pub query_info: QueryInfo, pub results: Vec<ResultCard>,
    pub facets: Facets, pub pagination: Pagination,
    pub summary_candidates: Vec<DocumentId>, pub took_ms: u32,
}
```

`deadline` is an absolute `Instant` set by the gateway, not a duration — every downstream call gets
`deadline - now()` so the total budget cannot be exceeded by a slow first step.

## 4. Internal Design

### 4.1 Normalisation (pure, ~1 ms)

1. Unicode NFKC.
2. Strip tatweel `U+0640`, strip Arabic diacritics (harakat `U+064B`–`U+0652`).
3. Fold Arabic-Indic digits `٠-٩` and `۰-۹` → ASCII.
4. Normalise alef variants (`أ إ آ` → `ا`), `ة` → `ه`, `ى` → `ي` in a *secondary* field only —
   the primary query keeps the original form so exact matches still win.
5. Collapse whitespace; trim; cap at 512 chars.
6. Extract operators: quoted phrases, `site:host`, `-term` exclusions.

The same normalisation function is used by [[Content Parser]] at index time. It lives in a shared
`xustive-text` crate — **divergence between query-time and index-time normalisation is the single
most common cause of "why does nothing match"**.

### 4.2 Orchestration

```rust
let norm   = normalize(&req.q);                                  // 1ms
let lang   = detector.detect(&norm).timeout(20ms).unwrap_or(Und);  // degradation
let expand = if req.expand { expander.expand(&norm, lang).timeout(30ms).unwrap_or_default() }
             else { Expansion::none() };
let hits   = index.multi_search(build_queries(&norm, &expand, &req)).timeout(800ms)?;
let cards  = merge_and_rerank(hits, &req);                       // stage 2 ranking
let facets = hits.facets_or_empty();
```

Detection and expansion failures degrade silently; only retrieval failure fails the request
([[Error Handling and Resilience]] §6).

### 4.3 Multi-search

One Meilisearch `POST /multi-search` with 2–3 federated queries:

| Query | Index | Purpose |
|:---|:---|:---|
| `q_primary` | `documents` | literal user terms, full weight |
| `q_expanded` | `documents` | expansion variants, scored ×0.7 |
| `q_comments` | `comments` | only if `include_comments`, `limit 100` |

Sending them as one request means one round trip and one Meilisearch scheduling slot.

### 4.4 Merge and re-rank

1. Union document hits by `id`, keeping the best position across primary/expanded legs.
2. Group comment hits by `document_id`; fetch any parent documents not already in the set (single
   `filter: id IN […]` call, only if it fits the remaining budget).
3. Score with the [[Ranking and Relevance]] §3 formula.
4. Collapse near-duplicates by `simhash` Hamming ≤ 3.
5. Apply per-domain (3) and per-author (2) caps, then source-type spread.
6. Truncate to `hits_per_page`; attach up to 2 `matched_comments` per card.

### 4.5 Summary handoff

The top **8** cards' ids become `summary_candidates`. The gateway mints a `summary_token` for them.
The pipeline itself never blocks on [[Summarizer]].

## 5. Configuration

| Key | Default | Notes |
|:---|:---|:---|
| `candidate_pool` | 200 | hits pulled from Meilisearch before re-rank |
| `comment_pool` | 100 | |
| `summary_candidates` | 8 | |
| `per_domain_cap` | 3 | |
| `per_author_cap` | 2 | |
| `simhash_collapse_distance` | 3 | |
| `expansion_weight` | 0.7 | |
| `ranking_profile` | `default` | hot-reloadable, `config/ranking.toml` |
| `timeout_*_ms` | see [[Error Handling and Resilience]] §6 | |

## 6. Data

Stateless per request. Reads `documents` and `comments` ([[Data Model]]). Writes nothing. Holds no
cache — deliberately, since a query-keyed cache is a query log ([[Security and Privacy]] §9).

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Detection timeout/error | 20 ms timeout | `lang = und`, retrieve across all languages |
| Expansion timeout/error | 30 ms timeout | raw query only; metric `expansion_skipped_total` |
| Meilisearch timeout | 800 ms | 504 `upstream_timeout` |
| Meilisearch 5xx | status | 503 `search_unavailable` |
| Comment leg slow | per-leg budget | serve document hits only |
| Facets slow | 150 ms | omit `facets` |
| Zero results after expansion | count == 0 | return empty + `did_you_mean` if the corrector has a candidate |
| Deadline already passed at re-rank | `Instant` check | return unranked top-N rather than 504 |

## 8. Performance

| Stage | p95 budget |
|:---|:---|
| normalise | 1 ms |
| detect | 3 ms |
| expand | 8 ms |
| multi-search | 50 ms |
| merge + re-rank (200 candidates) | 15 ms |
| **total (excl. summary)** | **≤ 200 ms** |

Re-ranking 200 candidates must be allocation-light: score in place, sort by `f32` key, no
intermediate `Vec<String>` clones.

## 9. Observability

`xustive_search_duration_seconds{stage}`, `xustive_search_results_total{lang}`,
`xustive_search_zero_results_total{lang}`, `xustive_expansion_skipped_total`,
`xustive_rerank_collapsed_total`. Span `search` with `lang`, `expanded_terms_count`,
`results_count`, `candidates` — **no query text**.

## 10. Security

Operators (`site:`) are parsed into typed filters, never string-interpolated into a Meilisearch
filter expression — the filter builder takes enum variants and escapes values. Result strings pass
through untouched; escaping is the client's job ([[UI - Results Page]], [[Security and Privacy]] T8).

## 11. Testing

- Unit: normalisation golden table (Arabic/Darija/Arabizi/French edge cases); operator parser;
  re-rank ordering with synthetic scores; diversity caps; simhash collapse.
- Property: normalisation is idempotent — `normalize(normalize(x)) == normalize(x)`.
- Integration: against a seeded Meilisearch, assert known query → known top result.
- Relevance: the nDCG@10 harness ([[Testing Strategy]], [[Ranking and Relevance]] §6).
- Degradation: fault-inject each dependency and assert the request still succeeds (except retrieval).

## 12. Open Questions

- [ ] "Did you mean" — needs a spell corrector; Meilisearch typo tolerance may make it redundant.
- [ ] Should the comment leg be skipped when the primary leg already returns > 50 strong hits?

## Related

[[Ranking and Relevance]] · [[Search Index]] · [[Query Expander]] · [[Language Detector]] ·
[[Summarizer]] · [[API Contract]] · [[Error Handling and Resilience]]
