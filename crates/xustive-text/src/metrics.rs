//! Error-rate metrics for the multimodal exit gates: **WER** (word error rate, voice — M3-T02.10)
//! and **CER** (character error rate, OCR — M3-T04.8).
//!
//! Both are edit distance normalised by the length of the *reference*: how much of the truth the
//! system got wrong. WER counts word substitutions, insertions and deletions; CER counts the same
//! at character granularity, which is the right unit for OCR (a one-letter slip is a small error,
//! not a whole wrong word).
//!
//! # Normalisation
//!
//! Both metrics normalise with [`crate::normalize`] first, so a diacritic or a presentation-form
//! difference is not scored as an error — the engine is judged on the words, not on Unicode
//! incidentals. This is the same normalisation the index applies, so the metric measures what a
//! searcher would actually experience.
//!
//! # Aggregation
//!
//! For a corpus, sum the edits and the reference lengths across all pairs and divide once
//! (micro-average) rather than averaging per-utterance rates — see [`Accumulator`]. A per-utterance
//! mean lets a two-word clip with one error (50 %) swamp a hundred-word clip with one error (1 %);
//! the micro-average weights by how much was actually said, which is what a WER number should mean.

/// Levenshtein edit distance between two token slices (substitution/insertion/deletion each cost 1).
///
/// Generic over the token type so it serves both word-level (`&str` tokens) and character-level
/// (`char` tokens) without duplication. O(n·m) time, O(min(n,m)) space — the two-row form, since a
/// full matrix over a long OCR page is needless memory.
pub fn edit_distance<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    // Keep the shorter sequence as the inner (column) axis so the rows are as small as possible.
    let (a, b) = if a.len() < b.len() { (b, a) } else { (a, b) };
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for (i, ai) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, bj) in b.iter().enumerate() {
            let cost = if ai == bj { 0 } else { 1 };
            curr[j + 1] = (prev[j] + cost) // substitute / match
                .min(prev[j + 1] + 1) // delete from a
                .min(curr[j] + 1); // insert into a
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// The edits and reference length of one pair — the raw counts, before dividing. Kept separate so a
/// corpus can be aggregated correctly (micro-average) rather than averaging ratios.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub edits: usize,
    pub reference_len: usize,
}

impl Counts {
    /// The rate, edits ÷ reference length. An empty reference is defined as 0.0 when the hypothesis
    /// is also empty (nothing to get wrong) and 1.0 otherwise (everything is an insertion) — the
    /// convention that keeps a corpus average well-defined.
    pub fn rate(&self) -> f32 {
        if self.reference_len == 0 {
            return if self.edits == 0 { 0.0 } else { 1.0 };
        }
        self.edits as f32 / self.reference_len as f32
    }
}

/// Word-level counts for one (reference, hypothesis) pair, after normalisation.
pub fn word_counts(reference: &str, hypothesis: &str) -> Counts {
    let r: Vec<String> = tokens(reference);
    let h: Vec<String> = tokens(hypothesis);
    Counts {
        edits: edit_distance(&r, &h),
        reference_len: r.len(),
    }
}

/// Character-level counts for one pair, after normalisation (whitespace collapsed to single spaces).
pub fn char_counts(reference: &str, hypothesis: &str) -> Counts {
    let r: Vec<char> = normalise_for_cer(reference).chars().collect();
    let h: Vec<char> = normalise_for_cer(hypothesis).chars().collect();
    Counts {
        edits: edit_distance(&r, &h),
        reference_len: r.len(),
    }
}

/// Word error rate for a single pair.
pub fn wer(reference: &str, hypothesis: &str) -> f32 {
    word_counts(reference, hypothesis).rate()
}

/// Character error rate for a single pair.
pub fn cer(reference: &str, hypothesis: &str) -> f32 {
    char_counts(reference, hypothesis).rate()
}

/// Accumulates counts across a corpus and yields the micro-averaged rate.
#[derive(Debug, Clone, Default)]
pub struct Accumulator {
    total: Counts,
    pairs: usize,
}

impl Accumulator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, counts: Counts) {
        self.total.edits += counts.edits;
        self.total.reference_len += counts.reference_len;
        self.pairs += 1;
    }

    /// Micro-averaged rate: total edits ÷ total reference length across every pair.
    pub fn rate(&self) -> f32 {
        self.total.rate()
    }

    pub fn pairs(&self) -> usize {
        self.pairs
    }

    pub fn edits(&self) -> usize {
        self.total.edits
    }

    pub fn reference_len(&self) -> usize {
        self.total.reference_len
    }
}

/// Normalise then split on whitespace into word tokens.
fn tokens(s: &str) -> Vec<String> {
    crate::normalize(s)
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Normalise and collapse whitespace for character comparison — otherwise a run of spaces the OCR
/// added would count as several character errors, which overstates the rate.
fn normalise_for_cer(s: &str) -> String {
    crate::normalize(s)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_distance_basics() {
        assert_eq!(edit_distance::<char>(&[], &[]), 0);
        assert_eq!(edit_distance(&['a', 'b', 'c'], &['a', 'b', 'c']), 0);
        // kitten → sitting: substitute k→s, e→i, insert g = 3.
        let kitten: Vec<char> = "kitten".chars().collect();
        let sitting: Vec<char> = "sitting".chars().collect();
        assert_eq!(edit_distance(&kitten, &sitting), 3);
    }

    #[test]
    fn edit_distance_is_symmetric() {
        let a: Vec<char> = "abcdef".chars().collect();
        let b: Vec<char> = "azced".chars().collect();
        assert_eq!(edit_distance(&a, &b), edit_distance(&b, &a));
    }

    #[test]
    fn a_perfect_transcript_scores_zero() {
        assert_eq!(wer("مواقيت الصلاة في وهران", "مواقيت الصلاة في وهران"), 0.0);
        assert_eq!(cer("paracetamol", "paracetamol"), 0.0);
    }

    #[test]
    fn one_wrong_word_in_four_is_a_quarter() {
        // One substitution out of four reference words.
        let r = "prayer times in oran";
        let h = "prayer times in algiers";
        assert!((wer(r, h) - 0.25).abs() < 1e-6, "wer was {}", wer(r, h));
    }

    #[test]
    fn insertions_and_deletions_count() {
        // Hypothesis dropped a word: one deletion out of three.
        assert!((wer("one two three", "one three") - 1.0 / 3.0).abs() < 1e-6);
        // Hypothesis added a word: one insertion out of two reference words.
        assert!((wer("one two", "one two three") - 0.5).abs() < 1e-6);
    }

    #[test]
    fn cer_counts_characters_not_words() {
        // A single-letter OCR slip in a 6-char word is 1/6, not a whole wrong word.
        let r = "algeria";
        let h = "algerio";
        assert!(
            (cer(r, h) - 1.0 / 7.0).abs() < 1e-6,
            "cer was {}",
            cer(r, h)
        );
    }

    #[test]
    fn normalisation_means_diacritics_are_not_errors() {
        // Arabic diacritics are folded by `normalize`, so a vowelled vs unvowelled form scores 0.
        assert_eq!(wer("سُونِلغاز", "سونلغاز"), 0.0);
    }

    #[test]
    fn an_empty_reference_is_defined() {
        assert_eq!(
            Counts {
                edits: 0,
                reference_len: 0
            }
            .rate(),
            0.0
        );
        assert_eq!(
            Counts {
                edits: 3,
                reference_len: 0
            }
            .rate(),
            1.0
        );
    }

    #[test]
    fn corpus_micro_average_weights_by_length() {
        // A 2-word clip with 1 error and a 100-word clip with 1 error: the micro-average is
        // 2/102 ≈ 0.0196, NOT the per-clip mean of (0.5 + 0.01)/2 = 0.255. Length must dominate.
        let mut acc = Accumulator::new();
        acc.add(Counts {
            edits: 1,
            reference_len: 2,
        });
        acc.add(Counts {
            edits: 1,
            reference_len: 100,
        });
        assert_eq!(acc.pairs(), 2);
        assert!(
            (acc.rate() - 2.0 / 102.0).abs() < 1e-6,
            "rate was {}",
            acc.rate()
        );
    }
}
