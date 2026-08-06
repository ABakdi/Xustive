---
tags:
  - architecture
  - search
type: architecture
status: specified
updated: 2026-08-06
---

# Ranking and Relevance

> How a set of matching documents becomes an ordered list. Implemented across [[Search Index]]
> (ranking rules) and [[Query Pipeline]] (post-ranking merge).

---

## 1. Two-Stage Ranking

| Stage | Where | Input | Output |
|:---|:---|:---|:---|
| **Retrieval + base rank** | Meilisearch ranking rules | expanded query | top 200 candidates |
| **Re-rank + merge** | [[Query Pipeline]] in-process | 200 candidates + comment hits | top 20 shown |

Stage 1 is tuned in index settings and is cheap. Stage 2 is where Algeria-specific signals
(freshness, source trust, engagement, comment evidence) are applied — they change often, and we do
not want to reindex to tune them.

---

## 2. Stage 1 — Meilisearch ranking rules

```
["words", "typo", "proximity", "attribute", "sort", "exactness",
 "freshness_desc",      // custom: published_at desc
 "quality_desc"]        // custom: quality_score desc
```

Searchable attributes, in priority order (`attribute` rule uses this order):

```
["title", "excerpt", "entities", "body", "translit_body", "author.name"]
```

Typo tolerance:

| Setting | Value | Reason |
|:---|:---|:---|
| `minWordSizeForTypos.oneTypo` | 4 | Arabic roots are short; 5 is too permissive |
| `minWordSizeForTypos.twoTypos` | 9 | |
| `disableOnAttributes` | `["entities"]` | proper nouns must match exactly |
| `disableOnWords` | wilaya names, operator names | prevents *Oran*→*Iran* class errors |

Full settings JSON lives in [[Search Index]].

---

## 3. Stage 2 — Re-rank formula

```
final = w_rel · rel_norm
      + w_fresh · freshness
      + w_trust · trust
      + w_eng · engagement_norm
      + w_comment · comment_evidence
      − w_spam · spam_score
```

| Signal | Default weight | Definition |
|:---|:---|:---|
| `rel_norm` | **0.55** | Meilisearch rank position normalised: `1 / log2(pos + 2)` |
| `freshness` | **0.20** | `exp(−age_days / τ)`, τ from the table below |
| `trust` | **0.10** | source `trust_tier`: A = 1.0, B = 0.6, C = 0.3, unknown = 0.4 |
| `engagement_norm` | **0.08** | `log1p(likes + 2·comments + 3·shares) / log1p(P99)` per platform |
| `comment_evidence` | **0.07** | `min(1, matched_comments / 3)` — the discussion matched, not just the post |
| `spam_score` | **0.15** | penalty, from [[Enrichment Pipeline]] |

### Freshness half-life τ

| Query intent | τ (days) | Detection |
|:---|:---|:---|
| News / event | 3 | temporal terms (اليوم, hier, "2026"), or ≥ 40 % of candidates < 7 days old |
| Social chatter | 7 | majority of candidates are social `source_type` |
| Evergreen / how-to | 90 | question words + no temporal marker |
| Default | 30 | |

If `published_at_precision = "unknown"`, multiply `freshness` by **0.5** — we refuse to reward a date
we guessed. See [[Data Model]].

### Weight configuration

All weights are in `config/ranking.toml`, hot-reloadable without restart, and every response records
the `ranking_profile` name in traces (never with the query text). A/B profiles: `default`,
`news_heavy`, `social_heavy`.

---

## 4. Diversity and de-clustering

Applied after scoring, before truncation:

1. **Per-domain cap** — max 3 results from one `domain` in the first 20.
2. **Per-author cap** — max 2 results from one `author.id` in the first 20.
3. **Near-duplicate collapse** — results within Hamming distance ≤ 3 on `simhash` collapse into one
   card with a `"+N similar"` affordance ([[Deduplication Service]], [[UI - Results Page]]).
4. **Source-type spread** — if the first 10 are all one `source_type` and other types have hits,
   promote the best 2 from the next type. Prevents Facebook drowning out web results.

---

## 5. Query understanding effects

| Input | Effect on ranking |
|:---|:---|
| Quoted `"…"` phrase | phrase match required; typo tolerance off for that span |
| `site:example.dz` | hard filter on `domain` |
| Language detected `ary` | boost documents where `language ∈ {ary, ar}`; still retrieve `fr` |
| Arabizi query | search both `body` and `translit_body` ([[Query Expander]]) |
| Expanded terms | expansion matches score at **0.7×** the weight of original-term matches |

Expansion must never outrank the literal query. If a user typed *Sonelgaz*, a document containing
only *سونلغاز* ranks below one containing *Sonelgaz* at equal relevance.

---

## 6. Evaluation

We do not tune ranking by vibes. See [[Testing Strategy]] for the harness.

| Artefact | Description |
|:---|:---|
| **Golden set** | 200 queries × top-10 judged 0–3 (irrelevant → perfect), Algerian-native judges |
| **Query mix** | 40 % Arabic, 25 % Darija/Arabizi, 20 % French, 10 % English, 5 % mixed |
| **Primary metric** | nDCG@10 |
| **Secondary** | MRR@10, recall@50, % queries with zero results, % results < 30 days |
| **Gate** | a ranking change must not drop nDCG@10 by > 1 % absolute |

Golden set lives at `eval/golden/*.jsonl`; the harness runs in CI nightly against a frozen index
snapshot.

---

## 7. Known failure modes

| Symptom | Likely cause | Mitigation |
|:---|:---|:---|
| Old evergreen pages beat breaking news | τ too long for the intent | intent detection table §3 |
| One Facebook group dominates | no author cap | §4.2 |
| Arabizi queries return nothing | `translit_body` missing/stale | reindex; [[Query Expander]] fallback |
| Rage-bait tops results | engagement weight too high | cap `engagement_norm` at P95, not P99 |
| Typo tolerance mangles wilaya names | short Arabic tokens | `disableOnWords` §2 |

---

## 8. Open Questions

- [ ] Add semantic (dense) retrieval as a third retrieval leg, fused with RRF?
- [ ] Should sentiment ever affect *ranking*, or only filtering? (currently: filtering only — ranking
      by sentiment would editorialise results)
- [ ] Per-wilaya geo boost when the query names a place?

## Related

[[Search Index]] · [[Query Pipeline]] · [[Query Expander]] · [[Data Model]] ·
[[Testing Strategy]] · [[Performance Budgets]]
