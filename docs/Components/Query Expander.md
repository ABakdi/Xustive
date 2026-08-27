---
tags:
  - component
  - serving
  - nlp
component-id: C04
binary: xustive-api (also the eval and A/B harnesses, and settings migration)
status: built
updated: 2026-08-27
---

# Query Expander

> **ID** C04 · **Crate** `xustive-lang` (`expand.rs`, `translit.rs`, `morph.rs`) · **Upstream**
> [[Query Pipeline]] · **Downstream** [[Search Index]] (second retrieval leg; synonyms setting)

## 1. Purpose

Bridge the gap between how Algerians *type* and how content is *written*. The same concept appears
as `سونلغاز`, `Sonelgaz`, `sonalgaz`; the same question as `شحال` and `ch7al`. Without expansion,
recall for Darija and Arabizi queries collapses — the eval harness measured 19 of 20 Arabizi
queries returning nothing at all before the pipeline called it.

This is the highest-leverage NLP component in the system for the Algerian market, and the one with
no off-the-shelf solution.

## 2. Responsibilities

**In scope**: Arabizi → Arabic transliteration; curated Darija/MSA/French equivalence classes;
light Arabic morphology; bounded, weighted variant generation; exporting the curated classes as
Meilisearch synonyms.

**Out of scope**: sentence translation (→ the translate tool, [[Tool Data Plane]]); dense
retrieval (→ [[Vector Index]], which the pipeline fuses separately); French/English spelling
(Meilisearch typo tolerance).

## 3. Interface

```rust
pub struct Expander { /* term → concept index, concepts, ExpanderConfig */ }
impl Expander {
    pub fn new(config: ExpanderConfig) -> Self;
    pub fn expand(&self, normalized: &str, lang: Lang) -> Expansion;
    pub fn concepts(&self) -> &[Concept];
    pub fn meili_synonyms(&self) -> HashMap<String, Vec<String>>;
}
pub struct Expansion { pub variants: Vec<Variant> }             // weight-ordered, ≤ max_variants
pub struct Variant { pub text: String, pub weight: f32, pub origin: Origin }
pub enum Origin { Lexicon, Translit, Morphology }
pub struct Concept { pub id: String, pub variants: Vec<String>, pub weight: f32 }
```

The expander returns **query terms**, not filters. There is no `Strategy` field and no trait.

**The rule that matters:** an expansion must never outrank the literal query. If a user typed
*Sonelgaz*, a document containing only *سونلغاز* ranks below one containing *Sonelgaz* at equal
relevance. Every weight is below 1.0 (`max_weight` 0.95, enforced), and the pipeline keeps the
primary leg's order first when it merges.

## 4. Internal Design

Per token, in order, each bounded; then sort by weight and truncate.

### 4.1 Curated classes (`Origin::Lexicon`)

`data/expansion/entities.tsv` and `synonyms.tsv`, compiled in (`include_str!`), one concept per
line: `concept_id<TAB>variant|variant|…<TAB>weight<TAB>note`, ≈ 80 rows each. Institutions,
wilayas, and the domain synonyms that let `شحال` reach a ministry page saying `كم`. A term can
belong to more than one concept. `synonyms.tsv` carries a `NEEDS NATIVE-SPEAKER REVIEW` banner
(blocker B7): a wrong entry here does not just mislabel, it pollutes results for every user of
that term.

These same classes are exported by `meili_synonyms()` into the `synonyms` setting of the
`documents` index at migration time ([[Search Index]] §4.2), so the engine applies them during
retrieval at no query-time cost. Meilisearch synonyms are **directional** — `oran → وهران` does
not imply the reverse — so every pair is emitted both ways; getting that wrong is the subtle
failure where expansion appears to work but only for people typing one script.

### 4.2 Transliteration (`Origin::Translit`, weight 0.6)

Arabizi → Arabic only, and only for Latin-script tokens the detector did **not** call French or
English — transliterating `facture` into Arabic letters is noise. Arabic input needs nothing to
reach Arabic documents.

A rule table over the Algerian convention (`2`→ء, `3`→ع, `5`/`kh`→خ, `7`→ح, `9`/`q`→ق, `ch`→ش,
`dj`→ج, `gh`→غ, `ou`→و …), digraphs before single letters. Transliteration is ambiguous in both
directions (`k` may be ك or ق; `a` may be ا or an unwritten vowel), so each position expands into
its candidates and the paths are scored by a **character-bigram model over Arabic**, beam width
12, keeping the top 4. That turns "which letter is it?" into "which sequence looks like Arabic?",
which is the question we can answer. The bigram table is hand-built, not corpus-trained.

Guardrails: tokens under 3 chars and tokens without letters are refused; output length is capped
at 2× input. The reverse direction, `to_arabizi`, exists for the autocomplete leg
([[Autocomplete Service]]), not for a stored `translit_body` — that field is declared on
`Document` and searchable, but nothing populates it (2026-08-27).

### 4.3 Light Arabic morphology (`Origin::Morphology`, weight 0.7, M7-T01.1)

Meilisearch does not stem Arabic, so `الكتاب` and `كتاب` are different tokens — the single biggest
source of Arabic word mismatch. Rather than stem the whole index (a reindex and a correctness
risk), each Arabic-script token yields its affix-stripped stem and definite-article form:
prefixes `وال فال بال كال لل ال`, suffixes `ات ون ين ان ها هم ية ة`, only when the stem keeps
≥ 3 letters. Surface stripping, not root extraction — cheap, safe, reversible. Weighted above
transliteration (affix stripping is more reliable than script guessing), below curated pairs.

### 4.4 Bounding

At most `max_expanded_tokens` (3) tokens per query, first-come in token order; tokens shorter than
`min_token_len` (3) skipped; variants already in the query or already produced are dropped; the
list is sorted by weight *before* truncation to `max_variants` (8), so the cap drops the weakest
rather than whichever came last. The pipeline then sends at most 12 terms.

Not built (2026-08-27): quoted-phrase exemption inside the expander (phrases are stripped by the
operator parser before the query reaches it), rarest-token-first selection, model-based
expansion (DziriBERT) — see §12.

## 5. Configuration

`ExpanderConfig` and `TranslitConfig`, in code; nothing in `config/*.toml`, no hot reload.

| Field | Default |
|:---|:---|
| `max_variants` | 8 |
| `max_expanded_tokens` | 3 |
| `min_token_len` | 3 |
| `max_weight` | 0.95 |
| `translit.beam_width` / `top_k` / `min_token_len` / `max_expansion` | 12 / 4 / 3 / 2 |

## 6. Data

Reads the two compiled-in TSVs. Writes nothing. Lexicons are versioned in git and reviewed like
code — a bad synonym row degrades relevance globally, twice: at query time and in the index's
synonym setting.

## 7. Failure Modes

| Failure | Response |
|:---|:---|
| Lexicon missing / malformed row | build-time; malformed rows are skipped by the parser |
| Nothing to expand | empty `Expansion`; the pipeline sends no second leg |
| Variant explosion | sorted, then truncated at `max_variants` |
| Query confidently French/English | transliteration skipped; curated and morphology still run |

There is no timeout of its own: the pipeline gates the whole leg on the deadline ladder
(`Stage::Expansion`, [[Query Pipeline]] §4).

## 8. Performance

Hash lookups per token, plus a beam search bounded by width 12 for transliterated tokens. No
budget is asserted by a test.

## 9. Observability

`xustive_query_expansion_total{lang}` when the expanded leg actually retrieved. The expanded
terms are returned in `query_info.expanded_terms`, never logged ([[Security and Privacy]]).

## 10. Security

Lexicon files are trusted inputs. Query input is bounded (512 chars) and the transducer has a
fixed beam. Variants are joined into a plain Meilisearch `q`, never into a filter expression.

## 11. Testing

- Unit in `expand.rs`, `translit.rs`, `morph.rs`: table both directions, short and letterless
  tokens refused, French/English skip, weight ceiling, cap-after-sort, morphology stems.
- The offline eval harness (`xustive eval`, [[Ranking and Relevance]]) runs the same expander and
  the same trigger as production, so the Arabizi slice is scored as users experience it.
- Meilisearch synonym export is exercised by the settings-drift check in `xustive migrate`.

## 12. Open Questions

- [ ] Populate `translit_body` at index time, or drop the field? Both were the plan; only
      query-time exists.
- [ ] Rarest-token-first selection needs document frequencies the API does not hold.
- [ ] Who owns lexicon curation, and what is the review process for community contributions?
- [ ] Dense retrieval (M7-T02) now runs beside this. Does the lexicon become fallback or debt?

## Related

[[Language Detector]] · [[Query Pipeline]] · [[Ranking and Relevance]] · [[Search Index]] ·
[[Autocomplete Service]] · [[Data Model]] · [[Glossary]]
