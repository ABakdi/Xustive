---
tags:
  - component
  - ingestion
  - nlp
component-id: C18
binary: xustive crawl (CLI); not wired into the crawl daemon
status: built (lexicon mode) — not in the production ingest path
updated: 2026-08-27
---

# Sentiment Engine

> **ID** C18 · **Crate** `xustive-lang` (`sentiment.rs`) · **Upstream** [[Content Parser]] output,
> via the `xustive crawl` command · **Downstream** [[Search Index]] (`sentiment.label` facet),
> the result card badge and the "tone" filter chips

## 1. Purpose

Assign a positive / neutral / negative label and score to every document, so readers can filter
results by tone ("what are people saying about X, and is it good or bad?"). This is a facet and a
display badge — by deliberate design it does **not** affect ranking ([[Ranking and Relevance]]),
because ranking by sentiment would editorialise results.

## 2. Responsibilities

**In scope**: lexicon sentiment for Arabic, Darija (both scripts), French and English; a
confidence that comes from evidence; a model id on every record.

**Out of scope**: emotion taxonomies, sarcasm, stance, toxicity/moderation; using sentiment to
rank; comments (nothing produces `Comment` records today).

## 3. Interface

```rust
pub const MODEL_ID: &str = "vader-dz@1";
pub struct Scorer { /* four lexicons + ScorerConfig */ }
impl Scorer {
    pub fn new(config: ScorerConfig) -> Self;
    pub fn score(&self, text: &str, lang: Lang) -> Sentiment;
    pub fn lexicon_size(&self, lang: &str) -> usize;
}
// xustive_core::Sentiment { label: SentimentLabel, score: f32 /* −1…+1 */, confidence: f32,
//                           model: String }
```

No batch call and no trait. `model` is stored on every record ([[Data Model]]) so that when the
lexicon changes we know exactly which documents carry stale labels. `Sentiment::default()` is
neutral, confidence 0, `model = "none"` — which is what every document from the daemon carries
(§4.6).

## 4. Internal Design

### 4.1 One mode

A VADER-style rule scorer. Deliberately not a model: at 100 documents/s/worker a 40 ms
transformer would dominate the ingestion budget, and a lexicon is explainable and tunable without
labelled data we do not have. The `transformer` and `hybrid` modes of the original design are
**not built** (2026-08-27) — see §12.

### 4.2 Lexicons

Compiled in from `data/sentiment/{ar,ary,fr,en}.tsv` (`include_str!`), ≈ 190 / 190 / 130 / 100
rows, `term<TAB>valence<TAB>notes`, valence −4…+4. Arabic-script Darija terms live in `ar.tsv`
alongside MSA (they share a lexicon at lookup time); `ary.tsv` is the **Latin-script** Arabizi
list (`mli7`, `khayb`, `zwina`). It carries a `NEEDS NATIVE-SPEAKER REVIEW` banner (blocker B7):
the file with the least prior art and the most consequence.

Which lexicons are consulted: `Ar` → `ar`; `Ary` → `ary` + `ar`; `Fr` → `fr`; `En` → `en`;
`Mixed`/`Und` → all four, keeping whichever gave the best coverage.

### 4.3 Rules

| Rule | Example | Effect |
|:---|:---|:---|
| Negation within 3 tokens | `ماشي مليح`, `pas bien`, `machi mlih` | flip and dampen ×0.74 |
| Intensifiers | `بزاف`, `très`, `bezaf` | ×1.3 |
| Diminishers | `شوية`, `un peu`, `chwiya` | ×0.7 |
| Emoji | 😍 😡 👍 💔 … | direct valence, read from the *original* text (normalisation drops nothing but would lowercase) |
| Punctuation | `!` (≤ 3), `؟`/`?` (≤ 2) | ×(1 + 0.05 per mark), so `!!!!!!!!` is not an argument |
| Elongation | `مليييييح` | treated as emphasis on the matched word |

Modifier words count toward coverage but carry no polarity themselves. Arabic clitics are
stripped before lookup so `والمليح` finds `مليح`.

### 4.4 Score and confidence

Score is VADER's saturating normalisation `x / √(x² + 15)`, clamped to ±1. Dividing by document
length instead was measurably wrong: a fixed amount of sentiment spread over a longer article
looked *weaker* rather than equally strong, and every crawled news article scored near zero.

Confidence is **evidence × direction**, multiplicative because both are required:

```
found     = lexicon hits + emoji hits
evidence  = max( min(found / 4, 1),  found / tokens )
strength  = min(|score| · 3, 1)
confidence = evidence · (0.3 + 0.7 · strength)
```

Evidence is the *stronger* of a saturating count and a share, because neither alone works across
the range of text we index: an absolute count suits a 1 200-word article with five sentiment
words; a share suits `machi mlih`, two tokens and unmistakably negative. Using only the count
forced every short comment to neutral; using only the share forced every article to neutral.
Below `min_confidence` the label is forced to **neutral** and the UI shows no badge — absence is
more honest than a shrug.

Labels: `score > 0.15` positive, `< −0.15` negative, otherwise neutral.

### 4.5 Text preparation

The first `max_chars` (1 000) of `title + body`, normalised with `xustive_text::normalize`.
Sentiment is usually established early, and scoring 200 KB is both slow and diluted.

### 4.6 Where it actually runs

`Scorer` is called from the one-off **`xustive crawl`** command only. The crawl daemon's
orchestrator and the [[Enrichment Pipeline]] do not call it, so documents indexed by `crawld`
carry `Sentiment::default()` — the `sentiment.label` facet on a production index is uniformly
`neutral` with `model = "none"`. Not built as of 2026-08-27; see §12.

## 5. Configuration

`ScorerConfig`, in code; nothing in `config/*.toml`.

| Field | Default |
|:---|:---|
| `positive_threshold` | 0.15 |
| `negative_threshold` | −0.15 |
| `min_confidence` | 0.35 |
| `max_chars` | 1000 |

## 6. Data

Reads the compiled-in lexicons. Writes `sentiment{}` onto `Document`. `sentiment.label` is
filterable and faceted on the `documents` index ([[Search Index]] §4.2) and is one of the three
facets the search response returns.

## 7. Failure Modes

| Failure | Response |
|:---|:---|
| Lexicon missing / malformed | build-time failure, not a runtime one |
| Language `und` / `mixed` | all four lexicons, best coverage wins, low confidence |
| Empty text | neutral, confidence 0 |
| Emoji-only text | still scores — emoji are read from the raw text |
| Low coverage | neutral, no badge |

## 8. Performance

Tokenise plus one hash lookup per token per lexicon consulted; no budget is asserted by a test.

## 9. Observability

None. The metrics in the original design (`xustive_sentiment_*`, and the coverage histogram whose
fall would mean the lexicon is going stale as slang shifts) are **not built**.

## 10. Security

Lexicons are git-reviewed data. Input is bounded to `max_chars`. Runs on crawled content only,
never on queries.

Fairness note: sentiment lexicons encode judgements. A term list that scores dialect or
politically-charged vocabulary carelessly will systematically mislabel whole communities of
speakers. Lexicon changes need a second native speaker, and §11's gate is stratified by language.

## 11. Testing

- `tests/sentiment_accuracy.rs`: an in-file labelled set with gates of ≥ 75 % accuracy, polarity
  never inverted, confidence higher when right than when wrong, and neutral text staying neutral.
- Unit: negation, intensifier, diminisher, emoji, punctuation cap, clitic stripping, the
  `EVIDENCE_SATURATION` choice ("une catastrophe et un echec" must label).

## 12. Open Questions

- [ ] Wire the scorer into the daemon path (an enrichment step, or the media-repass pattern), so
      the tone facet means something on a production index.
- [ ] Where does a labelled Algerian sentiment dataset come from? It gates any transformer mode.
- [ ] Is a 3-class label right, or would a continuous score with no label be more honest?
- [ ] Sarcasm is common in Darija social text and will flip labels. Documented, not mitigated.

## Related

[[Enrichment Pipeline]] · [[Data Model]] · [[Ranking and Relevance]] · [[UI - Results Page]] ·
[[UI - Filters and Facets]] · [[Language Detector]] · [[Testing Strategy]]
