---
tags:
  - component
  - serving
component-id: C05
binary: xustive-api
status: specified
updated: 2026-08-06
---

# Autocomplete Service

> **ID** C05 · **Binary** `xustive-api` · **Upstream** [[API Gateway]] · **Downstream** [[Search Index]]

## 1. Purpose

Serve as-you-type suggestions for `GET /suggest`. Constrained by an unusual requirement: we do not
log queries ([[Security and Privacy]] P1), so the usual "suggest what other people searched" source
of truth is mostly unavailable. Suggestions therefore come primarily from **the corpus**, not from
users.

## 2. Responsibilities

**In scope**: prefix suggestions from indexed entities and titles; transliteration suggestions
(Arabizi prefix → Arabic candidate); a curated static list; ranking and deduplication of suggestions.

**Out of scope**: personalisation; search history (there is none); full search (→ [[Query Pipeline]]).

## 3. Interface

```rust
pub trait SuggestService: Send + Sync {
    async fn suggest(&self, prefix: &str, limit: usize) -> Vec<Suggestion>;
}
pub struct Suggestion { pub text: String, pub kind: Kind, pub score: f32 }
pub enum Kind { Query, Entity, Transliteration, Curated }
```

Response shape in [[API Contract]] §4.

## 4. Internal Design

Four sources, merged and capped:

| Source | Weight | Built from |
|:---|:---|:---|
| **Entity FST** | 1.0 | `entities` + `sources.display_name` from [[Search Index]], rebuilt nightly into an in-memory FST |
| **Title prefix** | 0.7 | Meilisearch search restricted to `title`, `limit 5`, `attributesToRetrieve: ["title"]` |
| **Transliteration** | 0.6 | [[Query Expander]] applied to the prefix, when the prefix is Latin-script and looks Arabizi |
| **Curated** | 0.9 | `data/suggest/curated.tsv` — hand-written high-value queries (administrative procedures, wilaya names, common services) |

Merge rules: normalise each candidate (same `xustive-text` function), dedupe, drop any candidate that
is a strict prefix of another already in the set, sort by weight × source score, cap at `limit`.

### Why an FST

An in-memory finite-state transducer over ~200k entity strings answers a prefix query in
microseconds and costs ~20 MB. It is rebuilt nightly by a background task and swapped atomically
(`ArcSwap`) — no lock held during a query.

### Optional aggregate popularity (k-anonymous)

If enabled, the gateway increments a counter for the *normalised, ≤ 5-token* query in a Redis
HyperLogLog-backed structure with a daily key. A term becomes eligible as a `Query` suggestion only
when its distinct-bucket count ≥ `k_anonymity` (20) on a given day, and counters expire after 90
days. **This is off by default** and its existence is the subject of an open question — a popularity
counter is arguably a query log ([[Security and Privacy]] §9).

## 5. Configuration

| Key | Default |
|:---|:---|
| `limit` | 8 |
| `min_prefix_len` | 2 |
| `timeout_ms` | 100 |
| `fst_rebuild_cron` | `0 3 * * *` |
| `enable_popularity` | `false` |
| `k_anonymity` | 20 |
| `curated_path` | `data/suggest/curated.tsv` |

## 6. Data

Reads: `documents.entities`, `documents.title`, `sources.display_name`, the curated TSV.
Writes: nothing durable (unless `enable_popularity`, which writes expiring aggregate counters only).

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Meilisearch slow | 100 ms timeout | return FST + curated results only |
| FST not yet built (cold start) | null check | Meilisearch + curated only; build in background |
| FST rebuild fails | job error | keep serving the previous FST, `WARN`, alert if > 48 h stale |
| Empty result | count | return `[]`; the UI shows nothing, not an error |
| Prefix too short | guard | `[]` without calling anything |

Suggestion failures are always silent — the search box must never show an error because a suggestion
lookup failed ([[UI - Home Page]]).

## 8. Performance

| Metric | Budget |
|:---|:---|
| p95 latency | **≤ 40 ms** |
| p99 | ≤ 80 ms |
| FST lookup | ≤ 1 ms |
| Memory (FST) | ≤ 40 MB |
| Throughput | ≥ 2 000 rps per replica |

Autocomplete fires on nearly every keystroke (debounced 120 ms client-side,
[[UI - Home Page]]), so it is the highest-QPS endpoint in the system by an order of magnitude.

## 9. Observability

`xustive_suggest_duration_seconds`, `xustive_suggest_empty_total`,
`xustive_suggest_source_total{kind}`, `xustive_fst_size`, `xustive_fst_age_seconds`. Prefixes are
user input — never logged.

## 10. Security

Prefix is capped at 64 chars and validated as text. The curated file is git-reviewed. Suggestions
are rendered as plain text with the matched prefix highlighted client-side by index, not by
injecting markup ([[UI - Home Page]]).

Note the abuse angle: without k-anonymity, a popularity-driven autocomplete lets an attacker probe
what others are searching. The default-off stance in §4 exists for exactly this reason.

## 11. Testing

- Unit: merge/dedupe/prefix-subsumption rules; min-prefix guard.
- Integration: FST built from fixture entities returns expected suggestions for Arabic and Latin
  prefixes.
- Transliteration: typing `sonel` suggests `سونلغاز`; typing `وهر` suggests `وهران`.
- Degradation: kill Meilisearch → suggestions still returned from FST + curated.
- Load: 2 000 rps for 60 s, assert p95 ≤ 40 ms.

## 12. Open Questions

- [ ] Enable aggregate popularity at all? If yes, who reviews the k-anonymity proof?
- [ ] Should suggestions be biased toward the user's detected UI language, or stay language-agnostic?
- [ ] Trending queries widget on the home page — attractive, but the same privacy question, larger.

## Related

[[API Contract]] · [[Search Index]] · [[Query Expander]] · [[UI - Home Page]] ·
[[Security and Privacy]]
