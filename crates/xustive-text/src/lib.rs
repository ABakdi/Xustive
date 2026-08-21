//! Shared text normalisation for Xustive.
//!
//! # Why this crate exists
//!
//! [`normalize`] is called in two places: by the query pipeline on a user's query, and by the
//! content parser on every crawled document. **If those two ever diverge, Arabic search silently
//! stops matching** — no error, no alert, just gradually worse results.
//!
//! That is why normalisation lives in its own crate with no dependencies on anything else in the
//! workspace, and why the symmetry test in `tests/symmetry.rs` exists.
//!
//! # Two levels
//!
//! - [`normalize`] — conservative. NFKC, strip diacritics/tatweel/invisibles, fold digits,
//!   lowercase, collapse whitespace. Preserves letter identity, so exact matches still win.
//! - [`fold`] — aggressive. Everything `normalize` does, plus folding orthographic variants
//!   (`أ إ آ` → `ا`, `ة` → `ه`, `ى` → `ي`). Used for the secondary match field only.

pub mod metrics;
pub mod script;

use unicode_normalization::UnicodeNormalization;

/// Default cap applied by [`normalize`]. Queries are capped far lower by the API layer;
/// this bound exists so a hostile document cannot make normalisation unbounded work.
pub const DEFAULT_MAX_CHARS: usize = 200_000;

/// Options for [`normalize_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalizeOptions {
    /// Strip Arabic diacritics (harakat, Quranic annotation marks, superscript alef).
    pub strip_diacritics: bool,
    /// Strip the tatweel / kashida elongation character (`U+0640`).
    pub strip_tatweel: bool,
    /// Fold Arabic-Indic and Extended Arabic-Indic digits to ASCII `0-9`.
    pub fold_digits: bool,
    /// Lowercase Latin text.
    pub lowercase: bool,
    /// Fold orthographic variants of Arabic letters. Enabled by [`fold`], not by [`normalize`].
    pub fold_letters: bool,
    /// Maximum output length in `char`s.
    pub max_chars: usize,
}

impl Default for NormalizeOptions {
    fn default() -> Self {
        Self {
            strip_diacritics: true,
            strip_tatweel: true,
            fold_digits: true,
            lowercase: true,
            fold_letters: false,
            max_chars: DEFAULT_MAX_CHARS,
        }
    }
}

impl NormalizeOptions {
    /// The aggressive profile used by [`fold`].
    pub fn folding() -> Self {
        Self {
            fold_letters: true,
            ..Self::default()
        }
    }
}

/// The canonical normalisation. **Call this at both query time and index time.**
///
/// ```
/// use xustive_text::normalize;
/// // tatweel and harakat removed, Arabic-Indic digits folded
/// assert_eq!(normalize("الجَزَائِر ٢٠٢٦"), "الجزائر 2026");
/// // whitespace collapsed, Latin lowercased
/// assert_eq!(normalize("  Sonelgaz   FACTURE "), "sonelgaz facture");
/// ```
pub fn normalize(input: &str) -> String {
    normalize_with(input, &NormalizeOptions::default())
}

/// Aggressive folding for the secondary match field.
///
/// ```
/// use xustive_text::fold;
/// // alef variants collapse, ta marbuta becomes ha
/// assert_eq!(fold("أحمد"), fold("احمد"));
/// assert_eq!(fold("خدمة"), "خدمه");
/// ```
pub fn fold(input: &str) -> String {
    normalize_with(input, &NormalizeOptions::folding())
}

/// Normalisation with explicit options.
pub fn normalize_with(input: &str, opts: &NormalizeOptions) -> String {
    // Fast path: pure ASCII lowercase with no runs of whitespace is extremely common for
    // French/English queries and skips the whole Unicode pipeline.
    //
    // The two paths must produce identical output for every input the fast path accepts —
    // `prop_fast_path_matches_slow_path` asserts exactly that. It is not a theoretical concern:
    // NFKC can turn non-ASCII input into ASCII (`ﬀ` → `ff`), so a string can take the slow path
    // on the first call and the fast path on the second, and idempotency depends on them agreeing.
    if let Some(out) = ascii_fast_path(input, opts) {
        return out;
    }
    normalize_slow(input, opts)
}

/// The full Unicode pipeline.
fn normalize_slow(input: &str, opts: &NormalizeOptions) -> String {
    // 1. NFKC: canonical + compatibility composition. Collapses presentation forms
    //    (ﺍﻟﺠﺰﺍﺋﺮ), full-width Latin, ligatures.
    let composed: String = input.nfkc().collect();

    // 2-4. Single pass: drop invisibles/diacritics/tatweel, fold digits and letters.
    let mut out = String::with_capacity(composed.len());
    for ch in composed.chars() {
        if is_invisible(ch) {
            continue;
        }
        // Control characters other than whitespace are invisible garbage that would otherwise
        // make two identical-looking strings fail to match. Whitespace-like controls (tab,
        // newline) fall through and are collapsed to a space in step 7.
        if ch.is_control() && !ch.is_whitespace() {
            continue;
        }
        if opts.strip_tatweel && ch == '\u{0640}' {
            continue;
        }
        if opts.strip_diacritics && is_arabic_diacritic(ch) {
            continue;
        }
        let ch = if opts.fold_digits { fold_digit(ch) } else { ch };
        let ch = if opts.fold_letters {
            fold_letter(ch)
        } else {
            ch
        };
        out.push(ch);
    }

    // 5. Lowercase, then re-normalise. `to_lowercase` can emit sequences that are not NFKC
    //    (`U+0130` is the classic case), and idempotency depends on the output being normalised.
    let out = if opts.lowercase {
        out.to_lowercase().nfkc().collect()
    } else {
        out
    };

    // 7. Collapse whitespace runs to a single space and trim.
    let mut out = collapse_whitespace(&out);

    // 8. Cap length in chars, never splitting a char boundary.
    truncate_chars(&mut out, opts.max_chars);
    out
}

/// Handles the common all-ASCII case without allocating through the Unicode pipeline.
/// Returns `None` when the input needs the full path.
fn ascii_fast_path(input: &str, opts: &NormalizeOptions) -> Option<String> {
    if !input.is_ascii() {
        return None;
    }
    let mut out = String::with_capacity(input.len());
    let mut pending_space = false;
    let mut chars = 0usize;
    for b in input.bytes() {
        let c = b as char;
        // `char::is_whitespace`, not `u8::is_ascii_whitespace`. The two disagree on vertical tab
        // (`\x0B`): the ASCII variant follows the WhatWG Infra definition and excludes it, while
        // the Unicode White_Space property includes it. The slow path uses the Unicode one, so
        // using the ASCII one here would silently make the paths produce different output.
        if c.is_whitespace() {
            if !out.is_empty() {
                pending_space = true;
            }
            continue;
        }
        if c.is_control() {
            continue;
        }
        if pending_space {
            out.push(' ');
            chars += 1;
            pending_space = false;
        }
        out.push(if opts.lowercase {
            c.to_ascii_lowercase()
        } else {
            c
        });
        chars += 1;
        if chars >= opts.max_chars {
            break;
        }
    }
    Some(out)
}

/// Zero-width and bidi-control characters. These are invisible, survive copy/paste, and
/// otherwise cause a query to silently not match an identical-looking document.
#[inline]
fn is_invisible(ch: char) -> bool {
    matches!(ch,
        '\u{00AD}'                 // soft hyphen
        | '\u{200B}'..='\u{200F}'  // ZWSP, ZWNJ, ZWJ, LRM, RLM
        | '\u{202A}'..='\u{202E}'  // bidi embedding/override
        | '\u{2060}'..='\u{2064}'  // word joiner, invisible operators
        | '\u{2066}'..='\u{2069}'  // bidi isolates
        | '\u{FEFF}'               // BOM / ZWNBSP
    )
}

/// Arabic combining marks: harakat, hamza/madda above, Quranic annotation, superscript alef.
#[inline]
fn is_arabic_diacritic(ch: char) -> bool {
    matches!(ch,
        '\u{0610}'..='\u{061A}'    // honorifics, Quranic marks
        | '\u{064B}'..='\u{065F}'  // harakat, hamza above/below, madda
        | '\u{0670}'               // superscript alef
        | '\u{06D6}'..='\u{06ED}'  // Quranic annotation signs
        | '\u{08D3}'..='\u{08FF}'  // Arabic Extended-A marks
    )
}

/// Arabic-Indic (`٠-٩`) and Extended Arabic-Indic (`۰-۹`) digits to ASCII.
///
/// Algerians write Western Arabic numerals, so the ASCII form is the target, not the source.
#[inline]
fn fold_digit(ch: char) -> char {
    match ch {
        '\u{0660}'..='\u{0669}' => char::from(b'0' + (ch as u32 - 0x0660) as u8),
        '\u{06F0}'..='\u{06F9}' => char::from(b'0' + (ch as u32 - 0x06F0) as u8),
        _ => ch,
    }
}

/// Orthographic variant folding. Only applied by [`fold`].
///
/// Algerian writing is inconsistent about hamza placement and final ya/alef-maksura, so these
/// distinctions cost recall without buying precision.
#[inline]
fn fold_letter(ch: char) -> char {
    match ch {
        // alef variants -> bare alef
        '\u{0622}' | '\u{0623}' | '\u{0625}' | '\u{0671}' | '\u{0672}' | '\u{0673}'
        | '\u{0675}' => '\u{0627}',
        // alef maksura -> ya  (and Persian/Urdu ya variants)
        '\u{0649}' | '\u{06CC}' | '\u{06D2}' => '\u{064A}',
        // ta marbuta -> ha
        '\u{0629}' => '\u{0647}',
        // waw with hamza -> waw
        '\u{0624}' => '\u{0648}',
        // ya with hamza -> ya
        '\u{0626}' => '\u{064A}',
        // Persian/Urdu kaf -> Arabic kaf
        '\u{06A9}' | '\u{06AA}' => '\u{0643}',
        _ => ch,
    }
}

/// Collapse every run of Unicode whitespace to a single ASCII space, and trim the ends.
pub fn collapse_whitespace(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut pending = false;
    for ch in input.chars() {
        if ch.is_whitespace() {
            if !out.is_empty() {
                pending = true;
            }
            continue;
        }
        if ch == '\u{0000}' {
            continue;
        }
        if pending {
            out.push(' ');
            pending = false;
        }
        out.push(ch);
    }
    out
}

/// Truncate to `max` chars in place, respecting char boundaries.
fn truncate_chars(s: &mut String, max: usize) {
    if max == 0 {
        s.clear();
        return;
    }
    if let Some((idx, _)) = s.char_indices().nth(max) {
        s.truncate(idx);
        // A trailing space after truncation is noise.
        while s.ends_with(' ') {
            s.pop();
        }
    }
}

/// Split normalised text into whitespace-delimited tokens.
///
/// Deliberately simple: this is for cheap token counting and lexicon lookup, not for indexing.
/// Meilisearch owns real tokenisation.
pub fn tokens(normalized: &str) -> impl Iterator<Item = &str> {
    normalized.split(' ').filter(|t| !t.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_tatweel() {
        assert_eq!(normalize("مليـــــح"), "مليح");
    }

    #[test]
    fn strips_harakat() {
        assert_eq!(normalize("الجَزَائِر"), "الجزائر");
        assert_eq!(normalize("مُحَمَّد"), "محمد");
    }

    #[test]
    fn folds_arabic_indic_digits() {
        assert_eq!(normalize("٢٠٢٦"), "2026");
        assert_eq!(normalize("۱۲۳"), "123");
        assert_eq!(normalize("رقم ٠٥٥٦"), "رقم 0556");
    }

    #[test]
    fn strips_invisibles() {
        // ZWJ and RLM are invisible but break naive matching.
        assert_eq!(normalize("الجزائر\u{200F}"), "الجزائر");
        assert_eq!(normalize("test\u{200B}ing"), "testing");
        assert_eq!(normalize("\u{FEFF}oran"), "oran");
    }

    #[test]
    fn collapses_whitespace() {
        assert_eq!(normalize("  a   b \t\n c  "), "a b c");
        assert_eq!(normalize("\u{00A0}oran\u{00A0}"), "oran");
    }

    #[test]
    fn lowercases_latin() {
        assert_eq!(normalize("SONELGAZ Facture"), "sonelgaz facture");
        assert_eq!(normalize("Béjaïa"), "béjaïa");
    }

    #[test]
    fn nfkc_folds_presentation_forms() {
        // Arabic presentation forms (used by some old sites) compose to the standard letters.
        assert_eq!(normalize("\u{FEDF}\u{FE8E}"), normalize("\u{0644}\u{0627}"));
        // Full-width Latin
        assert_eq!(normalize("ＯＲＡＮ"), "oran");
    }

    #[test]
    fn normalize_preserves_letter_identity() {
        // `normalize` must NOT fold alef variants — exact matches have to stay winnable.
        assert_ne!(normalize("أحمد"), normalize("احمد"));
        assert_ne!(normalize("خدمة"), normalize("خدمه"));
    }

    #[test]
    fn fold_collapses_orthographic_variants() {
        assert_eq!(fold("أحمد"), fold("احمد"));
        assert_eq!(fold("إسلام"), fold("اسلام"));
        assert_eq!(fold("آمنة"), fold("امنه"));
        assert_eq!(fold("خدمة"), "خدمه");
        assert_eq!(fold("مصطفى"), "مصطفي");
        assert_eq!(fold("مسؤول"), "مسوول");
    }

    #[test]
    fn fold_handles_persian_urdu_variants() {
        // Some Algerian sites emit Persian kaf/ya from bad keyboard layouts.
        assert_eq!(fold("کتاب"), fold("كتاب"));
        assert_eq!(fold("علی"), fold("علي"));
    }

    #[test]
    fn arabizi_is_left_alone_apart_from_case() {
        // Digit-consonants must survive: `3` in "3aslema" is ع, not a number to fold.
        assert_eq!(normalize("Wach Rak 3aslema"), "wach rak 3aslema");
        assert_eq!(normalize("ch7al hada"), "ch7al hada");
    }

    #[test]
    fn mixed_script_and_code_switching() {
        assert_eq!(normalize("rani f la gare"), "rani f la gare");
        assert_eq!(normalize("راني في La Gare"), "راني في la gare");
    }

    #[test]
    fn empty_and_whitespace_only() {
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("   \t\n "), "");
        assert_eq!(normalize("\u{200B}\u{200B}"), "");
    }

    #[test]
    fn truncates_on_char_boundary() {
        let opts = NormalizeOptions {
            max_chars: 5,
            ..Default::default()
        };
        // Multi-byte chars must not be split.
        let out = normalize_with("الجزائر العاصمة", &opts);
        assert!(out.chars().count() <= 5);
        assert!(std::str::from_utf8(out.as_bytes()).is_ok());
    }

    #[test]
    fn truncation_does_not_leave_trailing_space() {
        let opts = NormalizeOptions {
            max_chars: 2,
            ..Default::default()
        };
        assert_eq!(normalize_with("ab cd", &opts), "ab");
    }

    #[test]
    fn ascii_fast_path_matches_slow_path() {
        // The fast path is an optimisation; it must not change behaviour.
        let opts = NormalizeOptions::default();
        for s in [
            "Hello   World",
            "  SONELGAZ ",
            "a\tb\nc",
            "",
            "  ",
            "ch7al",
            "a\u{0}b",    // NUL
            "a\u{e}b",    // SHIFT OUT — the character that found this bug
            "a\u{7f}b",   // DEL
            "\u{1}\u{2}", // controls only
            "a\rb\nc",    // CR/LF are whitespace, not to be silently deleted
            "A\u{b}0",    // vertical tab: ASCII and Unicode disagree on whether it is whitespace
            "a\u{c}b",    // form feed
        ] {
            assert_eq!(
                ascii_fast_path(s, &opts).expect("input is ASCII"),
                normalize_slow(s, &opts),
                "fast and slow paths diverged on {s:?}"
            );
        }
    }

    #[test]
    fn control_characters_are_stripped_but_line_breaks_become_spaces() {
        // Invisible controls break matching between two identical-looking strings.
        assert_eq!(normalize("a\u{e}b"), "ab");
        assert_eq!(normalize("\u{1}الجزائر\u{7f}"), "الجزائر");
        // ...but a newline is a word separator, not garbage.
        assert_eq!(normalize("a\nb"), "a b");
        assert_eq!(normalize("a\r\nb"), "a b");
        assert_eq!(normalize("a\tb"), "a b");
    }

    #[test]
    fn nfkc_can_turn_non_ascii_into_ascii() {
        // Regression: `ﬀ` (U+FB00) decomposes to "ff", so the first pass runs the slow path and
        // the second runs the fast path. If the two disagree, normalisation is not idempotent.
        let once = normalize("\u{e}\u{FB00}");
        assert_eq!(once, "ff");
        assert_eq!(
            normalize(&once),
            once,
            "not idempotent across a path switch"
        );
    }

    #[test]
    fn tokens_splits_normalized_text() {
        let n = normalize("  wach   rak khouya ");
        let t: Vec<_> = tokens(&n).collect();
        assert_eq!(t, ["wach", "rak", "khouya"]);

        let empty = normalize("");
        assert_eq!(tokens(&empty).count(), 0);
    }

    proptest::proptest! {
        /// The two implementations must be indistinguishable on every input the fast path
        /// accepts. Asserted directly rather than inferred from idempotency, because a
        /// divergence here shows up as a confusing symmetry failure somewhere else.
        #[test]
        fn prop_fast_path_matches_slow_path(s in "\\PC{0,200}") {
            let opts = NormalizeOptions::default();
            if let Some(fast) = ascii_fast_path(&s, &opts) {
                proptest::prop_assert_eq!(fast, normalize_slow(&s, &opts));
            }
        }

        /// Same, over inputs drawn specifically from the ASCII range including controls.
        #[test]
        fn prop_fast_path_matches_slow_path_ascii(s in "[\\x00-\\x7f]{0,200}") {
            let opts = NormalizeOptions::default();
            let fast = ascii_fast_path(&s, &opts).expect("generated input is ASCII");
            proptest::prop_assert_eq!(fast, normalize_slow(&s, &opts));
        }
    }

    #[test]
    fn idempotent_on_known_inputs() {
        for s in [
            "الجَزَائِر ٢٠٢٦",
            "مليـــــح بزاف",
            "SONELGAZ  Facture",
            "\u{FEFF}test\u{200B}",
            "ＯＲＡＮ",
            "",
        ] {
            let once = normalize(s);
            let twice = normalize(&once);
            assert_eq!(once, twice, "not idempotent for {s:?}");
        }
    }
}
