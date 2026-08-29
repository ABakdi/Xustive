//! A place a weather question can be about: one of the 58 wilayas, or one of the world cities
//! in [`crate::city`].
//!
//! Algeria first, always — `Oran` is the wilaya, never a city elsewhere that happens to share
//! part of the name — and everything else is looked up only when no wilaya matched.

use crate::city::{self, City};
use crate::wilaya::{self, Wilaya};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place {
    Wilaya(&'static Wilaya),
    City(&'static City),
}

impl Place {
    /// The place a query names, or nothing. Wilayas win ties by being tried first.
    pub fn find(query: &str) -> Option<Self> {
        wilaya::find(query)
            .map(Place::Wilaya)
            .or_else(|| city::find(query).map(Place::City))
    }

    /// Stable cache key. Wilayas keep their bare code so entries written before world cities
    /// existed are still read; cities are namespaced.
    pub fn key(&self) -> String {
        match self {
            Place::Wilaya(w) => w.code.to_string(),
            Place::City(c) => format!("c-{}", c.slug),
        }
    }

    pub fn name(&self, lang: &str) -> &'static str {
        match (self, lang) {
            (Place::Wilaya(w), "fr" | "en") => w.name_fr,
            (Place::Wilaya(w), _) => w.name_ar,
            (Place::City(c), "fr") => c.name_fr,
            (Place::City(c), "en") => c.name_en,
            (Place::City(c), _) => c.name_ar,
        }
    }

    /// What the card writes under the name: nothing for a wilaya (the reader knows where they
    /// are), the country for a city (`Paris` alone could be the one in Texas).
    pub fn country(&self, lang: &str) -> Option<&'static str> {
        match (self, lang) {
            (Place::Wilaya(_), _) => None,
            (Place::City(c), "fr") => Some(c.country_fr),
            (Place::City(c), "en") => Some(c.country_en),
            (Place::City(c), _) => Some(c.country_ar),
        }
    }

    pub fn coordinates(&self) -> (f64, f64) {
        match self {
            Place::Wilaya(w) => (w.latitude, w.longitude),
            Place::City(c) => (c.latitude, c.longitude),
        }
    }

    /// Plausible temperatures, for the validator. Algeria's range is tight enough to catch a
    /// sensor fault; the world's has to hold Siberia and the Gulf in the same bounds.
    pub fn temperature_bounds(&self) -> (f64, f64) {
        match self {
            Place::Wilaya(_) => (-25.0, 51.0),
            Place::City(_) => (-70.0, 60.0),
        }
    }

    pub fn is_wilaya(&self) -> bool {
        matches!(self, Place::Wilaya(_))
    }
}

impl Default for Place {
    fn default() -> Self {
        Place::Wilaya(wilaya::default_wilaya())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn algeria_wins_over_the_world() {
        // Both tables could match parts of these; the wilaya must win.
        assert_eq!(Place::find("météo Oran").unwrap().name("fr"), "Oran");
        assert_eq!(Place::find("weather Alger").unwrap().name("fr"), "Alger");
    }

    #[test]
    fn a_world_city_resolves_when_no_wilaya_does() {
        let p = Place::find("weather paris").unwrap();
        assert_eq!(p.name("fr"), "Paris");
        assert_eq!(p.country("fr"), Some("France"));
        assert_eq!(p.country("en"), Some("France"));
        assert_eq!(p.country("ar"), Some("فرنسا"));
        assert_eq!(p.key(), "c-paris");
        assert!(!p.is_wilaya());
    }

    #[test]
    fn a_wilaya_key_is_still_its_bare_code() {
        // Cache entries written before world cities existed must still be found.
        assert_eq!(Place::find("طقس وهران").unwrap().key(), "31");
    }
}
