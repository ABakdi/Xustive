//! Lexicon loading and token lookup.
//!
//! Lexicons are plain TSV in `data/`, not Rust source, so a native speaker can extend them
//! through a reviewable diff without touching code. That matters: the Darija lists are the part
//! of this system that most needs contributions from people who are not Rust programmers.
//!
//! Lookup is exact, per token, against a hash map — not substring matching. `راني` occurring
//! inside a longer word is not evidence of anything, and an Aho-Corasick scan over Arabic text
//! produces exactly that kind of false positive.

use std::collections::HashMap;

use xustive_text::{normalize, tokens};

/// How much a marker contributes as evidence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weight(pub f32);

impl Weight {
    /// Distinctly Darija: one occurrence is enough to call it.
    pub const STRONG: Weight = Weight(1.0);
    /// Used in Darija but also elsewhere: needs corroboration.
    pub const WEAK: Weight = Weight(0.5);

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "strong" => Some(Self::STRONG),
            "weak" => Some(Self::WEAK),
            other => other.parse::<f32>().ok().map(Weight),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LexiconError {
    #[error("cannot read lexicon {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("lexicon {path} is empty")]
    Empty { path: String },
}

/// A set of weighted marker terms.
#[derive(Debug, Clone, Default)]
pub struct Lexicon {
    /// Single-token markers, keyed by the normalised token.
    unigrams: HashMap<String, Weight>,
    /// Two-token markers, keyed by the normalised pair joined with a space.
    bigrams: HashMap<String, Weight>,
    name: &'static str,
}

impl Lexicon {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            ..Default::default()
        }
    }

    /// Parse TSV: `term <TAB> weight <TAB> gloss`. Blank lines and `#` comments are skipped.
    ///
    /// A malformed row is skipped with a warning rather than failing the load — one bad line in
    /// a community-edited file should not take detection offline.
    pub fn from_tsv(name: &'static str, text: &str) -> Self {
        let mut lex = Self::new(name);
        for (lineno, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut cols = line.split('\t');
            let (Some(term), weight_col) = (cols.next(), cols.next()) else {
                continue;
            };
            let weight = weight_col.and_then(Weight::parse).unwrap_or(Weight::STRONG);

            // Terms are stored normalised so lookup needs no per-call work.
            let term = normalize(term);
            let parts: Vec<&str> = tokens(&term).collect();
            match parts.len() {
                0 => {
                    tracing::warn!(lexicon = name, line = lineno + 1, "empty term, skipped");
                }
                1 => {
                    lex.unigrams.insert(parts[0].to_string(), weight);
                }
                2 => {
                    lex.bigrams.insert(parts.join(" "), weight);
                }
                _ => {
                    tracing::warn!(
                        lexicon = name,
                        line = lineno + 1,
                        "terms longer than two tokens are not supported, skipped"
                    );
                }
            }
        }
        lex
    }

    pub fn len(&self) -> usize {
        self.unigrams.len() + self.bigrams.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn contains(&self, token: &str) -> bool {
        self.unigrams.contains_key(token)
    }

    pub fn weight_of(&self, token: &str) -> Option<Weight> {
        self.unigrams.get(token).copied()
    }

    /// Score already-normalised text against this lexicon.
    pub fn score(&self, normalized: &str) -> Score {
        let toks: Vec<&str> = tokens(normalized).collect();
        let mut score = Score {
            total: toks.len(),
            ..Default::default()
        };

        for t in &toks {
            if let Some(w) = self.unigrams.get(*t) {
                score.add(*w);
            }
        }
        for pair in toks.windows(2) {
            let joined = pair.join(" ");
            if let Some(w) = self.bigrams.get(&joined) {
                score.add(*w);
            }
        }
        score
    }
}

/// The outcome of scoring text against a lexicon.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Score {
    /// Markers at [`Weight::STRONG`].
    pub strong: usize,
    /// Markers below strong.
    pub weak: usize,
    /// Sum of matched weights.
    pub weight: f32,
    /// Tokens examined.
    pub total: usize,
}

impl Score {
    fn add(&mut self, w: Weight) {
        if w.0 >= Weight::STRONG.0 {
            self.strong += 1;
        } else {
            self.weak += 1;
        }
        self.weight += w.0;
    }

    pub fn hits(&self) -> usize {
        self.strong + self.weak
    }

    /// Fraction of tokens that matched. Low coverage means low confidence, which is how we
    /// avoid confidently labelling text we did not actually recognise.
    pub fn coverage(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.hits() as f32 / self.total as f32
        }
    }

    /// The decision rule: one strong marker, or two weak ones.
    pub fn is_evidence(&self) -> bool {
        self.strong >= 1 || self.weak >= 2
    }
}

// The lexicons are compiled in. They are small, they are required for the detector to work at
// all, and a missing file at runtime would silently degrade Darija detection to nothing —
// exactly the kind of failure that goes unnoticed for months.
const DARIJA_AR_TSV: &str = include_str!("../../../data/lang/darija-ar.tsv");
const ARABIZI_TSV: &str = include_str!("../../../data/lang/arabizi.tsv");
const FRENCH_STOP_TSV: &str = include_str!("../../../data/lang/french-common.tsv");

/// Darija markers written in Arabic script.
pub fn darija_arabic() -> Lexicon {
    Lexicon::from_tsv("darija-ar", DARIJA_AR_TSV)
}

/// Darija markers written in Latin script (Arabizi).
pub fn arabizi() -> Lexicon {
    Lexicon::from_tsv("arabizi", ARABIZI_TSV)
}

/// Very common French words, used to distinguish French from Latin-script Darija.
pub fn french_common() -> Lexicon {
    Lexicon::from_tsv("french-common", FRENCH_STOP_TSV)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tsv_with_comments_and_blanks() {
        let lex = Lexicon::from_tsv("t", "# a comment\n\nواش\tstrong\twhat\nبزاف\tweak\ta lot\n");
        assert_eq!(lex.len(), 2);
        assert_eq!(lex.weight_of("واش"), Some(Weight::STRONG));
        assert_eq!(lex.weight_of("بزاف"), Some(Weight::WEAK));
    }

    #[test]
    fn terms_are_stored_normalised() {
        // A lexicon written with harakat must still match normalised query text.
        let lex = Lexicon::from_tsv("t", "بَزَّاف\tstrong\n");
        assert!(lex.contains("بزاف"));
    }

    #[test]
    fn weight_defaults_to_strong_when_absent() {
        let lex = Lexicon::from_tsv("t", "واش\n");
        assert_eq!(lex.weight_of("واش"), Some(Weight::STRONG));
    }

    #[test]
    fn numeric_weights_are_accepted() {
        let lex = Lexicon::from_tsv("t", "x\t0.25\n");
        assert_eq!(lex.weight_of("x"), Some(Weight(0.25)));
    }

    #[test]
    fn bigrams_are_supported() {
        let lex = Lexicon::from_tsv("t", "ما كاش\tstrong\n");
        let s = lex.score(&normalize("ما كاش والو"));
        assert_eq!(s.strong, 1, "bigram should match across two tokens");
    }

    #[test]
    fn malformed_rows_are_skipped_not_fatal() {
        let lex = Lexicon::from_tsv("t", "واش\tstrong\nثلاث كلمات هنا كثير\tstrong\n\t\t\n");
        assert_eq!(lex.len(), 1, "only the valid single-token row should load");
    }

    #[test]
    fn scoring_counts_strong_and_weak_separately() {
        let lex = Lexicon::from_tsv("t", "واش\tstrong\nبزاف\tweak\n");
        let s = lex.score(&normalize("واش راك بزاف"));
        assert_eq!(s.strong, 1);
        assert_eq!(s.weak, 1);
        assert_eq!(s.total, 3);
        assert!((s.weight - 1.5).abs() < 1e-6);
    }

    #[test]
    fn evidence_rule_is_one_strong_or_two_weak() {
        let lex = Lexicon::from_tsv("t", "a\tstrong\nb\tweak\nc\tweak\n");
        assert!(
            lex.score(&normalize("a x y")).is_evidence(),
            "one strong is enough"
        );
        assert!(
            !lex.score(&normalize("b x y")).is_evidence(),
            "one weak is not"
        );
        assert!(lex.score(&normalize("b c x")).is_evidence(), "two weak are");
    }

    #[test]
    fn coverage_is_zero_for_empty_input() {
        let lex = Lexicon::from_tsv("t", "a\tstrong\n");
        assert_eq!(lex.score("").coverage(), 0.0);
        assert!(!lex.score("").is_evidence());
    }

    #[test]
    fn substring_matches_do_not_count() {
        // The reason lookup is per-token rather than substring: `راني` inside a longer word is
        // not evidence, and a naive scan would report it.
        let lex = Lexicon::from_tsv("t", "راني\tstrong\n");
        let s = lex.score(&normalize("برانية"));
        assert_eq!(s.strong, 0, "substring match must not count");
    }

    #[test]
    fn shipped_lexicons_load_and_are_substantial() {
        let d = darija_arabic();
        let a = arabizi();
        let f = french_common();
        assert!(d.len() > 150, "darija lexicon has only {} entries", d.len());
        assert!(
            a.len() > 150,
            "arabizi lexicon has only {} entries",
            a.len()
        );
        assert!(f.len() > 80, "french lexicon has only {} entries", f.len());
    }

    #[test]
    fn shipped_lexicons_recognise_canonical_markers() {
        let d = darija_arabic();
        for t in [
            "واش",
            "راني",
            "بزاف",
            "كيفاش",
            "وين",
            "شحال",
            "نتاع",
            "خويا",
        ] {
            assert!(d.contains(t), "darija lexicon missing {t}");
        }
        let a = arabizi();
        for t in ["wach", "rani", "bezaf", "kifach", "win", "ch7al", "khoya"] {
            assert!(a.contains(t), "arabizi lexicon missing {t}");
        }
    }

    #[test]
    fn arabizi_and_french_lexicons_do_not_overlap_dangerously() {
        // A token in both would make Latin-script detection ambiguous every time it appears.
        let a = arabizi();
        let f = french_common();
        let overlap: Vec<&String> = a.unigrams.keys().filter(|k| f.contains(k)).collect();
        assert!(
            overlap.is_empty(),
            "tokens present in both arabizi and french lexicons: {overlap:?}"
        );
    }
}
