//! Weather: the *detector* only.
//!
//! Deliberately no data and no I/O. Matchers are pure and total by contract — they run on every
//! query, and one that reached for a cache would put a Redis round trip on the search path for
//! every search that is not about weather.
//!
//! So this decides whether a query is a weather question and which wilaya it names. Filling in the
//! answer from the cache is the serving plane's job, because the serving plane is the only thing
//! that has a cache handle.

use crate::place::Place;
use crate::wilaya;

/// A recognised weather question.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub place: Place,
    /// Whether the reader actually named this place.
    ///
    /// False means the place is a fallback, and the caller may replace it with a better guess —
    /// and, either way, must say on the card which place it assumed. A wrong city stated
    /// confidently is the failure mode worth avoiding here (M8-T05.6).
    pub named: bool,
    /// The query named a place, and it is not one we hold forecasts for — `weather kinshasa`.
    ///
    /// The caller must answer **nothing**: showing the reader's own city under a question about
    /// somewhere else is not a fallback, it is a wrong answer with a confident face
    /// (reported 2026-08-29 — `weather paris` returned Algiers).
    pub unknown_place: bool,
    /// Confidence, carried through to the answer the caller builds.
    pub confidence: f32,
}

const TRIGGERS: &[&str] = &[
    "طقس",
    "الطقس",
    "حالة الطقس",
    "درجة الحرارة",
    "الحرارة",
    "meteo",
    "la meteo",
    "temps qu il fait",
    "temperature",
    "weather",
    "forecast",
];

/// Whether this query is asking about the weather, and where.
///
/// A named wilaya is not required — `طقس` alone means "here", and the card offers a control to
/// change it. What *is* required is a trigger word: every article about a heatwave mentions
/// temperature, and without one half the corpus would come back as a weather card.
pub fn detect(query: &str) -> Option<Request> {
    let folded = wilaya::fold_for_match(query);
    let trigger = TRIGGERS
        .iter()
        .find(|t| folded.contains(&wilaya::fold_for_match(t)))?;

    let named = Place::find(query);
    // Nothing matched — but did they *name* somewhere? Whatever is left after the trigger and
    // the words that glue a question together is a place we do not know.
    let unknown_place = named.is_none() && names_somewhere(&folded, trigger);

    Some(Request {
        place: named.unwrap_or_default(),
        named: named.is_some(),
        unknown_place,
        // Naming a place alongside a trigger is a much stronger signal than the trigger alone:
        // `طقس وهران` is unambiguous, `weather` on its own could be a search about climate.
        confidence: match (named.is_some(), trigger.len() > 6) {
            (true, _) => 0.94,
            (false, true) => 0.82,
            (false, false) => 0.7,
        },
    })
}

/// Whether anything is left of the query once the trigger and the connective tissue are removed.
///
/// `weather` → nothing left, so the question is about here. `weather tomorrow` → nothing left
/// either, because `tomorrow` is on the list. `weather kinshasa` → something left, and that
/// something is a place this tool cannot answer for.
fn names_somewhere(folded_query: &str, trigger: &str) -> bool {
    const FILLER: &[&str] = &[
        // Connectives and articles.
        "in",
        "at",
        "of",
        "for",
        "the",
        "a",
        "is",
        "it",
        "what",
        "whats",
        "how",
        "like",
        "today",
        "tomorrow",
        "tonight",
        "now",
        "current",
        "currently",
        "this",
        "week",
        "weekend",
        "next",
        "en",
        "a",
        "au",
        "aux",
        "de",
        "du",
        "des",
        "la",
        "le",
        "les",
        "il",
        "fait",
        "quel",
        "quelle",
        "aujourd",
        "hui",
        "aujourdhui",
        "demain",
        "ce",
        "cette",
        "soir",
        "semaine",
        "maintenant",
        "actuel",
        "actuelle",
        "prevision",
        "previsions",
        "في",
        "فى",
        "ب",
        "ال",
        "اليوم",
        "غدا",
        "غدًا",
        "بكرا",
        "الان",
        "الآن",
        "هذا",
        "هذه",
        "الاسبوع",
        "الليلة",
        "كيف",
        "ما",
        "هو",
        "حال",
        "توقعات",
        "درجة",
        "حرارة",
        // The triggers themselves are stripped below, but their pieces appear alone too.
        "weather",
        "forecast",
        "meteo",
        "temperature",
        "temps",
        "طقس",
        "الطقس",
        "الحرارة",
    ];
    let mut rest = folded_query.replace(&wilaya::fold_for_match(trigger), " ");
    for word in TRIGGERS {
        rest = rest.replace(&wilaya::fold_for_match(word), " ");
    }
    rest.split_whitespace().any(|w| {
        // A number is a date or a temperature, not a place.
        !FILLER.contains(&w) && !w.chars().all(|c| c.is_ascii_digit()) && w.chars().count() > 1
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weather_questions_are_recognised_in_three_languages() {
        for (query, expected) in [
            ("طقس وهران", "Oran"),
            ("الطقس في قسنطينة", "Constantine"),
            ("meteo Alger", "Alger"),
            ("weather Batna", "Batna"),
            ("température Annaba", "Annaba"),
        ] {
            let found = detect(query).unwrap_or_else(|| panic!("{query:?} not detected"));
            assert_eq!(found.place.name("fr"), expected, "{query:?}");
        }
    }

    #[test]
    fn a_trigger_without_a_place_means_here() {
        // `طقس` alone is a legitimate question. The card names the wilaya it used and offers a
        // control, rather than asking the browser for a location it does not need.
        let found = detect("طقس").expect("a bare trigger should still ask about weather");
        assert_eq!(found.place.name("fr"), "Alger", "defaults to Algiers");
        assert!(
            !found.unknown_place,
            "naming nothing is not naming something unknown"
        );
        assert!(found.confidence < 0.9, "less certain without a place");
    }

    #[test]
    fn naming_a_place_raises_confidence() {
        let bare = detect("weather").unwrap();
        let placed = detect("weather Oran").unwrap();
        assert!(placed.confidence > bare.confidence);
    }

    #[test]
    fn a_place_name_alone_is_not_a_weather_question() {
        // Every article about a heatwave mentions temperature. Without a trigger word half the
        // corpus would come back as a weather card.
        assert!(detect("وهران").is_none());
        assert!(detect("Oran match").is_none());
        assert!(detect("الجزائر").is_none());
    }

    #[test]
    fn detection_does_no_io_and_is_fast() {
        // The contract every matcher is held to: it runs on every query, so it must be cheap.
        //
        // This test earned its place. The first version folded all 116 wilaya names on every
        // call and took 720 µs — seven times the budget. Precomputing the folded table took it
        // to roughly 60 µs in debug and well under 10 µs in release.
        //
        // The threshold is scaled for debug builds, which run this an order of magnitude slower
        // than the release binary that ships. Failing on a build nobody deploys would be as
        // useless as passing on one nobody measures.
        let budget = if cfg!(debug_assertions) { 1_500 } else { 200 };
        let started = std::time::Instant::now();
        for _ in 0..10_000 {
            let _ = detect("طقس وهران");
        }
        assert!(
            started.elapsed().as_millis() < budget,
            "10 000 detections took {:?}",
            started.elapsed()
        );
    }
}

#[cfg(test)]
mod places {
    use super::*;

    #[test]
    fn a_world_city_is_answered_for_itself() {
        for (query, expected) in [
            ("weather paris", "Paris"),
            ("météo à Londres", "Londres"),
            ("الطقس في مكة المكرمة", "مكة المكرمة"),
        ] {
            let r = detect(query).unwrap_or_else(|| panic!("{query:?}"));
            let lang = if query.starts_with("weather") {
                "en"
            } else if query.starts_with("météo") {
                "fr"
            } else {
                "ar"
            };
            assert_eq!(r.place.name(lang), expected, "{query:?}");
            assert!(r.named && !r.unknown_place, "{query:?}");
        }
    }

    #[test]
    fn a_place_we_do_not_hold_is_flagged_rather_than_replaced() {
        // The bug this exists for: `weather paris` used to answer for Algiers. Anything we
        // cannot answer for must be *refused*, not silently swapped for the reader's own city.
        for query in ["weather kinshasa", "météo Ouagadougou", "الطقس في سمرقند"] {
            let r = detect(query).unwrap_or_else(|| panic!("{query:?}"));
            assert!(r.unknown_place, "{query:?} should be flagged unknown");
            assert!(!r.named, "{query:?}");
        }
    }

    #[test]
    fn a_question_about_here_is_not_an_unknown_place() {
        for query in [
            "weather",
            "weather today",
            "météo aujourd'hui",
            "la meteo maintenant",
            "الطقس اليوم",
            "weather this weekend",
            "forecast tomorrow",
            "درجة الحرارة الان",
        ] {
            let r = detect(query).unwrap_or_else(|| panic!("{query:?}"));
            assert!(
                !r.unknown_place,
                "{query:?} names nowhere, so it means here"
            );
        }
    }
}
