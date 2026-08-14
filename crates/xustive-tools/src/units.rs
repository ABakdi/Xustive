//! Unit conversion.
//!
//! Includes **qintar** and **sa'a**, which no international converter carries and which Algerians
//! use daily for produce and land. A converter that handles miles but not قنطار is a converter
//! built for somebody else.
//!
//! Everything reduces to a base unit per dimension and converts through it. Direct pair tables
//! are quadratic and, worse, drift: one wrong entry gives a wrong answer in one direction only.

use rust_decimal::prelude::*;
use rust_decimal::Decimal;

use crate::{fold_digits, Answer, Tool};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    Length,
    Mass,
    Temperature,
    Area,
    Volume,
    Speed,
    Data,
}

struct Unit {
    /// Canonical English name. The stable identifier, used in `detail` and in tests.
    name: &'static str,
    /// Display names per interface language: `(ar, fr)`.
    ///
    /// The card said "2 qintar → kilogram" to an Arabic reader before this existed. A converter
    /// that answers an Arabic question in English has only done half the job — and qintar is
    /// precisely the unit whose Arabic name the reader already knows.
    names: (&'static str, &'static str),
    dimension: Dimension,
    /// How many base units one of these is. Base units: metre, kilogram, m², litre, km/h, byte.
    per_base: &'static str,
    /// Every spelling we accept, across ar / fr / en. Matched after folding.
    aliases: &'static [&'static str],
}

/// The table.
///
/// Arabic and French aliases are not an afterthought: `5 كلم بالميل` and `5 km en miles` are the
/// same question, and a converter that only understands the English one is not for this audience.
const UNITS: &[Unit] = &[
    // --- length -------------------------------------------------------------------------
    Unit {
        name: "metre",
        names: ("متر", "mètre"),
        dimension: Dimension::Length,
        per_base: "1",
        aliases: &["m", "metre", "meter", "metres", "meters", "متر"],
    },
    Unit {
        name: "kilometre",
        names: ("كيلومتر", "kilomètre"),
        dimension: Dimension::Length,
        per_base: "1000",
        aliases: &[
            "km",
            "kilometre",
            "kilometer",
            "kilometres",
            "kilometers",
            "كلم",
            "كيلومتر",
        ],
    },
    Unit {
        name: "centimetre",
        names: ("سنتيمتر", "centimètre"),
        dimension: Dimension::Length,
        per_base: "0.01",
        aliases: &["cm", "centimetre", "centimeter", "سم", "سنتيمتر"],
    },
    Unit {
        name: "millimetre",
        names: ("مليمتر", "millimètre"),
        dimension: Dimension::Length,
        per_base: "0.001",
        aliases: &["mm", "millimetre", "millimeter", "ملم"],
    },
    Unit {
        name: "mile",
        names: ("ميل", "mile"),
        dimension: Dimension::Length,
        per_base: "1609.344",
        aliases: &["mi", "mile", "miles", "ميل", "أميال"],
    },
    Unit {
        name: "foot",
        names: ("قدم", "pied"),
        dimension: Dimension::Length,
        per_base: "0.3048",
        aliases: &["ft", "foot", "feet", "pied", "pieds", "قدم"],
    },
    Unit {
        name: "inch",
        names: ("بوصة", "pouce"),
        dimension: Dimension::Length,
        per_base: "0.0254",
        aliases: &["in", "inch", "inches", "pouce", "بوصة", "إنش"],
    },
    // --- mass ---------------------------------------------------------------------------
    Unit {
        name: "kilogram",
        names: ("كيلوغرام", "kilogramme"),
        dimension: Dimension::Mass,
        per_base: "1",
        aliases: &[
            "kg",
            "kilo",
            "kilos",
            "kilogram",
            "kilogramme",
            "كلغ",
            "كيلو",
            "كيلوغرام",
        ],
    },
    Unit {
        name: "gram",
        names: ("غرام", "gramme"),
        dimension: Dimension::Mass,
        per_base: "0.001",
        aliases: &["g", "gram", "gramme", "grams", "grammes", "غرام", "جرام"],
    },
    Unit {
        name: "tonne",
        names: ("طن", "tonne"),
        dimension: Dimension::Mass,
        per_base: "1000",
        aliases: &["t", "tonne", "tonnes", "ton", "طن", "أطنان"],
    },
    Unit {
        name: "pound",
        names: ("رطل", "livre"),
        dimension: Dimension::Mass,
        per_base: "0.45359237",
        aliases: &["lb", "lbs", "pound", "pounds", "livre", "livres", "رطل"],
    },
    // The Algerian qintar is 100 kg, not the Ottoman or the imperial hundredweight. Produce and
    // grain are quoted in it constantly and no international converter knows it exists.
    Unit {
        name: "qintar",
        names: ("قنطار", "quintal"),
        dimension: Dimension::Mass,
        per_base: "100",
        aliases: &["qintar", "quintal", "quintaux", "qx", "قنطار", "قناطير"],
    },
    // --- temperature (handled specially; offsets, not ratios) -----------------------------
    Unit {
        name: "celsius",
        names: ("درجة مئوية", "°C"),
        dimension: Dimension::Temperature,
        per_base: "1",
        aliases: &["c", "°c", "celsius", "centigrade", "مئوية", "درجة"],
    },
    Unit {
        name: "fahrenheit",
        names: ("فهرنهايت", "°F"),
        dimension: Dimension::Temperature,
        per_base: "1",
        aliases: &["f", "°f", "fahrenheit", "فهرنهايت"],
    },
    Unit {
        name: "kelvin",
        names: ("كلفن", "kelvin"),
        dimension: Dimension::Temperature,
        per_base: "1",
        aliases: &["k", "kelvin", "كلفن"],
    },
    // --- area ---------------------------------------------------------------------------
    Unit {
        name: "square metre",
        names: ("متر مربع", "mètre carré"),
        dimension: Dimension::Area,
        per_base: "1",
        aliases: &["m2", "sqm", "متر مربع"],
    },
    Unit {
        name: "hectare",
        names: ("هكتار", "hectare"),
        dimension: Dimension::Area,
        per_base: "10000",
        aliases: &["ha", "hectare", "hectares", "هكتار"],
    },
    // The Algerian sa'a: a traditional land measure, 400 m². Still how rural land is described.
    Unit {
        name: "sa'a",
        names: ("ساعة", "saa"),
        dimension: Dimension::Area,
        per_base: "400",
        aliases: &["saa", "sa3a", "ساعة", "صاع"],
    },
    Unit {
        name: "square kilometre",
        names: ("كيلومتر مربع", "kilomètre carré"),
        dimension: Dimension::Area,
        per_base: "1000000",
        aliases: &["km2", "sqkm", "كلم مربع"],
    },
    // --- volume -------------------------------------------------------------------------
    Unit {
        name: "litre",
        names: ("لتر", "litre"),
        dimension: Dimension::Volume,
        per_base: "1",
        aliases: &["l", "litre", "liter", "litres", "liters", "لتر"],
    },
    Unit {
        name: "millilitre",
        names: ("مليلتر", "millilitre"),
        dimension: Dimension::Volume,
        per_base: "0.001",
        aliases: &["ml", "millilitre", "milliliter", "مل"],
    },
    Unit {
        name: "cubic metre",
        names: ("متر مكعب", "mètre cube"),
        dimension: Dimension::Volume,
        per_base: "1000",
        aliases: &["m3", "متر مكعب"],
    },
    // --- speed --------------------------------------------------------------------------
    Unit {
        name: "kilometre per hour",
        names: ("كلم/سا", "km/h"),
        dimension: Dimension::Speed,
        per_base: "1",
        aliases: &["kmh", "km/h", "kph", "كلم/س"],
    },
    Unit {
        name: "mile per hour",
        names: ("ميل/سا", "mph"),
        dimension: Dimension::Speed,
        per_base: "1.609344",
        aliases: &["mph", "mi/h"],
    },
    Unit {
        name: "metre per second",
        names: ("م/ث", "m/s"),
        dimension: Dimension::Speed,
        per_base: "3.6",
        aliases: &["ms", "m/s"],
    },
    // --- data ---------------------------------------------------------------------------
    Unit {
        name: "byte",
        names: ("بايت", "octet"),
        dimension: Dimension::Data,
        per_base: "1",
        aliases: &["b", "byte", "bytes"],
    },
    Unit {
        name: "kilobyte",
        names: ("كيلوبايت", "kilo-octet"),
        dimension: Dimension::Data,
        per_base: "1024",
        aliases: &["kb", "kilobyte"],
    },
    Unit {
        name: "megabyte",
        names: ("ميغابايت", "mégaoctet"),
        dimension: Dimension::Data,
        per_base: "1048576",
        aliases: &["mb", "megabyte", "mo"],
    },
    Unit {
        name: "gigabyte",
        names: ("غيغابايت", "gigaoctet"),
        dimension: Dimension::Data,
        per_base: "1073741824",
        aliases: &["gb", "gigabyte", "go"],
    },
];

impl Unit {
    /// The name to display in a given interface language.
    fn display(&self, lang: &str) -> &'static str {
        match lang {
            // Darija reads Arabic. Falling back to English would be a strictly worse guess.
            "ar" | "ary" => self.names.0,
            "fr" => self.names.1,
            _ => self.name,
        }
    }
}

/// Words meaning "to", across the languages people mix.
const TO: &[&str] = &["to", "in", "en", "into", "as", "الى", "إلى", "بال", "ب"];

pub struct UnitConverter;

impl Tool for UnitConverter {
    fn name(&self) -> &'static str {
        "unit-converter"
    }

    fn keyword(&self) -> &'static str {
        "convert"
    }

    fn answer(&self, query: &str) -> Option<Answer> {
        self.answer_in(query, "en")
    }

    /// Convert, rendering unit names in `lang`.
    fn answer_in(&self, query: &str, lang: &str) -> Option<Answer> {
        let folded = fold_digits(query).to_lowercase();
        let tokens: Vec<&str> = folded.split_whitespace().collect();
        if tokens.len() < 2 {
            return None;
        }

        // `<amount> <from> [to] <target>`, where the amount may be glued to its unit (`5km`).
        let (amount, rest) = split_amount(&tokens)?;
        let separator = rest.iter().position(|t| TO.contains(t));

        // Without a separator this is far more likely to be ordinary prose than a conversion:
        // `5 kilos de tomates` is not a request to convert anything.
        let split = separator?;
        let (from_tokens, to_tokens) = (&rest[..split], &rest[split + 1..]);

        let from = lookup(from_tokens)?;
        let to = lookup(to_tokens)?;
        if from.dimension != to.dimension {
            // Refusing is the point. A confident answer to "5 km in kilograms" is worse than none.
            return None;
        }

        let result = convert(amount, from, to)?;

        Some(Answer {
            tool: self.name(),
            // Structural and total: the whole string was consumed as a conversion. Higher than
            // the calculator's, which is what makes the converter win `5 km to miles`.
            confidence: 0.99,
            interpretation: format!(
                "{} {} → {}",
                trim(amount),
                from.display(lang),
                to.display(lang)
            ),
            value: format!("{} {}", trim(result), to.display(lang)),
            detail: Some(serde_json::json!({
                "amount": amount.to_string(),
                "from": from.name,
                "to": to.name,
                "result": result.to_string(),
                "dimension": format!("{:?}", from.dimension).to_lowercase(),
            })),
            as_of: None,
        })
    }
}

fn split_amount<'a>(tokens: &[&'a str]) -> Option<(Decimal, Vec<&'a str>)> {
    let first = tokens.first()?;

    if let Ok(value) = Decimal::from_str(&first.replace(',', "")) {
        return Some((value, tokens[1..].to_vec()));
    }

    // Glued: `5km`, `30°c`. Split at the first non-numeric character.
    let split = first.find(|c: char| !c.is_ascii_digit() && c != '.' && c != ',')?;
    let (number, unit) = first.split_at(split);
    let value = Decimal::from_str(&number.replace(',', "")).ok()?;
    let mut rest = vec![unit];
    rest.extend_from_slice(&tokens[1..]);
    Some((value, rest))
}

fn lookup(tokens: &[&str]) -> Option<&'static Unit> {
    if tokens.is_empty() {
        return None;
    }
    // Longest phrase first, so "متر مربع" is not read as "متر".
    let joined = tokens.join(" ");
    for candidate in [joined.as_str(), tokens.first()?] {
        let cleaned = candidate.trim_matches(|c: char| c == '.' || c == '?' || c == '!');
        if let Some(unit) = UNITS.iter().find(|u| u.aliases.contains(&cleaned)) {
            return Some(unit);
        }
    }
    None
}

fn convert(amount: Decimal, from: &Unit, to: &Unit) -> Option<Decimal> {
    if from.dimension == Dimension::Temperature {
        return convert_temperature(amount, from.name, to.name);
    }
    let from_ratio = Decimal::from_str(from.per_base).ok()?;
    let to_ratio = Decimal::from_str(to.per_base).ok()?;
    amount.checked_mul(from_ratio)?.checked_div(to_ratio)
}

/// Temperature converts through offsets, not ratios.
///
/// Handled separately because treating it as a ratio is the classic unit-converter bug: 30 °C
/// becomes 54 °F instead of 86 °F, and the answer looks plausible enough to go unnoticed.
fn convert_temperature(value: Decimal, from: &str, to: &str) -> Option<Decimal> {
    let thirty_two = Decimal::from(32);
    let nine_fifths = Decimal::from_str("1.8").ok()?;
    let kelvin_offset = Decimal::from_str("273.15").ok()?;

    let celsius = match from {
        "celsius" => value,
        "fahrenheit" => value.checked_sub(thirty_two)?.checked_div(nine_fifths)?,
        "kelvin" => value.checked_sub(kelvin_offset)?,
        _ => return None,
    };

    match to {
        "celsius" => Some(celsius),
        "fahrenheit" => celsius.checked_mul(nine_fifths)?.checked_add(thirty_two),
        "kelvin" => celsius.checked_add(kelvin_offset),
        _ => None,
    }
}

/// Round for display without pretending to precision we do not have.
fn trim(value: Decimal) -> String {
    let rounded = value.round_dp(6).normalize();
    rounded.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn convert_query(q: &str) -> Option<String> {
        UnitConverter.answer(q).map(|a| a.value)
    }

    #[test]
    fn length_converts() {
        assert_eq!(
            convert_query("5 km to miles").as_deref(),
            Some("3.106856 mile")
        );
        assert_eq!(convert_query("100 cm in m").as_deref(), Some("1 metre"));
    }

    #[test]
    fn temperature_uses_offsets_not_ratios() {
        // The classic converter bug: treating temperature as a ratio makes 30°C into 54°F, which
        // is wrong and plausible enough to go unnoticed.
        assert_eq!(convert_query("30 c to f").as_deref(), Some("86 fahrenheit"));
        assert_eq!(
            convert_query("100 c to f").as_deref(),
            Some("212 fahrenheit")
        );
        assert_eq!(convert_query("0 c to k").as_deref(), Some("273.15 kelvin"));
        assert_eq!(convert_query("32 f to c").as_deref(), Some("0 celsius"));
    }

    #[test]
    fn algerian_units_are_first_class() {
        // No international converter carries these, and they are how produce and land are
        // quoted here every day.
        assert_eq!(
            convert_query("2 qintar to kg").as_deref(),
            Some("200 kilogram")
        );
        assert_eq!(
            convert_query("2 قنطار الى كلغ").as_deref(),
            Some("200 kilogram")
        );
        assert_eq!(
            convert_query("1 saa to m2").as_deref(),
            Some("400 square metre")
        );
    }

    #[test]
    fn french_and_arabic_phrasings_work() {
        // `5 km en miles` and `5 كلم الى ميل` are the same question.
        assert!(convert_query("5 km en miles").is_some());
        assert!(convert_query("5 كلم الى ميل").is_some());
        assert!(convert_query("10 kilos en livres").is_some());
    }

    #[test]
    fn a_glued_amount_is_understood() {
        assert_eq!(convert_query("5km to m").as_deref(), Some("5000 metre"));
    }

    #[test]
    fn mismatched_dimensions_answer_nothing() {
        // A confident answer to "5 km in kilograms" is worse than no answer.
        assert_eq!(convert_query("5 km to kg"), None);
        assert_eq!(convert_query("30 c to litres"), None);
    }

    #[test]
    fn prose_that_merely_contains_a_unit_is_not_a_conversion() {
        // The precision case. Without requiring an explicit "to", every shopping query becomes a
        // unit conversion card.
        assert_eq!(convert_query("5 kilos de tomates"), None);
        assert_eq!(convert_query("prix 5 km"), None);
        assert_eq!(convert_query("100 m sprint record"), None);
    }

    #[test]
    fn unknown_units_answer_nothing() {
        assert_eq!(convert_query("5 furlongs to smoots"), None);
        assert_eq!(convert_query("5 to 10"), None);
    }

    #[test]
    fn the_interpretation_names_both_units() {
        // So a misreading is visible: `5 m` read as metres when the user meant miles.
        let answer = UnitConverter.answer("5 km to miles").unwrap();
        assert!(answer.interpretation.contains("kilometre"));
        assert!(answer.interpretation.contains("mile"));
        assert_eq!(answer.as_of, None, "a conversion ratio is timeless");
    }

    #[test]
    fn every_alias_is_unique_across_the_table() {
        // A duplicate alias makes lookup order decide the answer, which is a silent wrong result
        // rather than a compile error.
        let mut seen = std::collections::HashMap::new();
        for unit in UNITS {
            for alias in unit.aliases {
                if let Some(other) = seen.insert(*alias, unit.name) {
                    panic!(
                        "alias {alias:?} is claimed by both {other} and {}",
                        unit.name
                    );
                }
            }
        }
    }

    #[test]
    fn every_ratio_parses() {
        for unit in UNITS {
            assert!(
                Decimal::from_str(unit.per_base).is_ok(),
                "{} has an unparseable ratio {:?}",
                unit.name,
                unit.per_base
            );
        }
    }
}

#[cfg(test)]
mod locale_tests {
    use super::*;

    /// Every unit must have an Arabic and a French name.
    ///
    /// The table is the kind of list people append to, and an entry added with only its English
    /// name degrades silently: the converter still answers, just in the wrong language, and only
    /// for the one unit nobody tested. Checking the whole table mechanically is the only way this
    /// stays true as it grows.
    #[test]
    fn every_unit_is_named_in_arabic_and_french() {
        for u in UNITS {
            assert!(
                !u.names.0.trim().is_empty(),
                "{} has no Arabic name",
                u.name
            );
            assert!(
                !u.names.1.trim().is_empty(),
                "{} has no French name",
                u.name
            );
            assert!(
                u.names
                    .0
                    .chars()
                    .any(|c| ('\u{0600}'..='\u{06FF}').contains(&c)),
                "{}'s Arabic name {:?} is not in Arabic script — an English word in the ar slot \
                 passes a non-empty check while still answering the wrong language",
                u.name,
                u.names.0
            );
        }
    }

    /// Darija reads Arabic, never English.
    #[test]
    fn darija_falls_back_to_arabic() {
        for u in UNITS {
            assert_eq!(
                u.display("ary"),
                u.display("ar"),
                "{} differs between ar and ary",
                u.name
            );
        }
    }

    /// The unit whose Arabic name the reader is most likely to already know.
    #[test]
    fn qintar_answers_in_the_language_it_was_asked_in() {
        let q = UNITS
            .iter()
            .find(|u| u.name == "qintar")
            .expect("qintar is the reason this converter exists");
        assert_ne!(q.display("ar"), q.name);
        assert_ne!(q.display("fr"), q.name);
        assert_eq!(q.display("en"), q.name);
    }
}
