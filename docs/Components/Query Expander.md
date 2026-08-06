---
tags:
  - component
  - serving
  - nlp
component-id: C04
binary: xustive-api
status: specified
updated: 2026-08-06
---

# Query Expander

> **ID** C04 · **Binary** `xustive-api` · **Upstream** [[Query Pipeline]] · **Downstream** [[Search Index]]

## 1. Purpose

Bridge the gap between how Algerians *type* and how content is *written*. The same concept appears
as `سونلغاز`, `Sonelgaz`, `sonalgaz`, `صونلغاز`; the same question as `واش راهو` and `wach rahou`.
Without expansion, recall for Darija and Arabizi queries collapses — a user typing `ch7al` finds
nothing written as `شحال`.

This is the highest-leverage NLP component in the system for the Algerian market, and the one with no
off-the-shelf solution.

## 2. Responsibilities

**In scope**: Arabizi ↔ Arabic transliteration; Darija ↔ MSA synonym mapping; French/Arabic
entity aliasing; common misspelling folding; bounded variant generation with weights.

**Out of scope**: translation of full sentences; semantic/embedding retrieval (v2); spell correction
of French/English (Meilisearch typo tolerance handles it).

## 3. Interface

```rust
pub trait QueryExpander: Send + Sync {
    fn expand(&self, normalized: &str, lang: Lang) -> Expansion;
}

pub struct Expansion {
    pub variants: Vec<Variant>,   // ≤ max_variants, weight-ordered
    pub strategy: Strategy,       // Lexicon | Translit | Hybrid | None
}
pub struct Variant { pub text: String, pub weight: f32, pub origin: Origin }
pub enum Origin { Translit, Synonym, EntityAlias, Spelling, Model }
```

The expander returns **query strings**, not filters. [[Query Pipeline]] runs them as a second
retrieval leg weighted at 0.7 ([[Ranking and Relevance]] §5).

## 4. Internal Design

Four stages, applied in order, each bounded.

### 4.1 Transliteration (Arabizi ↔ Arabic)

A rule table over the Algerian Arabizi convention, where digits stand in for Arabic consonants:

| Arabizi | Arabic | Arabizi | Arabic |
|:---|:---|:---|:---|
| `2` | ء / أ | `7` | ح |
| `3` | ع | `8` / `gh` | غ |
| `9` / `q` | ق | `5` / `kh` | خ |
| `ch` | ش | `th` | ث |
| `dj` / `j` | ج | `ou` / `w` | و |

Transliteration is **ambiguous by nature** (`k` → ك or ق; `a` → ا or fatha), so the transducer emits
a *lattice* and we keep the top-`k` paths by a character-bigram language model trained on Arabic
text. Reverse direction (Arabic → Latin) is generated the same way for `translit_body` matching.

Guardrails: never expand tokens shorter than 3 chars; never expand tokens that are already valid
French/English dictionary words (`salon`, `train`) unless a Darija marker is present in the query.

### 4.2 Lexicon expansion

`data/expansion/*.tsv`, one concept per line:

```
# concept        variants (| separated)                                  weight  domain
sonelgaz         سونلغاز|صونلغاز|Sonelgaz|sonalgaz|سونالغاز              1.0     entity
work/job         خدمة|عمل|وظيفة|khedma|travail|boulot                     0.8     synonym
how_much         شحال|بشحال|ch7al|chhal|combien                           0.9     synonym
wilaya_oran      وهران|Oran|Wahran|وهران‎                                 1.0     entity
```

Categories to populate before beta ([[TODO]] M1):
- **Entities**: 58 wilayas + major communes, Algerian institutions (Sonelgaz, Seaal, Algérie
  Télécom, CNAS, ANEM, Air Algérie), banks, universities, clubs.
- **Domain synonyms**: administrative (وثائق / documents / papiers), employment, transport, health.
- **Function words / question words**: واش, كيفاش, وين, علاش and their Arabizi forms.

### 4.3 Model-based expansion (optional, feature-flagged)

DziriBERT masked-LM proposes near-synonyms for out-of-lexicon Darija tokens. Off by default: it is
~15 ms and noisier than the lexicon. Enabled only when the lexicon produced **zero** variants and
the budget allows. Every model-origin variant is capped at weight 0.5.

### 4.4 Assembly and bounding

- Expand at most `max_expanded_tokens` (3) tokens per query — the rarest ones by document frequency.
- Cartesian growth is capped: total variants ≤ `max_variants` (8).
- Deduplicate against the original query; drop any variant identical after normalisation.
- Quoted phrases are **never** expanded.

## 5. Configuration

| Key | Default | Notes |
|:---|:---|:---|
| `max_variants` | 8 | hard cap on the second retrieval leg |
| `max_expanded_tokens` | 3 | |
| `min_token_len` | 3 | |
| `translit_top_k` | 3 | lattice paths kept |
| `enable_model_expansion` | `false` | DziriBERT fallback |
| `model_variant_weight_cap` | 0.5 | |
| `lexicon_dir` | `data/expansion/` | hot-reload on SIGHUP |
| `timeout_ms` | 30 | caller-enforced |

## 6. Data

Reads lexicon TSVs and (optionally) DziriBERT weights. Writes nothing. Lexicons are versioned in git
and reviewed like code — a bad synonym row degrades relevance globally.

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Lexicon missing | startup | **Fatal** in prod, `WARN` + empty expansion in dev |
| Malformed lexicon row | parse | skip row, `WARN`, count metric |
| Expansion exceeds 30 ms | caller timeout | use raw query ([[Error Handling and Resilience]] §6) |
| Variant explosion | `max_variants` cap | truncate by weight |
| Model unavailable | flag/health check | lexicon-only, no error |
| Over-expansion harming precision | offline nDCG regression | revert lexicon commit |

## 8. Performance

| Path | Budget |
|:---|:---|
| Lexicon-only | ≤ 5 ms p95 |
| With transliteration lattice | ≤ 8 ms p95 |
| With DziriBERT fallback | ≤ 30 ms p95 |
| Memory | ≤ 60 MB (lexicons + FST); +500 MB if the model is enabled |

## 9. Observability

`xustive_expansion_variants` (histogram), `xustive_expansion_strategy_total{strategy}`,
`xustive_expansion_duration_seconds`, `xustive_expansion_skipped_total{reason}`. Variant *text* is
derived from the query and therefore must **not** be logged ([[Security and Privacy]] P1).

## 10. Security

Lexicon files are trusted inputs (git-reviewed). Query input is bounded and the transducer is
linear-time with a fixed lattice width — no exponential blow-up. Variants are passed as structured
Meilisearch query terms, never concatenated into a filter expression.

## 11. Testing

- Unit: transliteration table both directions; guardrails (short tokens, French homographs, quoted
  spans untouched); variant cap enforcement.
- Golden: 300 Arabizi ↔ Arabic pairs; assert the correct Arabic form appears in the top-3 variants
  for ≥ 85 %.
- **Relevance gate**: expansion must raise recall@50 on the Darija slice of the golden set by ≥ 15 %
  without dropping nDCG@10 by more than 1 % ([[Ranking and Relevance]] §6). A lexicon PR that fails
  this gate is rejected.
- Fuzz: random Unicode input must not panic or exceed the variant cap.

## 12. Open Questions

- [ ] Should expansion happen at **index** time instead (store `translit_body`), at query time, or
      both? Currently both — index-time for recall, query-time for coverage of unseen forms. Revisit
      once index size is measured ([[Data Model]] §9).
- [ ] Who owns lexicon curation, and what is the review process for community contributions?
- [ ] Dense-vector retrieval could subsume much of this in v2 — does the lexicon then become
      technical debt or a fallback?

## Related

[[Language Detector]] · [[Query Pipeline]] · [[Ranking and Relevance]] · [[Search Index]] ·
[[Data Model]] · [[Glossary]]
