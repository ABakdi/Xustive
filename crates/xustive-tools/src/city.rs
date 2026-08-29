//! Cities outside Algeria that Algerians ask the weather for.
//!
//! Deliberately a curated list rather than a geocoder. Two reasons, and the second is the one
//! that decides: the tool data plane fetches every place on a schedule and the serving plane
//! only ever reads a cache ([[Tool Data Plane]]) — a geocoder would mean a live lookup on the
//! search path, which this architecture does not have a route for. And a bounded list is a
//! bounded cost at the publisher: adding the whole world would be tens of thousands of requests
//! a day for cities nobody here searches.
//!
//! What earns a place: the Maghreb and the Arab world, the countries of the Algerian diaspora
//! (France above all), the capitals people read about, and the handful of world cities that
//! appear in any newsroom. Everything else is answered honestly — with nothing
//! ([[Instant Answers]] §weather).

use crate::wilaya::fold_for_match;

#[derive(Debug, Clone, PartialEq)]
pub struct City {
    /// Stable identifier, used in the cache key. Never displayed.
    pub slug: &'static str,
    pub name_en: &'static str,
    pub name_fr: &'static str,
    pub name_ar: &'static str,
    pub country_en: &'static str,
    pub country_fr: &'static str,
    pub country_ar: &'static str,
    /// Other ways people write it: short forms (`مكة` for `مكة المكرمة`), transliterations
    /// (`Makkah`), and the name without its administrative suffix (`Kuwait` for `Kuwait City`).
    pub aliases: &'static [&'static str],
    pub latitude: f64,
    pub longitude: f64,
}

macro_rules! city_aliases {
    ($slug:literal, $en:literal, $fr:literal, $ar:literal,
     $c_en:literal, $c_fr:literal, $c_ar:literal, $lat:literal, $lon:literal, $aliases:expr) => {
        City {
            slug: $slug,
            name_en: $en,
            name_fr: $fr,
            name_ar: $ar,
            country_en: $c_en,
            country_fr: $c_fr,
            country_ar: $c_ar,
            aliases: $aliases,
            latitude: $lat,
            longitude: $lon,
        }
    };
}

macro_rules! city {
    ($slug:literal, $en:literal, $fr:literal, $ar:literal,
     $c_en:literal, $c_fr:literal, $c_ar:literal, $lat:literal, $lon:literal) => {
        City {
            slug: $slug,
            name_en: $en,
            name_fr: $fr,
            name_ar: $ar,
            country_en: $c_en,
            country_fr: $c_fr,
            country_ar: $c_ar,
            aliases: &[],
            latitude: $lat,
            longitude: $lon,
        }
    };
}

// One line per city, on purpose: this is a table, and a table is read by scanning columns.
#[rustfmt::skip]
pub const CITIES: &[City] = &[
    // The Maghreb and the neighbours.
    city!("tunis", "Tunis", "Tunis", "تونس", "Tunisia", "Tunisie", "تونس", 36.80, 10.18),
    city_aliases!("casablanca", "Casablanca", "Casablanca", "الدار البيضاء", "Morocco", "Maroc", "المغرب", 33.57, -7.59, &["Casa", "كازابلانكا"]),
    city!("rabat", "Rabat", "Rabat", "الرباط", "Morocco", "Maroc", "المغرب", 34.02, -6.84),
    city!("marrakech", "Marrakesh", "Marrakech", "مراكش", "Morocco", "Maroc", "المغرب", 31.63, -7.99),
    city!("tanger", "Tangier", "Tanger", "طنجة", "Morocco", "Maroc", "المغرب", 35.76, -5.83),
    city!("tripoli-ly", "Tripoli", "Tripoli", "طرابلس", "Libya", "Libye", "ليبيا", 32.89, 13.19),
    city!("nouakchott", "Nouakchott", "Nouakchott", "نواكشوط", "Mauritania", "Mauritanie", "موريتانيا", 18.08, -15.98),
    // France — the diaspora's cities first.
    city!("paris", "Paris", "Paris", "باريس", "France", "France", "فرنسا", 48.86, 2.35),
    city!("marseille", "Marseille", "Marseille", "مرسيليا", "France", "France", "فرنسا", 43.30, 5.37),
    city!("lyon", "Lyon", "Lyon", "ليون", "France", "France", "فرنسا", 45.76, 4.84),
    city!("toulouse", "Toulouse", "Toulouse", "تولوز", "France", "France", "فرنسا", 43.60, 1.44),
    city!("lille", "Lille", "Lille", "ليل", "France", "France", "فرنسا", 50.63, 3.06),
    city!("nice", "Nice", "Nice", "نيس", "France", "France", "فرنسا", 43.70, 7.27),
    city!("montpellier", "Montpellier", "Montpellier", "مونبلييه", "France", "France", "فرنسا", 43.61, 3.88),
    city!("strasbourg", "Strasbourg", "Strasbourg", "ستراسبورغ", "France", "France", "فرنسا", 48.58, 7.75),
    city!("bordeaux", "Bordeaux", "Bordeaux", "بوردو", "France", "France", "فرنسا", 44.84, -0.58),
    city!("grenoble", "Grenoble", "Grenoble", "غرونوبل", "France", "France", "فرنسا", 45.19, 5.72),
    // Europe.
    city_aliases!("london", "London", "Londres", "لندن", "United Kingdom", "Royaume-Uni", "المملكة المتحدة", 51.51, -0.13, &["لندره"]),
    city!("madrid", "Madrid", "Madrid", "مدريد", "Spain", "Espagne", "إسبانيا", 40.42, -3.70),
    city!("barcelona", "Barcelona", "Barcelone", "برشلونة", "Spain", "Espagne", "إسبانيا", 41.39, 2.17),
    city!("alicante", "Alicante", "Alicante", "أليكانتي", "Spain", "Espagne", "إسبانيا", 38.35, -0.48),
    city!("rome", "Rome", "Rome", "روما", "Italy", "Italie", "إيطاليا", 41.90, 12.50),
    city!("milan", "Milan", "Milan", "ميلانو", "Italy", "Italie", "إيطاليا", 45.46, 9.19),
    city!("berlin", "Berlin", "Berlin", "برلين", "Germany", "Allemagne", "ألمانيا", 52.52, 13.40),
    city!("munich", "Munich", "Munich", "ميونخ", "Germany", "Allemagne", "ألمانيا", 48.14, 11.58),
    city!("frankfurt", "Frankfurt", "Francfort", "فرانكفورت", "Germany", "Allemagne", "ألمانيا", 50.11, 8.68),
    city!("brussels", "Brussels", "Bruxelles", "بروكسل", "Belgium", "Belgique", "بلجيكا", 50.85, 4.35),
    city!("amsterdam", "Amsterdam", "Amsterdam", "أمستردام", "Netherlands", "Pays-Bas", "هولندا", 52.37, 4.90),
    city!("geneva", "Geneva", "Genève", "جنيف", "Switzerland", "Suisse", "سويسرا", 46.20, 6.14),
    city!("zurich", "Zurich", "Zurich", "زيوريخ", "Switzerland", "Suisse", "سويسرا", 47.38, 8.54),
    city!("vienna", "Vienna", "Vienne", "فيينا", "Austria", "Autriche", "النمسا", 48.21, 16.37),
    city!("lisbon", "Lisbon", "Lisbonne", "لشبونة", "Portugal", "Portugal", "البرتغال", 38.72, -9.14),
    city!("athens", "Athens", "Athènes", "أثينا", "Greece", "Grèce", "اليونان", 37.98, 23.73),
    city_aliases!("istanbul", "Istanbul", "Istanbul", "إسطنبول", "Türkiye", "Turquie", "تركيا", 41.01, 28.98, &["Estambul", "استانبول"]),
    city!("ankara", "Ankara", "Ankara", "أنقرة", "Türkiye", "Turquie", "تركيا", 39.93, 32.86),
    city!("moscow", "Moscow", "Moscou", "موسكو", "Russia", "Russie", "روسيا", 55.76, 37.62),
    city!("stockholm", "Stockholm", "Stockholm", "ستوكهولم", "Sweden", "Suède", "السويد", 59.33, 18.07),
    city!("oslo", "Oslo", "Oslo", "أوسلو", "Norway", "Norvège", "النرويج", 59.91, 10.75),
    city!("dublin", "Dublin", "Dublin", "دبلن", "Ireland", "Irlande", "أيرلندا", 53.35, -6.26),
    // The Arab world and the Middle East.
    city_aliases!("cairo", "Cairo", "Le Caire", "القاهرة", "Egypt", "Égypte", "مصر", 30.04, 31.24, &["Le Caire", "Cairo"]),
    city!("alexandria", "Alexandria", "Alexandrie", "الإسكندرية", "Egypt", "Égypte", "مصر", 31.20, 29.92),
    city_aliases!("mecca", "Mecca", "La Mecque", "مكة المكرمة", "Saudi Arabia", "Arabie saoudite", "السعودية", 21.39, 39.86, &["مكة", "Makkah", "Makka"]),
    city_aliases!("medina", "Medina", "Médine", "المدينة المنورة", "Saudi Arabia", "Arabie saoudite", "السعودية", 24.47, 39.61, &["المدينة", "Madinah", "Madina"]),
    city!("riyadh", "Riyadh", "Riyad", "الرياض", "Saudi Arabia", "Arabie saoudite", "السعودية", 24.71, 46.68),
    city!("jeddah", "Jeddah", "Djeddah", "جدة", "Saudi Arabia", "Arabie saoudite", "السعودية", 21.49, 39.19),
    city!("dubai", "Dubai", "Dubaï", "دبي", "United Arab Emirates", "Émirats arabes unis", "الإمارات", 25.20, 55.27),
    city_aliases!("abu-dhabi", "Abu Dhabi", "Abou Dabi", "أبو ظبي", "United Arab Emirates", "Émirats arabes unis", "الإمارات", 24.45, 54.38, &["ابوظبي", "أبوظبي"]),
    city!("doha", "Doha", "Doha", "الدوحة", "Qatar", "Qatar", "قطر", 25.29, 51.53),
    city_aliases!("kuwait", "Kuwait City", "Koweït", "الكويت", "Kuwait", "Koweït", "الكويت", 29.38, 47.99, &["Kuwait", "Koweit"]),
    city!("manama", "Manama", "Manama", "المنامة", "Bahrain", "Bahreïn", "البحرين", 26.23, 50.59),
    city!("muscat", "Muscat", "Mascate", "مسقط", "Oman", "Oman", "عُمان", 23.59, 58.41),
    city!("amman", "Amman", "Amman", "عمان", "Jordan", "Jordanie", "الأردن", 31.95, 35.93),
    city!("beirut", "Beirut", "Beyrouth", "بيروت", "Lebanon", "Liban", "لبنان", 33.89, 35.50),
    city!("damascus", "Damascus", "Damas", "دمشق", "Syria", "Syrie", "سوريا", 33.51, 36.29),
    city!("baghdad", "Baghdad", "Bagdad", "بغداد", "Iraq", "Irak", "العراق", 33.31, 44.36),
    city_aliases!("jerusalem", "Jerusalem", "Jérusalem", "القدس", "Palestine", "Palestine", "فلسطين", 31.78, 35.22, &["القدس الشريف", "Al Quds"]),
    city!("gaza", "Gaza", "Gaza", "غزة", "Palestine", "Palestine", "فلسطين", 31.50, 34.47),
    city!("sanaa", "Sanaa", "Sanaa", "صنعاء", "Yemen", "Yémen", "اليمن", 15.37, 44.19),
    city!("khartoum", "Khartoum", "Khartoum", "الخرطوم", "Sudan", "Soudan", "السودان", 15.50, 32.56),
    city!("tehran", "Tehran", "Téhéran", "طهران", "Iran", "Iran", "إيران", 35.69, 51.39),
    // Africa.
    city!("dakar", "Dakar", "Dakar", "داكار", "Senegal", "Sénégal", "السنغال", 14.72, -17.47),
    city!("abidjan", "Abidjan", "Abidjan", "أبيدجان", "Ivory Coast", "Côte d'Ivoire", "ساحل العاج", 5.36, -4.01),
    city!("bamako", "Bamako", "Bamako", "باماكو", "Mali", "Mali", "مالي", 12.64, -8.00),
    city!("niamey", "Niamey", "Niamey", "نيامي", "Niger", "Niger", "النيجر", 13.51, 2.11),
    city!("lagos", "Lagos", "Lagos", "لاغوس", "Nigeria", "Nigeria", "نيجيريا", 6.52, 3.38),
    city!("nairobi", "Nairobi", "Nairobi", "نيروبي", "Kenya", "Kenya", "كينيا", -1.29, 36.82),
    city!("addis-ababa", "Addis Ababa", "Addis-Abeba", "أديس أبابا", "Ethiopia", "Éthiopie", "إثيوبيا", 9.02, 38.75),
    city!("johannesburg", "Johannesburg", "Johannesburg", "جوهانسبرغ", "South Africa", "Afrique du Sud", "جنوب أفريقيا", -26.20, 28.05),
    // The Americas.
    city_aliases!("new-york", "New York", "New York", "نيويورك", "United States", "États-Unis", "الولايات المتحدة", 40.71, -74.01, &["NYC"]),
    city!("washington", "Washington", "Washington", "واشنطن", "United States", "États-Unis", "الولايات المتحدة", 38.91, -77.04),
    city!("los-angeles", "Los Angeles", "Los Angeles", "لوس أنجلوس", "United States", "États-Unis", "الولايات المتحدة", 34.05, -118.24),
    city!("chicago", "Chicago", "Chicago", "شيكاغو", "United States", "États-Unis", "الولايات المتحدة", 41.88, -87.63),
    city!("montreal", "Montreal", "Montréal", "مونتريال", "Canada", "Canada", "كندا", 45.50, -73.57),
    city!("toronto", "Toronto", "Toronto", "تورونتو", "Canada", "Canada", "كندا", 43.65, -79.38),
    city!("ottawa", "Ottawa", "Ottawa", "أوتاوا", "Canada", "Canada", "كندا", 45.42, -75.70),
    city!("sao-paulo", "São Paulo", "São Paulo", "ساو باولو", "Brazil", "Brésil", "البرازيل", -23.55, -46.63),
    city!("buenos-aires", "Buenos Aires", "Buenos Aires", "بوينس آيرس", "Argentina", "Argentine", "الأرجنتين", -34.60, -58.38),
    // Asia and Oceania.
    city!("tokyo", "Tokyo", "Tokyo", "طوكيو", "Japan", "Japon", "اليابان", 35.68, 139.69),
    city!("beijing", "Beijing", "Pékin", "بكين", "China", "Chine", "الصين", 39.90, 116.41),
    city!("shanghai", "Shanghai", "Shanghai", "شنغهاي", "China", "Chine", "الصين", 31.23, 121.47),
    city!("hong-kong", "Hong Kong", "Hong Kong", "هونغ كونغ", "China", "Chine", "الصين", 22.32, 114.17),
    city!("seoul", "Seoul", "Séoul", "سيول", "South Korea", "Corée du Sud", "كوريا الجنوبية", 37.57, 126.98),
    city!("singapore", "Singapore", "Singapour", "سنغافورة", "Singapore", "Singapour", "سنغافورة", 1.35, 103.82),
    city!("bangkok", "Bangkok", "Bangkok", "بانكوك", "Thailand", "Thaïlande", "تايلاند", 13.76, 100.50),
    city!("kuala-lumpur", "Kuala Lumpur", "Kuala Lumpur", "كوالالمبور", "Malaysia", "Malaisie", "ماليزيا", 3.14, 101.69),
    city!("jakarta", "Jakarta", "Jakarta", "جاكرتا", "Indonesia", "Indonésie", "إندونيسيا", -6.21, 106.85),
    city!("delhi", "Delhi", "Delhi", "دلهي", "India", "Inde", "الهند", 28.61, 77.21),
    city!("mumbai", "Mumbai", "Bombay", "مومباي", "India", "Inde", "الهند", 19.08, 72.88),
    city!("karachi", "Karachi", "Karachi", "كراتشي", "Pakistan", "Pakistan", "باكستان", 24.86, 67.01),
    city!("islamabad", "Islamabad", "Islamabad", "إسلام آباد", "Pakistan", "Pakistan", "باكستان", 33.68, 73.05),
    city!("dhaka", "Dhaka", "Dacca", "دكا", "Bangladesh", "Bangladesh", "بنغلاديش", 23.81, 90.41),
    city!("baku", "Baku", "Bakou", "باكو", "Azerbaijan", "Azerbaïdjan", "أذربيجان", 40.41, 49.87),
    city!("sydney", "Sydney", "Sydney", "سيدني", "Australia", "Australie", "أستراليا", -33.87, 151.21),
    city!("melbourne", "Melbourne", "Melbourne", "ملبورن", "Australia", "Australie", "أستراليا", -37.81, 144.96),
];

/// The city a query names, if any — the longest matching name wins, so `new york` beats `york`
/// and a query naming two cities takes the more specific.
pub fn find(query: &str) -> Option<&'static City> {
    let haystack = fold_for_match(query);
    if haystack.is_empty() {
        return None;
    }
    let mut best: Option<(usize, &'static City)> = None;
    for city in CITIES {
        for name in [city.name_en, city.name_fr, city.name_ar]
            .into_iter()
            .chain(city.aliases.iter().copied())
        {
            let folded = fold_for_match(name);
            if folded.is_empty() || !contains_word(&haystack, &folded) {
                continue;
            }
            if best.is_none_or(|(len, _)| folded.chars().count() > len) {
                best = Some((folded.chars().count(), city));
            }
        }
    }
    best.map(|(_, c)| c)
}

/// Whole-word containment: `nice` must not match `nice weather`… but it must match `meteo nice`.
/// The distinction is word boundaries, which a plain `contains` does not have — and without it
/// `Nice` matched half the English queries in the corpus.
fn contains_word(haystack: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(at) = haystack[from..].find(needle) {
        let start = from + at;
        let end = start + needle.len();
        let before_ok = start == 0 || haystack[..start].ends_with(' ');
        let after_ok = end == haystack.len() || haystack[end..].starts_with(' ');
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
        if from >= haystack.len() {
            break;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cities_are_found_in_three_languages() {
        assert_eq!(find("weather paris").unwrap().slug, "paris");
        assert_eq!(find("météo à Londres").unwrap().slug, "london");
        assert_eq!(find("الطقس في باريس").unwrap().slug, "paris");
        assert_eq!(find("weather in new york").unwrap().slug, "new-york");
        assert_eq!(find("درجة الحرارة في مكة المكرمة").unwrap().slug, "mecca");
        // The short form is what people actually type.
        assert_eq!(find("الطقس في مكة").unwrap().slug, "mecca");
        assert_eq!(find("weather makkah").unwrap().slug, "mecca");
        assert_eq!(find("weather kuwait").unwrap().slug, "kuwait");
    }

    #[test]
    fn a_name_inside_another_word_is_not_a_match() {
        assert!(find("weather nicely").is_none());
        assert!(find("romee").is_none());
    }

    #[test]
    fn every_city_is_unique_and_plausible() {
        let mut slugs: Vec<&str> = CITIES.iter().map(|c| c.slug).collect();
        slugs.sort_unstable();
        let before = slugs.len();
        slugs.dedup();
        assert_eq!(before, slugs.len(), "duplicate slug");
        for c in CITIES {
            assert!((-90.0..=90.0).contains(&c.latitude), "{}", c.slug);
            assert!((-180.0..=180.0).contains(&c.longitude), "{}", c.slug);
        }
    }
}
