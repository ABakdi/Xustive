---
tags:
  - component
  - ingestion
  - nlp
component-id: C18
binary: xustive-worker
status: specified
updated: 2026-08-06
---

# Sentiment Engine

> **ID** C18 · **Binary** `xustive-worker` (transformer mode: `xustive-ml`) · **Upstream** [[Enrichment Pipeline]] · **Downstream** [[Search Index]] (facet), [[UI - Results Page]] (badge)

## 1. Purpose

Assign a positive / neutral / negative label and score to every document and comment, so users can
filter results by sentiment ("what are people saying about X, and is it good or bad?"). This is a
facet and a display badge — by deliberate design it does **not** affect ranking
([[Ranking and Relevance]] §8), because ranking by sentiment would editorialise results.

## 2. Responsibilities

**In scope**: sentiment classification for Arabic, Darija, French, and English; confidence
estimation; model selection per language; batching.

**Out of scope**: emotion taxonomies, sarcasm detection, stance detection, toxicity/moderation
(different problem, different consequences); using sentiment to rank.

## 3. Interface

```rust
pub trait SentimentScorer: Send + Sync {
    fn score(&self, text: &str, lang: Lang) -> Sentiment;
    fn score_batch(&self, items: &[(&str, Lang)]) -> Vec<Sentiment>;
}
pub struct Sentiment { pub label: Label, pub score: f32,      // −1.0 … +1.0
                       pub confidence: f32, pub model: &'static str }
```

`model` is stored on every record ([[Data Model]] §2) so that when the model changes we know exactly
which documents carry stale labels and can backfill selectively.

## 4. Internal Design

### 4.1 Two modes

| Mode | Engine | Cost | Use |
|:---|:---|:---|:---|
| `lexicon` (default) | VADER-style rule scorer with Algerian lexicons | ~1 ms | all documents, all comments |
| `transformer` | DziriBERT / DistilBERT fine-tuned classifier via `rust-bert` | ~40 ms | high-value documents; `hybrid` fallback |
| `hybrid` | lexicon first; escalate to transformer when `confidence < 0.6` | mixed | recommended for production |

Start with `lexicon` — it is 40× cheaper, and at 100 docs/s/worker a 40 ms model would dominate the
entire ingestion budget.

### 4.2 Lexicon scorer

Adapted VADER over four lexicons in `data/sentiment/{ar,ary,fr,en}.tsv`, each row
`term<TAB>valence<TAB>notes`, valence −4…+4.

Rule handling, all of which matter more in Darija than the base algorithm does:

| Rule | Example | Effect |
|:---|:---|:---|
| Negation | `ماشي مليح`, `pas bien`, `machi mlih` | flip and dampen ×0.74 |
| Intensifiers | `بزاف`, `très`, `bezaf`, `gaa3` | ×1.3 |
| Diminishers | `شوية`, `un peu`, `chwiya` | ×0.7 |
| Emoji | 😡 😍 👍 💔 | direct valence; **high signal in social text** |
| Punctuation | `!!!`, `؟؟؟` | ×1.1 per mark, capped |
| ALL CAPS / elongation | `مليييييح`, `SUPER` | ×1.2 |
| Arabizi forms | `mli7`, `khayb`, `zwina` | scored via the `ary` lexicon's Latin column |

Lexicon seeding (a [[Milestone 1 - Text Search MVP]] task): translate/adapt the VADER English
lexicon for French, use an existing Arabic sentiment lexicon for MSA, and **hand-build the Darija
lexicon** (~2 000 terms) — this is the part with no off-the-shelf source and it needs native
speakers.

### 4.3 Label thresholds

```
score >  0.15  → positive
score < −0.15  → negative
otherwise      → neutral
```

Confidence = `min(1, |score| · 2 + lexicon_coverage)`, where `lexicon_coverage` is the fraction of
tokens found in a lexicon. **Low coverage means low confidence** — that is how we avoid confidently
labelling text we did not understand. Below `min_confidence` (0.35) the label is forced to `neutral`
and the UI shows no badge at all ([[UI - Results Page]]).

### 4.4 Transformer mode

DziriBERT fine-tuned on an Algerian sentiment dataset (needs sourcing — see §12), batched at 32,
truncated to 256 tokens, running in `xustive-ml`. Output is calibrated (temperature scaling) so the
confidence is meaningful rather than the usual overconfident softmax.

### 4.5 Text preparation

Score `title + first 1 000 chars of body` — sentiment is usually established early, and scoring 200 KB
of text is both slow and diluted. Comments are scored whole (they are short).

## 5. Configuration

| Key | Default |
|:---|:---|
| `mode` | `lexicon` |
| `positive_threshold` | 0.15 |
| `negative_threshold` | −0.15 |
| `min_confidence` | 0.35 |
| `max_text_chars` | 1000 |
| `lexicon_dir` | `data/sentiment/` (hot-reload) |
| `transformer_batch` | 32 |
| `transformer_max_tokens` | 256 |
| `hybrid_escalate_below` | 0.6 |

## 6. Data

Reads lexicons (and model weights in transformer mode). Writes `sentiment{}` onto `Document` and
`Comment` ([[Data Model]]). The `model` field enables targeted backfill when lexicons change.

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Lexicon missing | startup | **Fatal** — silently unlabelled content is worse than not starting |
| Malformed lexicon row | parse | skip, `WARN` |
| Language `und` | input | score with all lexicons, take the highest-coverage result; low confidence |
| Empty/emoji-only text | guard | emoji-only still scores; empty → `neutral`, confidence 0 |
| Transformer OOM/unavailable | error | fall back to `lexicon`, `WARN` metric |
| Batch timeout | 5 s | fall back to lexicon for that batch |
| Systematic mislabelling after a lexicon edit | offline eval gate | revert the lexicon commit |

## 8. Performance

| Mode | Budget |
|:---|:---|
| Lexicon, 1 000 chars | ≤ 2 ms p95 |
| Lexicon batch of 100 comments | ≤ 20 ms |
| Transformer, batch 32 | ≤ 400 ms |
| Memory (lexicon) | ≤ 30 MB |
| Memory (transformer) | ≤ 800 MB |

## 9. Observability

`xustive_sentiment_label_total{label,lang,model}`, `xustive_sentiment_confidence` (histogram),
`xustive_sentiment_duration_seconds{mode}`, `xustive_sentiment_coverage` (histogram — lexicon hit
rate; **a falling value means the lexicon is going stale as slang shifts**),
`xustive_sentiment_fallback_total`.

## 10. Security

Lexicons are git-reviewed data. Text input is bounded. No user data is involved — this runs on
crawled corpus content only, never on queries.

Fairness note: sentiment lexicons encode judgements. A term list that scores dialect or
politically-charged vocabulary carelessly will systematically mislabel whole communities of speakers.
Lexicon PRs require review by a second native speaker, and §11's evaluation is stratified by language
so a regression on Darija cannot hide behind good French numbers.

## 11. Testing

- **Golden set**: 1 000 labelled items (250 per language, drawn from real Algerian social text),
  double-annotated with inter-annotator agreement reported.
- Targets: macro-F1 ≥ 0.70 lexicon mode, ≥ 0.80 transformer mode; **no language below 0.60**.
- Unit: negation, intensifier, diminisher, emoji, elongation rules — each with a Darija case, not
  only English.
- Calibration: confidence buckets must match observed accuracy within ±10 %.
- Regression gate: a lexicon PR that drops macro-F1 on any language by > 2 % is rejected.
- Neutral-bias check: the neutral rate should sit between 30 % and 60 %; outside that, thresholds are
  wrong.

## 12. Open Questions

- [ ] Where does a labelled Algerian sentiment dataset come from? Options: adapt an existing academic
      dataset (licence check needed), or annotate ~5 000 items ourselves. This gates transformer mode
      entirely.
- [ ] Should we surface *comment* sentiment as a separate "discussion mood" signal on result cards?
- [ ] Is a 3-class label the right output, or would a continuous score with no label be more honest
      given our accuracy?
- [ ] Sarcasm is common in Darija social text and will systematically flip labels. Do we detect it,
      or document the limitation and move on? (Currently: document it.)

## Related

[[Enrichment Pipeline]] · [[Data Model]] · [[Ranking and Relevance]] · [[UI - Results Page]] ·
[[UI - Filters and Facets]] · [[Language Detector]] · [[Testing Strategy]]
