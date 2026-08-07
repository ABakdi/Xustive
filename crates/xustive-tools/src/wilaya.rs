//! Wilaya reference.
//!
//! Postal codes, dial codes, seats and coordinates for all 58 wilayas. Static and compiled in:
//! this changes by decree every few years, and a network call to answer "what is the postal code
//! for Béjaïa" would be a network call on every such search.
//!
//! It is also the lookup [`crate::prayer`] and, later, weather use to turn a place named in a
//! query into coordinates — without ever asking the browser for a location.

use crate::{Answer, Tool};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wilaya {
    pub code: u8,
    pub name_ar: &'static str,
    pub name_fr: &'static str,
    /// The seat, not a centroid. A centroid of Tamanrasset sits in empty desert hundreds of
    /// kilometres from anyone, which would make its prayer times wrong for every actual resident.
    pub latitude: f64,
    pub longitude: f64,
    pub postal: &'static str,
    pub dial: &'static str,
}

pub use crate::wilaya_data::WILAYAS;

/// Algiers. Used when a query names no wilaya.
///
/// A default rather than a geolocation prompt: a search box that asks for location permission to
/// answer a question is asking for something it does not need, and the wilaya is a visible,
/// changeable control on the card.
pub fn default_wilaya() -> &'static Wilaya {
    by_code(16).expect("Algiers is code 16")
}

pub fn by_code(code: u8) -> Option<&'static Wilaya> {
    WILAYAS.iter().find(|w| w.code == code)
}

/// Find a wilaya named anywhere in a query.
///
/// Matches on the folded form, so `بجايه` finds `بجاية` and `Bejaia` finds `Béjaïa` — the two
/// spellings people actually type, neither of which is the canonical one.
///
/// Longest name first, so `عين تموشنت` is not matched as `عين صالح`'s prefix, and `سيدي بلعباس`
/// is not swallowed by a shorter entry.
pub fn find(query: &str) -> Option<&'static Wilaya> {
    let haystack = fold_for_match(query);
    if haystack.is_empty() {
        return None;
    }

    let mut best: Option<(usize, &'static Wilaya)> = None;
    for wilaya in WILAYAS {
        for name in [wilaya.name_ar, wilaya.name_fr] {
            let folded = fold_for_match(name);
            // Two characters is not a name; matching on it would make every query containing
            // those letters a wilaya lookup.
            if folded.chars().count() < 3 || !haystack.contains(&folded) {
                continue;
            }
            if best.is_none_or(|(len, _)| folded.chars().count() > len) {
                best = Some((folded.chars().count(), wilaya));
            }
        }
    }
    best.map(|(_, w)| w)
}

/// Fold a place name for matching, in both scripts.
///
/// `xustive_text::fold` handles Arabic — بجاية and بجايه collapse to one form — but deliberately
/// leaves Latin diacritics alone, because it mirrors what the search index does and Meilisearch
/// folds Latin itself. Changing it there would alter indexing behaviour to fix a lookup-table
/// problem.
///
/// Here the diacritics matter. Nobody types `Béjaïa` or `Sétif` into a search box; they type
/// `bejaia` and `setif`, and a lookup that only matches the canonical spelling matches nothing.
pub fn fold_for_match(input: &str) -> String {
    xustive_text::fold(input)
        .chars()
        .map(|c| match c {
            'à' | 'â' | 'ä' | 'á' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' | 'í' | 'ì' => 'i',
            'ô' | 'ö' | 'ó' | 'ò' | 'õ' => 'o',
            'ù' | 'û' | 'ü' | 'ú' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            // Apostrophes and hyphens are written inconsistently: M'Sila, MSila, M-Sila.
            '\'' | '’' | '-' => ' ',
            other => other,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub struct WilayaTool;

impl Tool for WilayaTool {
    fn name(&self) -> &'static str {
        "wilaya"
    }

    fn keyword(&self) -> &'static str {
        "wilaya"
    }

    fn answer(&self, query: &str) -> Option<Answer> {
        const TRIGGERS: &[&str] = &[
            "code postal",
            "الرمز البريدي",
            "رمز بريدي",
            "ولاية رقم",
            "indicatif",
            "رمز الهاتف",
            "postal code",
            "dial code",
        ];

        let folded = fold_for_match(query);
        let triggered = TRIGGERS.iter().any(|t| folded.contains(&fold_for_match(t)));

        // A bare `ولاية 06` names a wilaya by number, which is unambiguous enough to answer
        // without a trigger phrase.
        let by_number = query
            .split(|c: char| !c.is_ascii_digit())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse::<u8>().ok())
            .find_map(by_code)
            .filter(|_| folded.contains(&fold_for_match("ولاية")) || triggered);

        let wilaya = by_number.or_else(|| triggered.then(|| find(query)).flatten())?;

        Some(Answer {
            tool: self.name(),
            confidence: 0.88,
            interpretation: format!("{} · {}", wilaya.name_ar, wilaya.name_fr),
            value: format!(
                "{} · الرمز البريدي {}000 · الهاتف {}",
                wilaya.code, wilaya.postal, wilaya.dial
            ),
            detail: Some(serde_json::json!({
                "code": wilaya.code,
                "name_ar": wilaya.name_ar,
                "name_fr": wilaya.name_fr,
                "postal": wilaya.postal,
                "dial": wilaya.dial,
                "latitude": wilaya.latitude,
                "longitude": wilaya.longitude,
            })),
            // Static reference data. It changes by decree, not by the hour.
            as_of: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_fifty_eight_wilayas_are_present_and_numbered_once() {
        assert_eq!(
            WILAYAS.len(),
            58,
            "Algeria has 58 wilayas since the 2019 reorganisation"
        );
        let mut codes: Vec<u8> = WILAYAS.iter().map(|w| w.code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), 58, "duplicate wilaya codes");
        assert_eq!(*codes.first().unwrap(), 1);
        assert_eq!(*codes.last().unwrap(), 58);
    }

    #[test]
    fn every_coordinate_is_inside_algeria() {
        // A transposed latitude and longitude is the classic data-entry error, and it produces
        // prayer times and weather for the wrong continent while looking entirely plausible.
        for w in WILAYAS {
            assert!(
                (18.0..=38.0).contains(&w.latitude),
                "{} latitude {} is outside Algeria",
                w.name_fr,
                w.latitude
            );
            assert!(
                (-9.0..=12.0).contains(&w.longitude),
                "{} longitude {} is outside Algeria",
                w.name_fr,
                w.longitude
            );
        }
    }

    #[test]
    fn names_are_matched_in_both_scripts_and_common_spellings() {
        for (query, expected) in [
            ("مواقيت الصلاة وهران", "Oran"),
            ("heure priere Oran", "Oran"),
            ("code postal bejaia", "Béjaïa"),
            ("الرمز البريدي بجاية", "Béjaïa"),
            // The folded form, which is how people type it.
            ("الرمز البريدي بجايه", "Béjaïa"),
            ("meteo Constantine", "Constantine"),
        ] {
            let found = find(query).unwrap_or_else(|| panic!("{query:?} found nothing"));
            assert_eq!(found.name_fr, expected, "{query:?}");
        }
    }

    #[test]
    fn the_longest_matching_name_wins() {
        // `عين تموشنت` contains `عين`, which several wilayas share. A shortest-match rule would
        // answer with the wrong province and look entirely confident about it.
        assert_eq!(find("الرمز البريدي عين تموشنت").unwrap().code, 46);
        assert_eq!(find("code postal Sidi Bel Abbès").unwrap().code, 22);
    }

    #[test]
    fn a_query_naming_no_wilaya_finds_none() {
        assert!(find("سعر صرف الأورو").is_none());
        assert!(find("").is_none());
    }

    #[test]
    fn the_tool_needs_a_trigger_not_just_a_place_name() {
        // Every article about Oran mentions Oran. Without a trigger phrase, half the corpus
        // would come back as a postal-code lookup.
        assert!(WilayaTool.answer("وهران").is_none());
        assert!(WilayaTool.answer("Oran football").is_none());
        assert!(WilayaTool.answer("code postal Oran").is_some());
    }

    #[test]
    fn a_wilaya_number_is_answered() {
        let answer = WilayaTool
            .answer("ولاية 06")
            .expect("a numbered wilaya should answer");
        assert!(
            answer.interpretation.contains("بجاية"),
            "got {}",
            answer.interpretation
        );
    }

    #[test]
    fn the_answer_carries_the_data_a_card_needs() {
        let detail = WilayaTool
            .answer("code postal Oran")
            .unwrap()
            .detail
            .unwrap();
        assert_eq!(detail["code"], 31);
        assert_eq!(detail["dial"], "041");
        assert!(detail["latitude"].is_number());
    }
}
