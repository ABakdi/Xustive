---
tags:
  - architecture
  - search
type: architecture
status: implemented
updated: 2026-08-27
---

# Ranking and Relevance

> How a set of matching documents becomes an ordered list. Implemented across [[Search Index]]
> (ranking rules, `xustive-search::settings`), `xustive-search::rank` (the re-ranker) and the
> `/search` handler in `xustive-api` (the legs and the merge).
>
> Audited against the code on 2026-08-27; the 2026-08-06 design is kept where it was superseded,
> marked with the date.

---

## 1. Retrieval legs, then one re-rank

| Stage | Where | Input | Output |
|:---|:---|:---|:---|
| **Lexical retrieval + base rank** | Meilisearch ranking rules | normalised query, operators applied | top `search.candidate_pool` (200) |
| **Expansion leg** (conditional) | second Meilisearch query | expanded terms ([[Query Expander]]) | merged into the pool |
| **Semantic leg** (optional, `vector.text_enabled`) | text embedder + Qdrant `text_bge` | query embedding | fused by RRF into the pool |
| **Re-rank + collapse** | `rank::rerank`, in-process | fused pool | top page (20 by default, 50 max) |
| **Federated merge** (optional, `federation.enabled`) | after re-rank | SearXNG hits via the [[Federation Gateway]] | strip / blended cards, deduped by URL |

Stage 1 is tuned in index settings and is cheap. The re-rank is where Algeria-specific signals
(freshness by intent, source trust, domain authority, the reader's language, anonymous clicks) are
applied — they change often, and we do not want to reindex to tune them.

The expansion leg runs only when the first leg came back **few or weak**: fewer hits than a page,
or a top `_rankingScore` below `WEAK_TOP_SCORE = 0.6` (M7-T01.3) — a strong exact match scores near
1.0; a page where only some terms matched scores well below. Its failures are swallowed: it is an
improvement on the primary result, never a precondition for it.

The semantic leg (M7-T02) embeds the query, k-NNs the text collection, and fuses those candidates
with the lexical ones by reciprocal-rank fusion (`k = 60`, the standard) so neither list dominates —
dense recall pulls in documents lexical missed, lexical precision keeps exact matches on top. It
runs before the interaction and re-rank stages so they see the fused set.

Superseded (2026-08-27): the design's "200 candidates + comment hits" merge. The `comments` index
exists and is migrated ([[ADR-0003 - Comments in a Separate Index]]) but is **not queried at search
time**; `matched_comments` on a result is always empty.

Every leg after the first is subject to the deadline ladder in
[[Error Handling and Resilience]] §6 — expansion and semantic are skipped below 35 % of budget,
re-rank below 8 %.

---

## 2. Stage 1 — Meilisearch ranking rules

`documents` index (`xustive_search::settings::documents_settings`):

```
["words", "typo", "proximity", "attribute", "sort", "exactness",
 "published_at:desc",
 "quality_score:desc"]
```

(The design named these `freshness_desc` / `quality_desc`; Meilisearch custom rules are
`<attribute>:desc`.)

Searchable attributes, in priority order (`attribute` rule uses this order; the ordering is
load-bearing):

```
["title", "excerpt", "entities", "body", "media.ocr_text", "translit_body", "author.name"]
```

`media.ocr_text` was added with the [[Image Pipeline]] so text inside images is findable.

Typo tolerance:

| Setting | Value | Reason |
|:---|:---|:---|
| `minWordSizeForTypos.oneTypo` | 4 | Arabic roots are short; 5 is too permissive |
| `minWordSizeForTypos.twoTypos` | 9 | |
| `disableOnAttributes` | `["entities"]` | proper nouns must match exactly |
| `disableOnWords` | wilaya names (وهران, قسنطينة, عنابة, سطيف, تلمسان, بجاية, Oran, Setif, Annaba, Bejaia, Tlemcen) and operator/institution names (سونلغاز, سيال, موبيليس, جيزي, أوريدو, Sonelgaz, Seaal, CNAS, ANEM, Mobilis, Djezzy, Ooredoo) | prevents *Oran*→*Iran* class errors |

Also set: a `dictionary` of the same brand tokens so the tokenizer keeps them whole;
`separatorTokens` `| · — –` and `nonSeparatorTokens` `@ # _`; `stopWords` (Arabic, French,
English); and `synonyms` generated from the [[Query Expander]] lexicon — Meilisearch synonyms are
**directional**, so every pair is emitted both ways. Full settings JSON lives in [[Search Index]].

A query made only of stop words is refused before it reaches the engine
(`settings::is_all_stop_words`), because Meilisearch would otherwise match everything.

---

## 3. Stage 2 — Re-rank formula

```
final = w_rel · rel
      + w_fresh · freshness
      + w_trust · trust
      + w_auth · authority
      + w_qual · quality
      + w_int · interaction
      + w_lang · ui_language
      − w_spam · spam_score
```

| Signal | Default weight | Definition |
|:---|:---|:---|
| `rel` | **0.55** | engine position normalised: `exp(−pos / 10)` |
| `freshness` | **0.10** | `exp(−age_days / τ)`, τ from the intent table below |
| `trust` | **0.06** | source `trust_tier` from `data/sources/seeds.tsv`: A = 1.0, B = 0.6, C = 0.3; unknown source = B |
| `authority` | **0.09** | domain fame (§3.1), keyed on the host so discovery pages get it too |
| `quality` | **0.05** | `quality_score` from [[Enrichment Pipeline]]; 0.4 when absent |
| `interaction` | **0.07** | anonymous smoothed CTR for this document above the k-floor ([[Interaction Signals]]); 0 when absent — it only ever adds |
| `ui_language` | **0.10** | 1 when the document is in the language the reader chose in the nav bar (Darija and Arabic count as each other), else 0 |
| `spam_score` | **0.15** | penalty, from [[Enrichment Pipeline]] |

**The rule that governs everything here: textual relevance dominates.** The additive side weights
sum to 0.47, deliberately below the relevance gap across twenty positions (0.48). That bound is
what makes "relevance dominates" true by construction rather than by hoping the numbers work out;
each time a signal was added (interaction in M6, the reader's language later) the others were
rebalanced *down* rather than the side budget widened. The unit tests pin it
(`relevance_dominates_every_other_signal`, `high_ctr_cannot_lift_an_irrelevant_document_to_the_top`).

Superseded 2026-08-27: the design's `1 / log2(pos + 2)` relevance curve drops 0.37 between
positions 0 and 1 — more than every other signal combined — so freshness and trust became
decorative. The exponential with decay constant 10 gives a 0.05 gap between neighbours (adjacent
candidates are near-equally relevant and the engine's order between them is close to arbitrary)
and 0.48 across twenty (nothing climbs the list on side signals alone). The design's
`engagement_norm` (0.08) and `comment_evidence` (0.07) signals are not implemented: no social
platform is collected, and comments are not searched (§1).

### 3.1 Domain authority — the "famous websites" signal

A per-domain prior for how well-known a site is, independent of any query: it is what lets a film
search surface `imdb.com` above a forum thread that mentions the same title. Two sources, merged at
API startup (`state::load_authority`):

- **Curated prior** — `data/sources/authority.tsv`, compiled in so a missing file cannot silently
  flatten the signal. This is where *Algeria-first* lives in the ranker: any `.dz` host gets
  `HOME_FLOOR = 0.62` even when unlisted; an unlisted non-`.dz` host gets `BASELINE = 0.35`. A few
  global institutions are listed above the home floor — correct, because for a plainly global
  query the global authority should win, and the weight is small enough to only break ties.
- **Earned** — domain-level PageRank over the crawl's cross-domain link graph
  (`xustive_ingest::pagerank`, `make … pagerank` via `xustive-cli`), stored in Redis
  (`pagerank:authority`), capped at `PAGERANK_CAP = 0.85`, with the `.dz` floor applied. The
  curated list wins on conflict — a human vouching for a domain outranks the link graph — and
  PageRank fills in every crawled domain the prior does not name. Never run, or no Redis, just
  leaves the prior.

### Freshness half-life τ

| Query intent | τ (days) | Detection (`rank::infer_intent`) |
|:---|:---|:---|
| News / event | 3 | a temporal marker in the query (اليوم, عاجل, أمس, hier, urgent, breaking, latest …), or ≥ 40 % of candidates < 7 days old |
| Evergreen / how-to | 90 | a procedure marker (كيفاش, شروط, وثائق, comment, procedure, guide, how, what is …) |
| Default | 30 | |

Markers are checked first, then the candidate-age fallback: if most of what matched is from the
last week, the query is almost certainly about something current even without a temporal word.
The design's "social chatter, τ = 7" intent is not implemented (no social sources).

If `published_at_precision = "unknown"`, `freshness` is multiplied by `unknown_date_factor` =
**0.5** — we refuse to reward a date we guessed. See [[Data Model]].

### Weight configuration

`rank::Weights` is loaded **once at API startup** from `config/ranking.toml` when the file exists
(it is not in the repo; the built-in defaults apply), otherwise defaults. Not hot-reloadable — a
tuned file needs an API restart. The design's `ranking_profile` name in traces and the
`news_heavy` / `social_heavy` A/B profiles do not exist; A/B comparison is offline, with
`make eval-ab` (`xustive-cli ab`) scoring two weight files against the golden set.

`rank::Explain` records every weighted component per result so the CLI's `--explain` can answer
"why is this result third?".

---

## 4. Diversity and de-clustering

Applied after scoring, before truncation:

1. **Near-duplicate collapse** — results within Hamming distance ≤ `simhash_collapse_distance`
   (3) on `simhash` fold into the best-scoring copy, which given the trust weight is usually the
   most accountable publisher; the folded ones are kept on `Ranked.collapsed` for a `"+N similar"`
   affordance ([[Deduplication Service]], [[UI - Results Page]]).
2. **Per-domain cap** — at most `per_domain_cap` (3) results from one `domain` in the page.
   Capped results are **deferred, not dropped**: they move below the diverse set rather than
   disappearing.

Not implemented (design, 2026-08-06): the per-author cap (max 2 per `author.id`) and the
source-type spread (promote the next type when the first 10 are all one `source_type`). Both were
motivated by social sources, which are not collected.

---

## 5. Query understanding effects

| Input | Effect on ranking |
|:---|:---|
| Quoted `"…"` phrase | kept quoted for the engine, so terms must be adjacent |
| `site:example.dz` | hard filter on `domain` |
| `-term` | excluded |
| the reader's UI language | `ui_language` weight (§3) — a French reader still sees the best Arabic page for an Arabic query, just after the French pages that are as good |
| Arabizi / Darija query | expansion leg searches both `body` and `translit_body` ([[Query Expander]]) |
| News vertical (`?v=news`) | a saved filter: web documents with a date we actually know (a guessed date is not news); ranking unchanged |

Those three operators (`xustive_search::operators`) are the whole grammar, chosen because they are
the ones people already try. No boolean `AND`/`OR`/`NOT`, no nesting — a grammar in a search box is
a thing users get wrong and blame themselves for. Operators are extracted from the **raw** query,
before normalisation folds the quotes away.

Superseded 2026-08-27: the design's "expansion matches score at 0.7× the original terms". The
expansion leg is a second retrieval whose hits are **merged after** the primary leg's; there is no
per-term down-weight. The intent survives: expansion must never outrank the literal query. If a
user typed *Sonelgaz*, a document containing only *سونلغاز* ranks below one containing *Sonelgaz*
at equal relevance.

---

## 6. Evaluation

We do not tune ranking by vibes. Harness: `eval/` (README there), `xustive-search::eval`,
`xustive-cli eval`. See [[Testing Strategy]].

| Artefact | Description |
|:---|:---|
| **Golden set** | `eval/golden/v1.jsonl`, 200 queries, generated by `eval/build_golden.py` — **machine-judged** (`"judged_by": "machine"`) by how much of a query's terms a document contains. Flip to `"human"` one query at a time after native-speaker review |
| **Primary metric** | nDCG@10 |
| **Secondary** | MRR@10 (grade ≥ 2 only), zero-result share; each report states what share of its score rests on machine labels |
| **Gate** | `make eval-check` fails when nDCG@10 drops more than `NDCG_TOLERANCE` = 1 % absolute against `eval/reports/baseline.json` |

Because the judgements partly agree with the engine by construction, a high score says the engine
agrees with a heuristic, not that it serves Algerian readers well. What the set *can* do is notice
when a change makes the engine disagree with its past self, and two query families are genuinely
not circular: Arabizi queries judged against Arabic documents (the real test of transliteration),
and orthographic variants (`بجايه` for `بجاية`) judged on the canonical form.

The eval run also replays an anonymous click stream over its own top results and re-scores, so an
interaction-signal change is measured the same way (M6-T09.1). `make eval` writes a dated report
to `eval/reports/`. The design's nightly CI run against a frozen snapshot is not set up; the gate
is run by hand (or `make check`) against a live index. `make golden` and the SERP yardstick
(`xustive-cli serp-eval`, `eval/serp-queries.txt`) compare against a public engine's top results
for the same queries.

The design's human-judged 200-query set with a fixed language mix (40 % Arabic, 25 % Darija /
Arabizi, 20 % French, 10 % English, 5 % mixed) remains the target.

---

## 7. Known failure modes

| Symptom | Likely cause | Mitigation |
|:---|:---|:---|
| Old evergreen pages beat breaking news | τ too long for the intent | marker tables in `rank.rs` §3 |
| One site owns the page | per-domain cap too high | `per_domain_cap` §4 |
| Arabizi queries return nothing | `translit_body` missing/stale, or the expansion leg skipped under deadline | reindex; check `xustive_degraded_total{stage="expansion"}` |
| A popular-but-wrong page climbs | interaction weight too high | bounded by construction (§3); lower `interaction` |
| Typo tolerance mangles wilaya names | short Arabic tokens | `disableOnWords` §2 |
| A famous global site beats the Algerian answer | authority above the home floor | `authority.tsv` is curated; the `.dz` floor is 0.62 |

---

## 8. Open Questions

- [x] Add semantic (dense) retrieval as a third retrieval leg, fused with RRF? — **Done**
      (M7-T02, `vector.text_enabled`, off by default).
- [ ] Should sentiment ever affect *ranking*, or only filtering? (currently: filtering only — ranking
      by sentiment would editorialise results)
- [ ] Per-wilaya geo boost when the query names a place? (`geo.wilaya` is filterable, not weighted)
- [ ] Human judgements for the golden set (see §6).

## Related

[[Search Index]] · [[Query Pipeline]] · [[Query Expander]] · [[Data Model]] ·
[[Interaction Signals]] · [[Testing Strategy]] · [[Performance Budgets]]
