//! Weather: the *detector* only.
//!
//! Deliberately no data and no I/O. Matchers are pure and total by contract — they run on every
//! query, and one that reached for a cache would put a Redis round trip on the search path for
//! every search that is not about weather.
//!
//! So this decides whether a query is a weather question and which wilaya it names. Filling in the
//! answer from the cache is the serving plane's job, because the serving plane is the only thing
//! that has a cache handle.

use crate::wilaya::{self, Wilaya};

/// A recognised weather question.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub wilaya: &'static Wilaya,
    /// Whether the reader actually named this place.
    ///
    /// False means the wilaya is a fallback, and the caller may replace it with a better guess —
    /// and, either way, must say on the card which place it assumed. A wrong city stated
    /// confidently is the failure mode worth avoiding here (M8-T05.6).
    pub named: bool,
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

    let named = wilaya::find(query);

    Some(Request {
        wilaya: named.unwrap_or_else(wilaya::default_wilaya),
        named: named.is_some(),
        // Naming a place alongside a trigger is a much stronger signal than the trigger alone:
        // `طقس وهران` is unambiguous, `weather` on its own could be a search about climate.
        confidence: match (named.is_some(), trigger.len() > 6) {
            (true, _) => 0.94,
            (false, true) => 0.82,
            (false, false) => 0.7,
        },
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
            assert_eq!(found.wilaya.name_fr, expected, "{query:?}");
        }
    }

    #[test]
    fn a_trigger_without_a_place_means_here() {
        // `طقس` alone is a legitimate question. The card names the wilaya it used and offers a
        // control, rather than asking the browser for a location it does not need.
        let found = detect("طقس").expect("a bare trigger should still ask about weather");
        assert_eq!(found.wilaya.code, 16, "defaults to Algiers");
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
