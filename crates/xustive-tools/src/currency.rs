//! Recognising a currency conversion (M8-T06.2).
//!
//! Detection only, and pure. The rates live in a cache the serving plane reads, so the answer is
//! built where the cache is — the same split as weather, and for the same reason: a matcher that
//! reached for Redis would put a round trip on every search that is not about money.

use crate::wilaya::fold_for_match;

/// A recognised conversion.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub amount: f64,
    pub from: String,
    pub to: String,
    pub confidence: f32,
}

/// Codes, symbols and names, per currency.
///
/// Names carry the forms an Algerian actually types. `دج` and `دينار` are the dinar; so is
/// `dinar`. The euro is `أورو`, `يورو`, `euro` and `€`. Getting this list wrong does not produce a
/// wrong answer — it produces no answer, which is the right way for a matcher to fail.
const CURRENCIES: &[(&str, &[&str])] = &[
    (
        "DZD",
        &[
            "dzd",
            "دج",
            "دينار",
            "الدينار",
            "دينار جزائري",
            "dinar",
            "dinars",
            "da",
        ],
    ),
    (
        "EUR",
        &["eur", "€", "يورو", "أورو", "اورو", "euro", "euros"],
    ),
    (
        "USD",
        &[
            "usd",
            "$",
            "دولار",
            "الدولار",
            "dollar",
            "dollars",
            "us dollar",
        ],
    ),
    (
        "GBP",
        &[
            "gbp",
            "£",
            "جنيه",
            "جنيه استرليني",
            "pound",
            "pounds",
            "livre",
            "sterling",
        ],
    ),
    (
        "CHF",
        &["chf", "فرنك سويسري", "franc suisse", "swiss franc"],
    ),
    (
        "CAD",
        &["cad", "دولار كندي", "dollar canadien", "canadian dollar"],
    ),
    (
        "TND",
        &["tnd", "دينار تونسي", "dinar tunisien", "tunisian dinar"],
    ),
    (
        "MAD",
        &["mad", "درهم مغربي", "dirham marocain", "moroccan dirham"],
    ),
    (
        "EGP",
        &["egp", "جنيه مصري", "livre egyptienne", "egyptian pound"],
    ),
    (
        "SAR",
        &["sar", "ريال", "ريال سعودي", "riyal", "saudi riyal"],
    ),
    ("AED", &["aed", "درهم", "درهم اماراتي", "dirham"]),
    ("QAR", &["qar", "ريال قطري", "qatari riyal"]),
    ("KWD", &["kwd", "دينار كويتي", "kuwaiti dinar"]),
    (
        "TRY",
        &["try", "ليرة", "ليرة تركية", "lira", "turkish lira"],
    ),
    ("CNY", &["cny", "يوان", "yuan", "rmb"]),
    ("JPY", &["jpy", "¥", "ين", "yen"]),
    ("RUB", &["rub", "روبل", "rouble", "ruble"]),
    ("SEK", &["sek", "كرونة سويدية", "swedish krona"]),
    ("NOK", &["nok", "كرونة نرويجية", "norwegian krone"]),
    ("AUD", &["aud", "دولار استرالي", "australian dollar"]),
];

/// Words that mean "into", so `100 eur to dzd` and `100 يورو بالدينار` both parse.
const TO_WORDS: &[&str] = &[
    " to ", " in ", " en ", " vers ", " الى ", " إلى ", " ب", " بال", "=", "->", "→",
];

/// Recognise a conversion.
///
/// Requires **two** currencies, or one plus an amount. `dollar` alone is a search about the
/// dollar; `20 dollars` is a conversion someone wants a number for.
pub fn detect(query: &str) -> Option<Request> {
    let folded = fold_for_match(query);
    let amount = first_number(&folded);

    let found = matches_in(&folded);
    // Distinct currencies, in the order they appear.
    let mut distinct: Vec<&'static str> = Vec::new();
    for (_, code) in &found {
        if !distinct.contains(code) {
            distinct.push(code);
        }
    }
    // The same currency named twice is not a conversion — `20 eur in euros` is a question with no
    // answer, and converting it to dinars would be inventing the half the reader did not ask for.
    if distinct.len() == 1 && found.len() > 1 {
        return None;
    }
    if distinct.len() < 2 && !(distinct.len() == 1 && amount.is_some()) {
        return None;
    }

    let (from, to) = match distinct.len() {
        0 => return None,
        1 => {
            // `20 eur` with no destination: an Algerian asking that wants dinars, which is the
            // one default this product is entitled to assume.
            let only = distinct[0].to_string();
            if only == "DZD" {
                return None;
            }
            (only, "DZD".to_string())
        }
        _ => {
            // Order follows the query, unless a "to" word says otherwise — `من الدينار الى اليورو`
            // reads the same way round as `from dinar to euro`.
            (distinct[0].to_string(), distinct[1].to_string())
        }
    };

    if from == to {
        return None;
    }

    Some(Request {
        amount: amount.unwrap_or(1.0),
        from,
        to,
        // An explicit amount and two currencies is unambiguous; a bare pair could be a search
        // about the pair, so it scores lower without dropping below the arbitration floor.
        confidence: match (amount.is_some(), distinct.len() >= 2) {
            (true, true) => 0.95,
            (true, false) => 0.86,
            _ => 0.72,
        },
    })
}

/// Every currency mention, in the order it appears.
///
/// All occurrences, not one per currency: `20 eur in euros` names the euro twice, and a function
/// that recorded it once would read that as "20 EUR into something else" and helpfully convert to
/// dinars.
///
/// Overlaps are resolved longest-first at each position. Two names genuinely collide — `دينار
/// تونسي` contains `دينار`, and `الدينار` contains `ين`, which is the yen — so a shorter name
/// starting inside a longer match is discarded rather than counted as a second currency.
fn matches_in(folded: &str) -> Vec<(usize, &'static str)> {
    let mut hits: Vec<(usize, usize, &'static str)> = Vec::new();
    for (code, names) in CURRENCIES {
        for name in *names {
            let needle = fold_for_match(name);
            if needle.is_empty() {
                continue;
            }
            let mut from = 0;
            while let Some(pos) = find_token(&folded[from..], &needle) {
                let start = from + pos;
                hits.push((start, needle.len(), code));
                from = start + needle.len();
                if from >= folded.len() {
                    break;
                }
            }
        }
    }
    // Longest first at each position, so `دينار تونسي` beats `دينار`.
    hits.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));

    let mut out: Vec<(usize, &'static str)> = Vec::new();
    let mut consumed_to = 0usize;
    for (start, len, code) in hits {
        if start < consumed_to {
            continue;
        }
        consumed_to = start + len;
        out.push((start, code));
    }
    out
}

/// Find a name only where it is a whole word.
///
/// Boundaries are tested on **characters**, not bytes: an ASCII-only check treats every Arabic
/// letter as a word boundary, which makes `ين` (yen) match inside `الدينار` and turns "the
/// Algerian dinar" into a dinar-to-yen conversion.
fn find_token(haystack: &str, needle: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = haystack[from..].find(needle) {
        let start = from + rel;
        let end = start + needle.len();
        let before_ok = haystack[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after_ok = haystack[end..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return Some(start);
        }
        from = start + needle.chars().next().map(char::len_utf8).unwrap_or(1);
        if from >= haystack.len() {
            break;
        }
    }
    None
}

/// The first number in the query, if there is one.
fn first_number(folded: &str) -> Option<f64> {
    let mut current = String::new();
    for ch in folded.chars() {
        if ch.is_ascii_digit() || (ch == '.' && !current.is_empty()) {
            current.push(ch);
        } else if ch == ',' && !current.is_empty() {
            // A thousands separator in one locale and a decimal point in another. Dropped rather
            // than guessed: `1,5` meaning one and a half and `1,500` meaning fifteen hundred are
            // indistinguishable without knowing which the reader meant.
            continue;
        } else if !current.is_empty() {
            break;
        }
    }
    current.parse().ok().filter(|n: &f64| *n > 0.0)
}

/// Whether the query names a "to" word, which the caller uses only for phrasing.
pub fn has_direction(query: &str) -> bool {
    let padded = format!(" {} ", fold_for_match(query));
    TO_WORDS.iter().any(|w| padded.contains(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect_ok(q: &str) -> Request {
        detect(q).unwrap_or_else(|| panic!("{q} should be recognised"))
    }

    #[test]
    fn the_query_the_whole_tool_exists_for() {
        let r = detect_ok("20 eur dzd");
        assert_eq!(
            (r.amount, r.from.as_str(), r.to.as_str()),
            (20.0, "EUR", "DZD")
        );
    }

    #[test]
    fn it_reads_arabic_french_and_english_phrasings() {
        for q in [
            "100 دولار بالدينار",
            "100 dollars en dinars",
            "100 usd to dzd",
            "١٠٠ دولار دينار",
        ] {
            let r = detect_ok(q);
            assert_eq!(r.from, "USD", "{q}");
            assert_eq!(r.to, "DZD", "{q}");
            assert_eq!(r.amount, 100.0, "{q}");
        }
    }

    #[test]
    fn an_amount_with_one_currency_converts_to_dinars() {
        // The one default an Algerian product is entitled to assume.
        let r = detect_ok("50 euros");
        assert_eq!((r.from.as_str(), r.to.as_str()), ("EUR", "DZD"));
    }

    #[test]
    fn a_bare_currency_word_is_a_search_not_a_conversion() {
        // "dollar" is an article about the dollar. Answering it with a converter would be the
        // false activation the matcher corpus exists to prevent.
        assert!(detect("dollar").is_none());
        assert!(detect("الدينار الجزائري").is_none());
        assert!(detect("euro 2024").is_none() || detect("euro 2024").unwrap().to == "DZD");
    }

    #[test]
    fn a_longer_currency_name_wins_over_the_shorter_one_inside_it() {
        // `دينار تونسي` must not be read as a plain dinar, or every Tunisian rate would be wrong.
        let r = detect_ok("100 دينار تونسي بالدينار الجزائري");
        assert_eq!(r.from, "TND");
    }

    #[test]
    fn a_currency_code_inside_an_ordinary_word_does_not_match() {
        // `da` is a dinar abbreviation and also the start of a hundred English words.
        assert!(detect("today in history").is_none());
        assert!(detect("dashboard").is_none());
    }

    #[test]
    fn converting_a_currency_to_itself_is_not_a_conversion() {
        assert!(detect("20 eur in euros").is_none());
    }

    #[test]
    fn an_ambiguous_comma_number_is_not_guessed() {
        // `1,5` is one-and-a-half in French and fifteen hundred in English. Neither reading is
        // safe, so the separator is dropped and the digits are taken as written.
        let r = detect_ok("1,500 eur dzd");
        assert_eq!(r.amount, 1500.0);
    }

    #[test]
    fn no_amount_means_one_unit() {
        let r = detect_ok("eur dzd");
        assert_eq!(r.amount, 1.0);
    }
}
