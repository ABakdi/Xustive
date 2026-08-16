//! Wilaya gazetteer hinting (M2-T06.5).
//!
//! Which of Algeria's 58 wilayas is a document about? For an Algeria-first engine this is a
//! first-class signal: "floods in Béchar" and "floods in Annaba" are different stories, and a
//! reader in one wilaya wants the local one first.
//!
//! The method is deliberately plain: scan the text for wilaya names, count the mentions, and hint
//! the one named most. A gazetteer, not a model — the set of names is fixed, small, and known, so a
//! lookup is both cheaper and more predictable than anything learned, and it never invents a place
//! that is not there.
//!
//! # Matching
//!
//! Names are matched after folding ([`xustive_text::fold`]), which is what the search index and the
//! language detector already use, so `الجزائر` and `الجَزائر` match and casing on the French names
//! does not matter. Matching is whole-token, not substring: `عنابة` must appear as a word, because a
//! substring match turns every longer word that happens to contain a short wilaya name into a false
//! hit.
//!
//! # The data
//!
//! Mirrors `xustive-tools`' `WILAYAS` — the same 58 wilayas, generated from that source so the two
//! do not drift. Only the names are needed here; coordinates and dial codes are the tool's concern.
//! A test asserts the count is 58, which catches a table truncated in editing.

use std::collections::HashMap;

use xustive_text::{fold, tokens};

/// `(code, name_ar, name_fr)` for all 58 wilayas.
const WILAYAS: &[(u8, &str, &str)] = &[
    (1, "أدرار", "Adrar"),
    (2, "الشلف", "Chlef"),
    (3, "الأغواط", "Laghouat"),
    (4, "أم البواقي", "Oum El Bouaghi"),
    (5, "باتنة", "Batna"),
    (6, "بجاية", "Béjaïa"),
    (7, "بسكرة", "Biskra"),
    (8, "بشار", "Béchar"),
    (9, "البليدة", "Blida"),
    (10, "البويرة", "Bouira"),
    (11, "تمنراست", "Tamanrasset"),
    (12, "تبسة", "Tébessa"),
    (13, "تلمسان", "Tlemcen"),
    (14, "تيارت", "Tiaret"),
    (15, "تيزي وزو", "Tizi Ouzou"),
    (16, "الجزائر", "Alger"),
    (17, "الجلفة", "Djelfa"),
    (18, "جيجل", "Jijel"),
    (19, "سطيف", "Sétif"),
    (20, "سعيدة", "Saïda"),
    (21, "سكيكدة", "Skikda"),
    (22, "سيدي بلعباس", "Sidi Bel Abbès"),
    (23, "عنابة", "Annaba"),
    (24, "قالمة", "Guelma"),
    (25, "قسنطينة", "Constantine"),
    (26, "المدية", "Médéa"),
    (27, "مستغانم", "Mostaganem"),
    (28, "المسيلة", "M'Sila"),
    (29, "معسكر", "Mascara"),
    (30, "ورقلة", "Ouargla"),
    (31, "وهران", "Oran"),
    (32, "البيض", "El Bayadh"),
    (33, "إليزي", "Illizi"),
    (34, "برج بوعريريج", "Bordj Bou Arréridj"),
    (35, "بومرداس", "Boumerdès"),
    (36, "الطارف", "El Tarf"),
    (37, "تندوف", "Tindouf"),
    (38, "تيسمسيلت", "Tissemsilt"),
    (39, "الوادي", "El Oued"),
    (40, "خنشلة", "Khenchela"),
    (41, "سوق أهراس", "Souk Ahras"),
    (42, "تيبازة", "Tipaza"),
    (43, "ميلة", "Mila"),
    (44, "عين الدفلى", "Aïn Defla"),
    (45, "النعامة", "Naâma"),
    (46, "عين تموشنت", "Aïn Témouchent"),
    (47, "غرداية", "Ghardaïa"),
    (48, "غليزان", "Relizane"),
    (49, "تيميمون", "Timimoun"),
    (50, "برج باجي مختار", "Bordj Badji Mokhtar"),
    (51, "أولاد جلال", "Ouled Djellal"),
    (52, "بني عباس", "Béni Abbès"),
    (53, "عين صالح", "In Salah"),
    (54, "عين قزام", "In Guezzam"),
    (55, "تقرت", "Touggourt"),
    (56, "جانت", "Djanet"),
    (57, "المغير", "El M'Ghair"),
    (58, "المنيعة", "El Meniaa"),
];

/// The wilaya a document is about, if one is named clearly enough.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WilayaHint {
    pub code: u8,
    /// The Arabic name, as stored on the document.
    pub name: &'static str,
    /// How many times it was named. The confidence, such as it is.
    pub mentions: usize,
}

/// Hint the wilaya a text is about: the one named most often.
///
/// Returns `None` when no wilaya is named, or when the leader is named only once *and* tied with
/// another — a single ambiguous mention is not enough to geolocate a document, and guessing between
/// two would be worse than saying nothing. A clear single mention (no tie) does hint, because most
/// local stories name their wilaya exactly once.
pub fn detect_wilaya(text: &str) -> Option<WilayaHint> {
    // Fold and tokenise once; membership is a set lookup per token.
    let folded = fold(text);
    let toks: Vec<&str> = tokens(&folded).collect();
    if toks.is_empty() {
        return None;
    }
    let present: std::collections::HashSet<&str> = toks.iter().copied().collect();

    // Single-token names are matched by set membership; multi-word French names (e.g. "El Meniaa",
    // "Tizi Ouzou") by looking for their folded token sequence in the token stream.
    let mut counts: HashMap<u8, (usize, &'static str)> = HashMap::new();
    for &(code, ar, fr) in WILAYAS {
        let mut n = 0usize;
        for name in [ar, fr] {
            n += count_name(&toks, &present, name);
        }
        if n > 0 {
            counts.entry(code).or_insert((0, ar)).0 += n;
        }
    }

    let mut ranked: Vec<(u8, usize, &'static str)> =
        counts.into_iter().map(|(c, (n, ar))| (c, n, ar)).collect();
    // Most mentions first; break ties by lowest code so the result is deterministic.
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    let (code, mentions, name) = *ranked.first()?;
    // Reject a lone, tied mention: two different wilayas named once each is not a location.
    if mentions == 1 && ranked.iter().filter(|r| r.1 == 1).count() > 1 {
        return None;
    }
    Some(WilayaHint {
        code,
        name,
        mentions,
    })
}

/// Count occurrences of a wilaya name (its folded tokens) in the token stream.
fn count_name(toks: &[&str], present: &std::collections::HashSet<&str>, name: &str) -> usize {
    let folded = fold(name);
    let parts: Vec<&str> = tokens(&folded).collect();
    match parts.as_slice() {
        [] => 0,
        // Single token: a set lookup, then count exact matches.
        [one] => {
            if present.contains(one) {
                toks.iter().filter(|t| *t == one).count()
            } else {
                0
            }
        }
        // Multi-token: count non-overlapping runs of the sequence.
        seq => toks.windows(seq.len()).filter(|w| w == &seq).count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_has_all_58_wilayas() {
        assert_eq!(WILAYAS.len(), 58, "a wilaya was lost in editing");
        // Codes are 1..=58 with none missing.
        let mut codes: Vec<u8> = WILAYAS.iter().map(|w| w.0).collect();
        codes.sort_unstable();
        assert_eq!(codes, (1..=58).collect::<Vec<_>>());
    }

    #[test]
    fn a_clearly_named_wilaya_is_hinted() {
        let h = detect_wilaya("فيضانات في ولاية بشار خلفت أضرارا في بشار").unwrap();
        assert_eq!(h.name, "بشار");
        assert_eq!(h.mentions, 2);
    }

    #[test]
    fn the_most_named_wilaya_wins() {
        // Annaba named twice, Oran once.
        let h = detect_wilaya("مباراة في عنابة ثم عنابة بعد وهران").unwrap();
        assert_eq!(h.name, "عنابة");
    }

    #[test]
    fn a_french_name_is_matched() {
        let h = detect_wilaya("un match a Oran hier soir a Oran").unwrap();
        assert_eq!(h.code, 31); // Oran
    }

    #[test]
    fn a_multi_word_french_name_matches_as_a_sequence() {
        let h = detect_wilaya("les resultats a Tizi Ouzou cette annee").unwrap();
        assert_eq!(h.code, 15); // Tizi Ouzou
    }

    #[test]
    fn no_wilaya_named_is_none() {
        assert!(detect_wilaya("the ministry announced a new policy today").is_none());
    }

    /// Two wilayas named once each is not a location — better to say nothing than guess.
    #[test]
    fn a_single_tied_mention_is_ambiguous() {
        assert!(detect_wilaya("طريق بين وهران و عنابة").is_none());
    }

    /// But one clear single mention does hint — most local stories name their wilaya once.
    #[test]
    fn one_clear_single_mention_hints() {
        let h = detect_wilaya("حادث مرور في ولاية سطيف").unwrap();
        assert_eq!(h.name, "سطيف");
        assert_eq!(h.mentions, 1);
    }
}
