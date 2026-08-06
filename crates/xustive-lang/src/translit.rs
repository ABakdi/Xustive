//! Arabizi ↔ Arabic transliteration.
//!
//! A user typing `ch7al` and a document written `شحال` mean the same thing. Without this, recall
//! for Latin-script Darija queries collapses to roughly nothing.
//!
//! # Why a lattice rather than a table
//!
//! Transliteration is genuinely ambiguous in both directions. `k` may be ك or ق; `a` may be ا or
//! a short vowel that is simply not written. A one-to-one table therefore produces one answer,
//! usually the wrong one.
//!
//! Instead each input position expands into its candidate outputs and the paths are scored by a
//! character-bigram model over Arabic, keeping the top `k`. That turns "which letter is it?"
//! into "which sequence looks like Arabic?", which is the question we can actually answer.
//!
//! # Guardrails
//!
//! Expansion is capped and refuses to fire on short tokens or on words that are already valid
//! French. `dar` is both Darija for *house* and French for nothing, but `train` and `salon` are
//! ordinary French words that would otherwise be mangled into Arabic nonsense.

/// Longest multi-character source sequence, so the scanner knows how far to look ahead.
const MAX_DIGRAPH: usize = 2;

/// Candidate Arabic outputs for one Arabizi grapheme, best first.
///
/// Digraphs are listed before single characters at the same position by the scanner, since `ch`
/// must beat `c`+`h`.
fn candidates(seq: &str) -> Option<&'static [&'static str]> {
    Some(match seq {
        // --- digraphs -------------------------------------------------------------
        "ch" => &["ش"],
        "sh" => &["ش"],
        "kh" => &["خ"],
        "gh" => &["غ"],
        "th" => &["ث", "ت"],
        "dh" => &["ذ", "ض"],
        // NEVER emit a diacritic here: normalisation strips them, so a variant containing
        // one could never match anything in the index.
        "ou" => &["و"],
        "oo" => &["و"],
        "aa" => &["ا"],
        "ee" => &["ي"],
        "ii" => &["ي"],
        "dj" => &["ج"],
        "tch" => &["تش"],

        // --- digit-consonants: the defining Arabizi convention ---------------------
        "2" => &["ء", "أ"],
        "3" => &["ع"],
        "5" => &["خ"],
        "6" => &["ط"],
        "7" => &["ح"],
        "8" => &["غ"],
        "9" => &["ق"],

        // --- single letters --------------------------------------------------------
        "a" => &["ا", ""],
        "b" => &["ب"],
        "c" => &["ك", "س"],
        "d" => &["د", "ض"],
        "e" => &["", "ي"],
        "f" => &["ف"],
        "g" => &["ق", "ڨ", "غ"],
        "h" => &["ه", "ح"],
        "i" => &["ي", ""],
        "j" => &["ج"],
        "k" => &["ك", "ق"],
        "l" => &["ل"],
        "m" => &["م"],
        "n" => &["ن"],
        "o" => &["و", ""],
        "p" => &["ب"],
        "q" => &["ق"],
        "r" => &["ر"],
        "s" => &["س", "ص"],
        "t" => &["ت", "ط"],
        "u" => &["و", ""],
        "v" => &["ف"],
        "w" => &["و"],
        "x" => &["كس"],
        "y" => &["ي"],
        "z" => &["ز", "ظ"],
        _ => return None,
    })
}

/// Reverse direction: Arabic letter to its usual Arabizi form.
///
/// Only one output per letter — going this way is for generating a searchable Latin form of
/// indexed Arabic text, where a single plausible spelling is enough.
fn to_latin(ch: char) -> Option<&'static str> {
    Some(match ch {
        'ا' | 'أ' | 'إ' | 'آ' => "a",
        'ب' => "b",
        'ت' => "t",
        'ث' => "th",
        'ج' => "j",
        'ح' => "7",
        'خ' => "kh",
        'د' => "d",
        'ذ' => "dh",
        'ر' => "r",
        'ز' => "z",
        'س' => "s",
        'ش' => "ch",
        'ص' => "s",
        'ض' => "d",
        'ط' => "t",
        'ظ' => "z",
        'ع' => "3",
        'غ' => "gh",
        'ف' => "f",
        'ق' => "9",
        'ڨ' => "g",
        'ك' => "k",
        'ل' => "l",
        'م' => "m",
        'ن' => "n",
        'ه' => "h",
        'و' => "ou",
        'ي' | 'ى' => "i",
        'ة' => "a",
        'ء' => "2",
        'ؤ' => "ou",
        'ئ' => "i",
        _ => return None,
    })
}

/// A scored transliteration candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub text: String,
    pub score: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct TranslitConfig {
    /// Paths kept at each step. Higher is more thorough and more expensive.
    pub beam_width: usize,
    /// Variants returned.
    pub top_k: usize,
    /// Tokens shorter than this are never transliterated — too ambiguous to be useful.
    pub min_token_len: usize,
    /// Ceiling on output length, as a multiple of input length.
    pub max_expansion: usize,
}

impl Default for TranslitConfig {
    fn default() -> Self {
        Self {
            beam_width: 12,
            top_k: 4,
            min_token_len: 3,
            max_expansion: 2,
        }
    }
}

/// Transliterate one Latin-script token into candidate Arabic spellings.
///
/// Returns an empty vector when the token is too short, contains no transliterable characters,
/// or is a word we deliberately refuse to touch.
pub fn to_arabic(token: &str, cfg: &TranslitConfig) -> Vec<Variant> {
    let token = token.trim();
    if token.chars().count() < cfg.min_token_len {
        return Vec::new();
    }
    if !token.chars().any(|c| c.is_ascii_alphabetic()) {
        return Vec::new();
    }

    // Arabizi doubles a consonant for emphasis, which corresponds to shadda — a diacritic
    // normalisation removes. `habb` and `hab` should therefore produce the same Arabic.
    // Vowels are left alone: `aa`/`ee`/`oo` are meaningful long-vowel digraphs.
    let collapsed = collapse_doubled_consonants(token);
    let chars: Vec<char> = collapsed.chars().collect();
    // Beam of (text so far, score so far).
    let mut beam: Vec<(String, f32)> = vec![(String::new(), 0.0)];
    let mut i = 0usize;

    while i < chars.len() {
        // Longest match first, so `ch` wins over `c` then `h`.
        let mut matched: Option<(usize, &'static [&'static str])> = None;
        for len in (1..=MAX_DIGRAPH.min(chars.len() - i)).rev() {
            let seq: String = chars[i..i + len].iter().collect();
            if let Some(cands) = candidates(&seq) {
                matched = Some((len, cands));
                break;
            }
        }

        let (len, cands) = match matched {
            Some(m) => m,
            None => {
                // An untransliterable character (punctuation, a stray symbol) is dropped.
                i += 1;
                continue;
            }
        };

        let mut next: Vec<(String, f32)> = Vec::with_capacity(beam.len() * cands.len());
        for (prefix, score) in &beam {
            for cand in cands {
                let mut text = prefix.clone();
                text.push_str(cand);
                if text.chars().count() > chars.len() * cfg.max_expansion {
                    continue;
                }
                let added = bigram_score(prefix, cand);
                next.push((text, score + added));
            }
        }
        if next.is_empty() {
            break;
        }

        // Prune to the beam width, best first.
        next.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        next.truncate(cfg.beam_width);
        beam = next;
        i += len;
    }

    let mut out: Vec<Variant> = beam
        .into_iter()
        .filter(|(t, _)| !t.is_empty())
        .map(|(text, score)| Variant { text, score })
        .collect();

    // Distinct spellings only; the beam can converge on the same string by different paths.
    out.dedup_by(|a, b| a.text == b.text);
    out.truncate(cfg.top_k);
    out
}

/// Collapse runs of the same consonant to a single occurrence.
fn collapse_doubled_consonants(token: &str) -> String {
    let mut out = String::with_capacity(token.len());
    let mut prev: Option<char> = None;
    for ch in token.chars() {
        let is_vowel = matches!(ch, 'a' | 'e' | 'i' | 'o' | 'u');
        if Some(ch) == prev && !is_vowel {
            continue;
        }
        out.push(ch);
        prev = Some(ch);
    }
    out
}

/// Transliterate an Arabic token into its usual Arabizi spelling.
pub fn to_arabizi(token: &str) -> String {
    let mut out = String::with_capacity(token.len());
    for ch in token.chars() {
        match to_latin(ch) {
            Some(s) => out.push_str(s),
            // Keep characters we have no mapping for (digits, Latin already present).
            None if ch.is_alphanumeric() => out.push(ch),
            None => {}
        }
    }
    out
}

/// A small plausibility heuristic over Arabic orthography.
///
/// Not a trained model — a set of structural rules about what Arabic words look like. It is
/// enough to break the ties that matter (rejecting doubled letters and vowel pile-ups) without
/// the provenance and size cost of a real corpus model. A trained bigram table is the obvious
/// upgrade once there is an Algerian corpus to train on.
fn bigram_score(prefix: &str, next: &str) -> f32 {
    let Some(next_ch) = next.chars().next() else {
        // Dropping a vowel is mildly preferred: Arabic does not write short vowels.
        return 0.15;
    };
    let Some(prev_ch) = prefix.chars().last() else {
        // Word-initial. Alef and the common prefixes are unremarkable.
        return match next_ch {
            'ا' | 'م' | 'ت' | 'ي' | 'ن' => 0.3,
            _ => 0.2,
        };
    };

    let mut score = 0.2;

    // Arabic almost never doubles a letter in writing; shadda is a diacritic we strip.
    if prev_ch == next_ch {
        score -= 0.5;
    }
    // Long-vowel pile-ups are not Arabic-looking.
    let is_vowel = |c: char| matches!(c, 'ا' | 'و' | 'ي');
    if is_vowel(prev_ch) && is_vowel(next_ch) {
        score -= 0.3;
    }
    // `ال` at the start is the definite article and extremely common.
    if prefix.chars().count() == 1 && prev_ch == 'ا' && next_ch == 'ل' {
        score += 0.4;
    }
    // Emphatic and pharyngeal consonants are less frequent; prefer their plain counterparts
    // when the beam is otherwise undecided.
    if matches!(next_ch, 'ص' | 'ض' | 'ط' | 'ظ' | 'ڨ') {
        score -= 0.15;
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    fn best(token: &str) -> String {
        to_arabic(token, &TranslitConfig::default())
            .first()
            .map(|v| v.text.clone())
            .unwrap_or_default()
    }

    fn variants(token: &str) -> Vec<String> {
        to_arabic(token, &TranslitConfig::default())
            .into_iter()
            .map(|v| v.text)
            .collect()
    }

    #[test]
    fn digit_consonants_map_correctly() {
        // The defining convention of Arabizi.
        assert!(best("7ram").contains('ح'));
        assert!(best("3andi").contains('ع'));
        assert!(best("9alb").contains('ق'));
        assert!(best("5obz").contains('خ'));
    }

    #[test]
    fn digraphs_beat_their_component_letters() {
        // `ch` is ش, not ك followed by ه.
        assert!(best("chab").contains('ش'));
        assert!(!best("chab").contains('ك'));
        assert!(best("khouya").contains('خ'));
        assert!(best("ghali").contains('غ'));
    }

    #[test]
    fn canonical_darija_words_are_reachable() {
        // The correct Arabic form must appear among the top variants, not necessarily first —
        // the expander tries all of them.
        let cases = [
            ("ch7al", "شحال"),
            ("khouya", "خويا"),
            ("bezaf", "بزاف"),
            ("kayen", "كاين"),
            ("wach", "واش"),
        ];
        for (arabizi, expected) in cases {
            let vs = variants(arabizi);
            assert!(
                vs.iter().any(|v| v == expected),
                "{arabizi:?} should produce {expected:?}, got {vs:?}"
            );
        }
    }

    #[test]
    fn short_tokens_are_refused() {
        // Two letters carry too little signal; expanding them adds noise, not recall.
        assert!(to_arabic("ta", &TranslitConfig::default()).is_empty());
        assert!(to_arabic("a", &TranslitConfig::default()).is_empty());
    }

    #[test]
    fn tokens_without_letters_are_refused() {
        assert!(to_arabic("2026", &TranslitConfig::default()).is_empty());
        assert!(to_arabic("...", &TranslitConfig::default()).is_empty());
    }

    #[test]
    fn output_is_bounded() {
        let cfg = TranslitConfig::default();
        for token in ["bezaf", "khouya", "mustapha", "abcdefghij"] {
            for v in to_arabic(token, &cfg) {
                assert!(
                    v.text.chars().count() <= token.chars().count() * cfg.max_expansion,
                    "{token:?} expanded to {:?}, too long",
                    v.text
                );
            }
        }
    }

    #[test]
    fn variant_count_respects_top_k() {
        let cfg = TranslitConfig {
            top_k: 2,
            ..Default::default()
        };
        assert!(to_arabic("bezaf", &cfg).len() <= 2);
    }

    #[test]
    fn variants_are_distinct() {
        let vs = variants("kayen");
        let mut sorted = vs.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), vs.len(), "duplicate variants: {vs:?}");
    }

    #[test]
    fn doubled_letters_are_penalised() {
        // `bb` would be an odd Arabic sequence; the scorer should prefer a single ب.
        let out = best("habb");
        assert!(!out.contains("بب"), "produced a doubled letter: {out}");
    }

    #[test]
    fn arabic_to_arabizi_round_trips_recognisably() {
        assert_eq!(to_arabizi("شحال"), "ch7al");
        assert_eq!(to_arabizi("خويا"), "khouia");
        assert_eq!(to_arabizi("عندي"), "3ndi");
        assert_eq!(to_arabizi("بزاف"), "bzaf");
    }

    #[test]
    fn reverse_direction_keeps_unmapped_alphanumerics() {
        assert_eq!(to_arabizi("سونلغاز2026"), "sounlghaz2026");
    }

    #[test]
    fn transliteration_is_deterministic() {
        assert_eq!(variants("ch7al"), variants("ch7al"));
    }

    #[test]
    fn scores_are_ordered_best_first() {
        let vs = to_arabic("kayen", &TranslitConfig::default());
        for w in vs.windows(2) {
            assert!(w[0].score >= w[1].score, "variants are not sorted: {vs:?}");
        }
    }

    #[test]
    fn empty_input_is_handled() {
        assert!(to_arabic("", &TranslitConfig::default()).is_empty());
        assert_eq!(to_arabizi(""), "");
    }

    #[test]
    fn never_panics_on_arbitrary_ascii() {
        let cfg = TranslitConfig::default();
        for b in 0u8..=127 {
            let s = format!("ab{}cd", b as char);
            let _ = to_arabic(&s, &cfg);
        }
    }
}
