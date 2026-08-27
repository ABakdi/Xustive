---
tags:
  - component
  - serving
  - nlp
component-id: C03
binary: xustive-api (also linked into the crawl daemon and the CLI)
status: built
updated: 2026-08-27
---

# Language Detector

> **ID** C03 · **Crate** `xustive-lang` (`detect.rs`, `lexicon.rs`) ·
> **Upstream** [[Query Pipeline]], [[Content Parser]] · **Downstream** [[Query Expander]],
> [[Sentiment Engine]], ranking's `ui_language` signal

## 1. Purpose

Classify a piece of text as Arabic (`ar`), Algerian Darija (`ary`), French (`fr`), English (`en`),
`mixed`, or undetermined (`und`), plus its script. Everything downstream — expansion, sentiment
lexicon choice, the `language` facet, the reader's-language ranking nudge — branches on this.

The hard part is not French vs. English. It is **Darija**, which appears in Arabic script, in Latin
script (Arabizi: *wach rak*, *3aslema*, *khouya*), and code-switched with French mid-sentence
(*rani f la gare*). Off-the-shelf detectors call all of these `ar` or `fr` and lose the distinction.

## 2. Responsibilities

**In scope**: language + script classification with a confidence score; short-text handling
(queries are 1–5 words); code-switch detection via a `secondary` language; a stable label set.

**Out of scope**: translation; transliteration (→ [[Query Expander]]); per-token language tagging.

## 3. Interface

```rust
pub struct Detector { /* four Lexicons + DetectorConfig */ }   // build once, Sync
impl Detector {
    pub fn new(config: DetectorConfig) -> Self;
    pub fn detect(&self, text: &str) -> Detection;              // normalises first
    pub fn detect_normalized(&self, normalized: &str) -> Detection;
}
pub struct Detection {
    pub lang: Lang,                       // Ar | Ary | Fr | En | Mixed | Und
    pub confidence: f32,                  // 0.0..=1.0, already length-adjusted
    pub script: Script,                   // Arabic | Latin | Mixed | Unknown
    pub secondary: Option<(Lang, f32)>,   // code-switched text
}
impl Detection { pub fn is_actionable(&self) -> bool }         // lang != Und && conf >= 0.5
```

Pure and synchronous — no I/O, no async, no trait. The query handler calls
`detect_normalized` on the already-normalised query; the parser calls `detect` on
`title + first 2 000 chars`. An explicit `?lang=` on a search bypasses detection with
confidence 1.0.

## 4. Internal Design

Three layers, cheapest first.

### Layer 1 — Script (`xustive_text::script`)

Count Unicode blocks over letters only (digits, punctuation, emoji excluded from the
denominator). Above `script_dominance` of one script → `Arabic` or `Latin`; both present in
quantity → `Mixed`; nothing to go on → `Unknown`, which is an immediate `Und`.

### Layer 2 — Lexicons (the layer that produces `ary`)

Four compiled-in TSV lexicons (`include_str!` of `data/lang/`): `darija-ar.tsv` (≈ 540 rows),
`arabizi.tsv` (≈ 270), `french-common.tsv` (≈ 175), `english-common.tsv` (≈ 165). Rows are
`term<TAB>weight<TAB>gloss`, unigrams and bigrams; the French and English lists are disjoint by
construction (a test asserts it). A `Score` is strong/weak hit counts, summed weight, and
**coverage** (hits / tokens). The decision rule is `is_evidence()`: one strong marker or two weak.

| Script | Rule |
|:---|:---|
| Arabic | Darija evidence → `Ary`, conf `0.6 + 0.12·weight (≤ 0.3) + 0.2·coverage (≤ 0.1)`, cap 0.97. Otherwise `Ar` at 0.65 — absence of markers is weak evidence |
| Latin | Arabizi evidence, or digit-as-consonant (`2 3 5 7 9` *adjacent to letters* — a bare `2026` is not Darija) plus a marker, and Arabizi hits ≥ French hits → `Ary` (cap 0.95). Else French if it out-hits Arabizi and ties or beats English → `Fr`. Else English if it beats both → `En`. Ties go to French, which carries far more Algerian traffic |
| Mixed | Darija in either script + French words → `Mixed` at 0.7 with `secondary = (Fr, share)` when the share ≥ `mixed_secondary_min`, else `Ary` at 0.75 with the French recorded; French dominant → `Fr` with `(Ar, fraction)`; otherwise `Mixed` at 0.6 |

Whenever the losing side still scored, it is reported as `secondary` rather than flattened away.

### Layer 3 — Statistical (`whatlang`), Latin script only, last resort

Only when no lexicon verdict was reached, and only to separate French from English — the one
place a general-purpose trigram model genuinely helps. `lingua-rs` was specified originally;
`whatlang` was chosen because this layer's job turned out to be narrow, and `lingua`'s accuracy
would buy little for a large increase in build time and binary size.

The detector is built with an **allow-list of `{Ara, Fra, Eng}`**. Unrestricted, whatlang ranked
"best places to visit in algeria" as Latin, "what documents do i need for a passport" as Catalan,
"software engineer jobs remote" as Norwegian — each fell through and became `Und`, and English
was detected 53 % of the time on the labelled set. Asking "which of these three" is the question
we actually have. Any other winner is `None` → `Und`.

> **Reading its confidence correctly.** `Info::confidence()` is the *margin between candidates*,
> not the probability the choice is right. We take its **choice** and attach our own calibrated
> confidence: 0.80 when `is_reliable()`, 0.62 on a clear margin (≥ 0.30), 0.40 otherwise — below
> the floor, so genuinely ambiguous text becomes `Und`.

### Length discount and floor (`finish`)

The length discount `min(1, tokens / full_confidence_tokens)` applies to **statistical verdicts
only**. Trigram detection genuinely is unreliable on two or three tokens; lexicon evidence is the
opposite — `wach rak khouya` is three tokens that are all strong markers, stronger evidence than
the same three buried in a long document. Discounting lexicon verdicts made every short Darija
query fall through to `Und`, the exact failure this detector exists to prevent. Anything below
`min_confidence` becomes `Und` with its (low) confidence kept.

**Design rule:** a wrong confident answer is much worse than `Und`. `Und` is *safe*: the
[[Query Pipeline]] retrieves across all languages instead of narrowing wrongly.

## 5. Configuration

`DetectorConfig`, in code — nothing in `config/*.toml`, no hot reload.

| Field | Default | Notes |
|:---|:---|:---|
| `min_confidence` | 0.55 | below → `Und` |
| `full_confidence_tokens` | 5 | statistical length discount saturates here |
| `script_dominance` | `script::DEFAULT_DOMINANCE` | block ratio for a pure-script call |
| `mixed_secondary_min` | 0.30 | secondary share needed to emit `Mixed` |
| `max_chars` | 4096 | truncation before detection |

The thresholds `darija_strong_threshold` / `darija_weak_threshold` from the original spec are the
fixed `is_evidence` rule (1 strong or 2 weak), not tunables.

## 6. Data

Lexicons are compiled into the binary. They are small, required for the detector to work at
all, and a missing file at runtime would silently degrade Darija detection to nothing — exactly
the failure that goes unnoticed for months. Editing them means a rebuild. Both Darija lists carry
a `NEEDS NATIVE-SPEAKER REVIEW` banner (blocker B7).

## 7. Failure Modes

| Failure | Response |
|:---|:---|
| Lexicon missing / malformed | impossible at runtime — a build failure |
| Empty / whitespace / symbols only | `Und`, confidence 0, script `Unknown` |
| Text in a language we do not serve | whatlang returns something else → `Und` |
| Short ambiguous Latin text | statistical verdict discounted below the floor → `Und` |

There is no timeout: detection is a few map lookups per token.

## 8. Performance

No budget is asserted by a test. Per call: normalise, tokenise, up to four hash-map passes, and
whatlang only on the Latin fallback path.

## 9. Observability

`xustive_lang_detected_total{lang, script}` on every search — the distribution is itself a
product metric: if `ary` is a small share of queries, either detection is broken or the audience
assumption is wrong. Detection time is folded into `xustive_search_duration_seconds{stage="detect"}`.
No separate undetermined counter (`und` is a label value of the first metric).

## 10. Security

Input is untrusted text truncated to `max_chars`. Hash-map lookups and a linear trigram pass; no
regex, no allocation proportional to input beyond the truncation cap.

## 11. Testing

- `tests/detection_accuracy.rs`: an in-file labelled set with gates of ≥ 92 % overall, ≥ 85 % on
  `ary`, Darija almost never called French, and nothing confidently wrong on undetermined input.
- Unit: Darija in Arabic script, MSA not mistaken for Darija, Arabizi, French not mistaken for
  Arabizi, a bare year is not Arabizi evidence, code-switching records a secondary, short
  ambiguous input is `Und`, empty/symbol-only input.

## 12. Open Questions

- [ ] Train a small char-ngram model on Algerian corpora instead of lexicon rules? Better recall,
      but adds a model artefact and provenance questions.
- [ ] Per-token language tagging to support code-switched *ranking* (currently only detection).
- [ ] Kabyle/Tamazight (`kab`) as a first-class label? `is_tifinagh` exists in `xustive-text`;
      nothing above it does.

## Related

[[Query Expander]] · [[Query Pipeline]] · [[Content Parser]] · [[Sentiment Engine]] ·
[[Ranking and Relevance]] · [[Glossary]]
