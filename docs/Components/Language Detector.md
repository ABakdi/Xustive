---
tags:
  - component
  - serving
  - nlp
component-id: C03
binary: xustive-api
status: specified
updated: 2026-08-06
---

# Language Detector

> **ID** C03 · **Binary** `xustive-api` (also linked into `xustive-worker`) · **Upstream** [[Query Pipeline]], [[Content Parser]] · **Downstream** [[Query Expander]], [[Sentiment Engine]]

## 1. Purpose

Classify a piece of text as Arabic (`ar`), Algerian Darija (`ary`), French (`fr`), English (`en`),
`mixed`, or undetermined (`und`), plus its script. Everything downstream — tokenisation, expansion,
sentiment model choice, ranking boosts — branches on this.

The hard part is not French vs. English. It is **Darija**, which appears in Arabic script, in Latin
script (Arabizi: *wach rak*, *3aslema*, *khouya*), and code-switched with French mid-sentence
(*rani f la gare*). Off-the-shelf detectors call all of these `ar` or `fr` and lose the distinction.

## 2. Responsibilities

**In scope**: language + script classification with a confidence score; short-text handling (queries
are 1–5 words); code-switch detection; a stable label vocabulary.

**Out of scope**: translation; transliteration (→ [[Query Expander]]); per-token language tagging in v1.

## 3. Interface

```rust
pub trait LanguageDetector: Send + Sync {
    fn detect(&self, text: &str) -> Detection;
}

pub struct Detection {
    pub lang: Lang,            // Ar | Ary | Fr | En | Mixed | Und
    pub confidence: f32,       // 0.0..=1.0
    pub script: Script,        // Arabic | Latin | Mixed
    pub secondary: Option<(Lang, f32)>,   // for code-switched text
}
```

Pure and synchronous — no I/O, no async. Callers wrap it in a timeout only because
`spawn_blocking` may queue.

## 4. Internal Design

Three-layer cascade, cheapest first:

### Layer 1 — Script detection (µs)

Count Unicode blocks. `> 60 %` Arabic block → `Script::Arabic`; `> 60 %` Latin → `Script::Latin`;
otherwise `Mixed`. Digits, punctuation, and emoji are excluded from the denominator.

### Layer 2 — Statistical detection

`lingua-rs` restricted to `{Arabic, French, English}` with `.with_preloaded_language_models()` and
low-accuracy mode disabled. Returns a base language + confidence.

### Layer 3 — Darija discrimination

Layers 1–2 cannot produce `ary`; this layer does.

| Input | Rule |
|:---|:---|
| Arabic script, `lingua` says `ar` | Score against a **Darija marker lexicon** (≈ 1 500 entries: راني, واش, بزاف, كيفاش, نتاع, دروك, خويا, بلاصة…). ≥ 1 strong marker or ≥ 2 weak → `ary`. |
| Latin script, `lingua` unconfident or says `fr`/`en` weakly | Score against an **Arabizi marker set** (digit-letters `3`,`7`,`9`,`2` used as consonants; tokens `wach`, `rak`, `bezaf`, `khoya`, `nta3`, `ch7al`). Two independent signals → `ary` with `Script::Latin`. |
| Both an Arabizi/Darija marker **and** ≥ 30 % confident French tokens | `Mixed` with `secondary = (Fr, …)` |
| < 3 tokens and no marker hit | `Und` — refuse to guess |

Darija/Arabizi lexicons live in `data/lang/` as plain TSV (`term<TAB>weight<TAB>script`) so
contributors can extend them without touching Rust. Loaded into an FST/`AhoCorasick` automaton at
startup.

### Short-query handling

Queries average 2–4 tokens, where statistical detectors are unreliable. Confidence is scaled by
token count: `conf_adj = conf · min(1, tokens / 5)`. Below `min_confidence`, return `Und` — and
`Und` is *safe*: [[Query Pipeline]] then retrieves across all languages instead of narrowing wrongly.

**Design rule:** a wrong confident answer is much worse than `Und`. Narrowing to `fr` on a Darija
query produces zero results; `und` produces slightly noisier but non-empty results.

## 5. Configuration

| Key | Default | Notes |
|:---|:---|:---|
| `min_confidence` | 0.55 | below → `Und` |
| `darija_strong_threshold` | 1 | strong markers needed |
| `darija_weak_threshold` | 2 | weak markers needed |
| `script_dominance` | 0.60 | block ratio for a pure-script call |
| `mixed_secondary_min` | 0.30 | to emit `Mixed` |
| `lexicon_dir` | `data/lang/` | hot-reloadable via SIGHUP |
| `max_chars` | 4096 | documents are truncated before detection |

## 6. Data

Reads two lexicon files. Writes nothing. Memory: ~8 MB for `lingua` preloaded models + ~2 MB for
the automatons.

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Lexicon file missing at boot | startup check | **Fatal** — refuse to start (silent Darija loss is worse) |
| Lexicon malformed line | parse | skip line, `WARN`, continue |
| Text empty / whitespace | guard | `Und`, confidence 0 |
| Text is only URLs/emoji | token filter | `Und` |
| Detection slower than budget | caller timeout | `Und` |

## 8. Performance

| Input | Budget |
|:---|:---|
| Query (≤ 5 tokens) | ≤ 3 ms p95 |
| Document (4 KB truncated) | ≤ 12 ms p95 |
| Memory | ≤ 12 MB |

Detection is called once per query and once per document — at 100 docs/min/worker it must not
allocate per call. Reuse a thread-local scratch buffer.

## 9. Observability

`xustive_lang_detected_total{lang, script}` — the distribution is itself a product metric: if `ary`
is < 10 % of queries, either detection is broken or the audience assumption is wrong.
`xustive_lang_undetermined_total`, `xustive_lang_duration_seconds`.

## 10. Security

Input is untrusted text of bounded length. No regex backtracking (Aho-Corasick is linear); no
allocation proportional to input beyond the truncation cap.

## 11. Testing

- **Golden set**: 1 000 labelled strings — 250 per language plus 100 Arabizi and 100 code-switched.
  Target: ≥ 92 % accuracy overall, ≥ 85 % on `ary`, ≤ 3 % of `ary` misclassified as `fr`.
- Unit: script ratios; marker thresholds; short-input `Und` behaviour.
- Property: detection is deterministic and stable under whitespace changes.
- Regression: every real-world misdetection reported becomes a golden-set row.

## 12. Open Questions

- [ ] Train a small fastText/char-ngram model on Algerian corpora instead of lexicon rules? Better
      recall, but adds a model artefact and training data provenance questions.
- [ ] Per-token language tagging to support code-switched *ranking* (currently only detection).
- [ ] Should Kabyle/Tamazight (`kab`) be a first-class label? Not in v1 scope — flagged for v2.

## Related

[[Query Expander]] · [[Query Pipeline]] · [[Content Parser]] · [[Sentiment Engine]] · [[Glossary]]
