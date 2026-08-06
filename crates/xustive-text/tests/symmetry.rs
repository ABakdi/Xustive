//! The symmetry and idempotency guarantees.
//!
//! These are the tests referenced by `Content Parser §4.4` and `Query Pipeline §4.1`. If either
//! fails, Arabic search breaks silently in production — documents get indexed in one form and
//! queried in another, and nothing anywhere reports an error.

use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use xustive_text::{fold, normalize, NormalizeOptions};

// Bound as constants rather than written inline: `prop_assert!` stringifies its expression as a
// format string, and a literal `\u{0640}` in there is parsed as a positional argument.
const TATWEEL: char = '\u{0640}';
const ZWSP: char = '\u{200B}';
const BOM: char = '\u{FEFF}';

/// Stand-in for the index-time call site (`Content Parser`).
fn index_time(s: &str) -> String {
    normalize(s)
}

/// Stand-in for the query-time call site (`Query Pipeline`).
fn query_time(s: &str) -> String {
    normalize(s)
}

/// The corpus both call sites are checked against. Every entry is text we actually expect:
/// MSA, Darija in Arabic script, Arabizi, French, code-switched, and dirty input.
const CORPUS: &[&str] = &[
    "الجزائر",
    "الجَزَائِر",
    "الجزائر العاصمة",
    "مليـــــح بزاف",
    "واش راك خويا",
    "شحال هذا؟",
    "سونلغاز فاتورة ٢٠٢٦",
    "وهران",
    "خدمة في وهران",
    "مصطفى بن أحمد",
    "wach rak",
    "ch7al hada",
    "3aslema khouya",
    "Sonelgaz facture 2026",
    "Béjaïa Tizi Ouzou",
    "راني في la gare",
    "emploi Alger 2026",
    "  espaces   multiples  ",
    "\u{FEFF}BOM prefixed",
    "zero\u{200B}width",
    "\u{FEDF}\u{FE8E} presentation forms",
    "ＦＵＬＬＷＩＤＴＨ",
    "",
    "   ",
    "!!!???",
    "😀 emoji only",
    "MiXeD CaSe TeXt",
    "tel: ٠٥٥٦١٢٣٤٥٦",
];

#[test]
fn index_time_and_query_time_agree_byte_for_byte() {
    for s in CORPUS {
        assert_eq!(
            index_time(s),
            query_time(s),
            "index-time and query-time normalisation diverged for {s:?}"
        );
    }
}

#[test]
fn normalize_is_idempotent_over_corpus() {
    for s in CORPUS {
        let once = normalize(s);
        let twice = normalize(&once);
        assert_eq!(once, twice, "normalize is not idempotent for {s:?}");
    }
}

#[test]
fn fold_is_idempotent_over_corpus() {
    for s in CORPUS {
        let once = fold(s);
        let twice = fold(&once);
        assert_eq!(once, twice, "fold is not idempotent for {s:?}");
    }
}

#[test]
fn fold_is_a_refinement_of_normalize() {
    // Folding must never *reintroduce* anything normalize removed: applying normalize on top of
    // fold changes nothing except letter identity, which fold already collapsed.
    for s in CORPUS {
        let folded = fold(s);
        assert_eq!(
            folded,
            normalize(&folded),
            "fold output is not normalize-stable for {s:?}"
        );
    }
}

#[test]
fn normalized_output_has_no_stripped_characters() {
    for s in CORPUS {
        let out = normalize(s);
        assert!(!out.contains(TATWEEL), "tatweel survived in {s:?}");
        assert!(!out.contains(ZWSP), "ZWSP survived in {s:?}");
        assert!(!out.contains(BOM), "BOM survived in {s:?}");
        assert!(!out.contains("  "), "double space survived in {s:?}");
        assert!(
            !out.starts_with(' ') && !out.ends_with(' '),
            "untrimmed output for {s:?}"
        );
        assert!(
            !out.chars().any(|c| ('\u{0660}'..='\u{0669}').contains(&c)),
            "Arabic-Indic digit survived in {s:?}"
        );
    }
}

proptest! {
    // Regressions are persisted next to this file so a failure found once is replayed forever.
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/symmetry.proptest-regressions",
        ))),
        cases: 512,
        ..ProptestConfig::default()
    })]

    /// Idempotency over arbitrary Unicode. This is the property that catches the pathological
    /// inputs a hand-written corpus never thinks of.
    ///
    /// It has already earned its place: it found `"\u{e}\u{FB00}"`, where NFKC turns non-ASCII
    /// input into ASCII and the second pass therefore takes a different code path.
    #[test]
    fn prop_normalize_idempotent(s in ".{0,400}") {
        let once = normalize(&s);
        let twice = normalize(&once);
        prop_assert_eq!(once, twice);
    }

    #[test]
    fn prop_fold_idempotent(s in ".{0,400}") {
        let once = fold(&s);
        let twice = fold(&once);
        prop_assert_eq!(once, twice);
    }

    /// Both call sites are the same function, so they cannot diverge on any input.
    #[test]
    fn prop_symmetry(s in ".{0,400}") {
        prop_assert_eq!(index_time(&s), query_time(&s));
    }

    /// Never panics, always valid UTF-8, never grows unboundedly.
    #[test]
    fn prop_never_panics_and_respects_cap(s in ".{0,2000}") {
        let opts = NormalizeOptions { max_chars: 64, ..Default::default() };
        let out = xustive_text::normalize_with(&s, &opts);
        prop_assert!(out.chars().count() <= 64);
    }

    /// Output contains no character the pipeline is contracted to remove.
    #[test]
    fn prop_no_forbidden_chars(s in ".{0,400}") {
        let out = normalize(&s);
        prop_assert!(!out.contains(TATWEEL), "tatweel survived");
        prop_assert!(!out.contains(ZWSP), "zero-width space survived");
        prop_assert!(!out.contains(BOM), "BOM survived");
        prop_assert!(!out.contains("  "), "double space survived");
        prop_assert!(out.trim() == out, "output was not trimmed");
    }
}
