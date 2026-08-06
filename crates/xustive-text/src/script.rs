//! Writing-system detection.
//!
//! Layer 1 of the language detection cascade: cheap Unicode-block counting that tells us whether
//! text is Arabic-script, Latin-script, or genuinely mixed. The statistical detector and the
//! Darija lexicons build on top of this.

/// Dominant writing system of a piece of text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    Arabic,
    Latin,
    Mixed,
    /// No cased/scripted letters at all: digits, punctuation, emoji, or empty.
    Unknown,
}

/// Ratio of Arabic to Latin letters, ignoring everything else.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScriptRatio {
    pub arabic: usize,
    pub latin: usize,
    /// Letters that are neither Arabic nor Latin (Tifinagh, CJK, …).
    pub other: usize,
}

impl ScriptRatio {
    /// Total letters considered. Digits, punctuation, whitespace and emoji are excluded.
    pub fn total(&self) -> usize {
        self.arabic + self.latin + self.other
    }

    /// Fraction of letters that are Arabic, or 0.0 when there are no letters.
    pub fn arabic_fraction(&self) -> f32 {
        let t = self.total();
        if t == 0 {
            0.0
        } else {
            self.arabic as f32 / t as f32
        }
    }

    /// Fraction of letters that are Latin, or 0.0 when there are no letters.
    pub fn latin_fraction(&self) -> f32 {
        let t = self.total();
        if t == 0 {
            0.0
        } else {
            self.latin as f32 / t as f32
        }
    }
}

/// Fraction of one script needed to call the text as that script rather than `Mixed`.
pub const DEFAULT_DOMINANCE: f32 = 0.60;

/// Count letters by script.
///
/// Digits are deliberately excluded: Arabizi uses `3`, `7`, `9` as consonants, so counting them
/// as evidence either way is wrong.
pub fn ratio(text: &str) -> ScriptRatio {
    let mut r = ScriptRatio {
        arabic: 0,
        latin: 0,
        other: 0,
    };
    for ch in text.chars() {
        if !ch.is_alphabetic() {
            continue;
        }
        if is_arabic(ch) {
            r.arabic += 1;
        } else if ch.is_ascii_alphabetic() || is_latin_extended(ch) {
            r.latin += 1;
        } else {
            r.other += 1;
        }
    }
    r
}

/// Classify text by dominant script using [`DEFAULT_DOMINANCE`].
pub fn detect(text: &str) -> Script {
    detect_with(text, DEFAULT_DOMINANCE)
}

/// Classify text by dominant script with an explicit threshold.
pub fn detect_with(text: &str, dominance: f32) -> Script {
    let r = ratio(text);
    if r.total() == 0 {
        return Script::Unknown;
    }
    if r.arabic_fraction() >= dominance {
        Script::Arabic
    } else if r.latin_fraction() >= dominance {
        Script::Latin
    } else {
        Script::Mixed
    }
}

/// Arabic script blocks, including the Supplement, Extended-A/B and presentation forms.
#[inline]
pub fn is_arabic(ch: char) -> bool {
    matches!(ch,
        '\u{0600}'..='\u{06FF}'    // Arabic
        | '\u{0750}'..='\u{077F}'  // Arabic Supplement
        | '\u{0870}'..='\u{089F}'  // Arabic Extended-B
        | '\u{08A0}'..='\u{08FF}'  // Arabic Extended-A
        | '\u{FB50}'..='\u{FDFF}'  // Presentation Forms-A
        | '\u{FE70}'..='\u{FEFF}'  // Presentation Forms-B
    )
}

/// Latin beyond ASCII: French accents, and the Latin Extended blocks.
#[inline]
pub fn is_latin_extended(ch: char) -> bool {
    matches!(ch,
        '\u{00C0}'..='\u{024F}'    // Latin-1 Supplement letters, Extended-A, Extended-B
        | '\u{1E00}'..='\u{1EFF}'  // Latin Extended Additional
    )
}

/// Tifinagh, the script used for written Tamazight.
///
/// Not a v1 target, but detecting it lets us avoid mislabelling it as `other` noise when the
/// question comes up. See the open question in the RTL/localisation note.
#[inline]
pub fn is_tifinagh(ch: char) -> bool {
    matches!(ch, '\u{2D30}'..='\u{2D7F}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_pure_arabic() {
        assert_eq!(detect("الجزائر العاصمة"), Script::Arabic);
        assert_eq!(detect("واش راك خويا"), Script::Arabic);
    }

    #[test]
    fn detects_pure_latin() {
        assert_eq!(detect("Bonjour tout le monde"), Script::Latin);
        assert_eq!(detect("wach rak khouya"), Script::Latin);
        assert_eq!(detect("Béjaïa"), Script::Latin);
    }

    #[test]
    fn detects_mixed_code_switching() {
        // Real Algerian pattern: Arabic sentence with a French noun phrase.
        assert_eq!(detect("راني في la gare"), Script::Mixed);
    }

    #[test]
    fn digits_do_not_vote() {
        // Arabizi: digits are consonants here, so they must not drag the ratio either way.
        assert_eq!(detect("ch7al 3andek"), Script::Latin);
        let r = ratio("3aslema");
        assert_eq!(r.total(), 6); // "aslema", the 3 is excluded
    }

    #[test]
    fn punctuation_and_emoji_do_not_vote() {
        assert_eq!(detect("الجزائر!!! 😀"), Script::Arabic);
        let r = ratio("!!! 😀 123");
        assert_eq!(r.total(), 0);
    }

    #[test]
    fn empty_is_unknown() {
        assert_eq!(detect(""), Script::Unknown);
        assert_eq!(detect("   "), Script::Unknown);
        assert_eq!(detect("2026"), Script::Unknown);
    }

    #[test]
    fn presentation_forms_count_as_arabic() {
        assert_eq!(detect("\u{FEDF}\u{FE8E}"), Script::Arabic);
    }

    #[test]
    fn fractions_are_consistent() {
        let r = ratio("abc الجزائر"); // 3 latin, 7 arabic
        assert_eq!(r.latin, 3);
        assert_eq!(r.arabic, 7);
        assert!((r.arabic_fraction() + r.latin_fraction() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn tifinagh_recognised_but_not_a_target() {
        assert!(is_tifinagh('\u{2D30}'));
        assert!(!is_arabic('\u{2D30}'));
    }
}
