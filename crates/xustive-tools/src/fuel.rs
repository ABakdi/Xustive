//! Fuel prices.
//!
//! # Why this is a table and not a fetcher
//!
//! Algerian fuel prices are **administered**: set nationally by the Autorité de Régulation des
//! Hydrocarbures, uniform across all 58 wilayas, and unchanged for years at a time. The current
//! prices took effect on 1 January 2026; the previous ones had stood since 2020.
//!
//! There is nothing to poll. The ARH publishes no feed, Naftal publishes no feed, and the January
//! 2026 change was applied at midnight **with no announcement from either of them** — the country
//! found out at the pump. A fetcher would have nothing to fetch.
//!
//! So the values are compiled in, with the authority and the effective date attached, and the card
//! shows both. A price with a date beside it is something a driver can sanity-check in a second;
//! the same number alone is a claim they have to take on faith.
//!
//! # The failure this design has, and what is done about it
//!
//! Prices change silently, so this table will eventually be wrong and nothing will signal it. That
//! is a real weakness and it cannot be engineered away without a source that does not exist.
//!
//! What it can be converted into is a **loud** failure instead of a silent one:
//! [`tests::the_table_is_due_for_review`] fails once `REVIEW_BY` passes, so the build breaks and
//! someone re-checks the prices against the ARH. A broken build is a cheap problem. A search engine
//! confidently quoting a price that changed eight months ago is not.

use crate::{fold_digits, Answer, Tool};

/// When someone must re-verify this table against the ARH.
///
/// Public because it is a property of the data, not a test fixture: anything reporting on this
/// table — an admin page, an operator check — needs to know when it stops being trustworthy.
///
/// Set past the point where a Loi de Finances change would have taken effect and been reported.
/// Deliberately a hard test failure rather than a warning: a warning in CI output is a warning
/// nobody reads, and the whole risk here is that being wrong produces no symptom.
pub const REVIEW_BY: &str = "2027-03-01";

/// The authority that sets these prices, shown on the card.
const AUTHORITY: &str = "ARH";

/// When the current prices took effect.
const EFFECTIVE: &str = "2026-01-01";

/// One administered price.
pub struct Fuel {
    pub id: &'static str,
    pub name_ar: &'static str,
    pub name_fr: &'static str,
    pub name_en: &'static str,
    /// Dinars per litre.
    pub price: f64,
    /// What it was before `EFFECTIVE`, so the card can show the change.
    pub previous: f64,
}

/// The table.
///
/// Petrol is a single grade. Algeria completed its phase-out of leaded petrol in 2021, so the
/// "super" and "normale" grades an older reference would list no longer exist at the pump, and
/// offering them would be answering a question about a product nobody sells.
pub const PRICES: &[Fuel] = &[
    Fuel {
        id: "essence",
        name_ar: "بنزين",
        name_fr: "Essence sans plomb",
        name_en: "Petrol (unleaded)",
        price: 47.00,
        previous: 45.62,
    },
    Fuel {
        id: "gasoil",
        name_ar: "مازوت",
        name_fr: "Gasoil",
        name_en: "Diesel",
        price: 31.00,
        previous: 29.01,
    },
    Fuel {
        id: "gpl",
        name_ar: "سيرغاز",
        name_fr: "GPL-c (sirghaz)",
        name_en: "LPG (autogas)",
        price: 12.00,
        previous: 9.00,
    },
];

pub struct FuelTool;

/// Words that mean "what does fuel cost", across the four languages the engine serves.
const TRIGGERS: &[&str] = &[
    "carburant",
    "essence",
    "gasoil",
    "gazoil",
    "diesel",
    "mazout",
    "sirghaz",
    "gpl",
    "petrol",
    "fuel price",
    "وقود",
    "بنزين",
    "مازوت",
    "سيرغاز",
    "المحروقات",
];

/// Words that turn a mention of fuel into a question about its price.
///
/// Required, because `essence` on its own is as likely to be part of `station essence près de moi`
/// as a price question, and a price card above a search for a petrol station is an interruption.
const PRICE_WORDS: &[&str] = &[
    "prix",
    "cout",
    "coût",
    "combien",
    "tarif",
    "price",
    "cost",
    "how much",
    "سعر",
    "أسعار",
    "بكم",
    "تسعيرة",
    "chhal",
    "ch7al",
    "kadech",
    "9adech",
];

impl Tool for FuelTool {
    fn name(&self) -> &'static str {
        "fuel"
    }

    fn keyword(&self) -> &'static str {
        "fuel"
    }

    fn answer(&self, query: &str) -> Option<Answer> {
        self.answer_in(query, "fr")
    }

    fn answer_in(&self, query: &str, lang: &str) -> Option<Answer> {
        let q = fold_digits(query).to_lowercase();

        let matched: Vec<&Fuel> = PRICES.iter().filter(|f| mentions(&q, f)).collect();
        let mentions_fuel = !matched.is_empty() || TRIGGERS.iter().any(|t| q.contains(t));
        if !mentions_fuel {
            return None;
        }

        // A bare grade name is not a price question. `station essence` and `moteur diesel` both
        // mention a fuel and neither wants this card.
        if !PRICE_WORDS.iter().any(|w| q.contains(w)) {
            return None;
        }

        // Naming a specific grade answers about that one; asking generally lists all three.
        let shown: Vec<&Fuel> = if matched.is_empty() {
            PRICES.iter().collect()
        } else {
            matched
        };

        // Named in the reader's language. "Essence sans plomb" answered to someone who asked
        // "سعر البنزين" is answering a different person's question.
        let value = shown
            .iter()
            .map(|f| format!("{} {:.2} {}", f.name(lang), f.price, unit(lang)))
            .collect::<Vec<_>>()
            .join(" · ");

        let detail: Vec<serde_json::Value> = shown
            .iter()
            .map(|f| {
                serde_json::json!({
                    "id": f.id,
                    "name_ar": f.name_ar,
                    "name_fr": f.name_fr,
                    "name_en": f.name_en,
                    "price": f.price,
                    "previous": f.previous,
                    "unit": "DZD/L",
                })
            })
            .collect();

        Some(Answer {
            tool: "fuel",
            confidence: 0.9,
            // The interpretation carries the date because these are administered prices that
            // change without notice. A driver who knows they changed last week can tell at a
            // glance that this table has not caught up.
            interpretation: format!("{AUTHORITY} · depuis {EFFECTIVE}"),
            value,
            detail: Some(serde_json::json!({
                "fuels": detail,
                "authority": AUTHORITY,
                "effective": EFFECTIVE,
                // Says outright that this is not a live quote. The card renders it as a note.
                "administered": true,
            })),
            // No `as_of`: that field means "when this was measured", and an administered price is
            // not measured. Its date is `effective`, which is a different claim and lives in
            // `detail` so it cannot be rendered as a freshness stamp.
            as_of: None,
        })
    }
}

impl Fuel {
    /// The grade's name in `lang`, defaulting to French.
    ///
    /// Darija reads Arabic script, so `ary` takes the Arabic name rather than falling through to
    /// French — the two are not interchangeable for a reader.
    pub fn name(&self, lang: &str) -> &'static str {
        match lang {
            "ar" | "ary" => self.name_ar,
            "en" => self.name_en,
            _ => self.name_fr,
        }
    }
}

/// The per-litre unit, in script the reader uses.
fn unit(lang: &str) -> &'static str {
    match lang {
        "ar" | "ary" => "دج/ل",
        _ => "DA/L",
    }
}

fn mentions(q: &str, f: &Fuel) -> bool {
    match f.id {
        "essence" => q.contains("essence") || q.contains("بنزين") || q.contains("petrol"),
        "gasoil" => {
            q.contains("gasoil")
                || q.contains("gazoil")
                || q.contains("diesel")
                || q.contains("مازوت")
        }
        "gpl" => q.contains("gpl") || q.contains("sirghaz") || q.contains("سيرغاز"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(q: &str) -> Option<String> {
        FuelTool.answer(q).map(|a| a.value)
    }

    #[test]
    fn the_table_is_due_for_review() {
        // Not a lint. Algerian fuel prices are administered and change with no announcement — the
        // January 2026 rise was applied at midnight and neither the ARH nor Naftal said anything.
        // Being wrong here therefore produces no symptom at all, so the only defence is a date
        // that forces a human to look again.
        //
        // If this fails: check the current prices against the ARH or a Loi de Finances reference,
        // update PRICES and EFFECTIVE if they moved, and push REVIEW_BY forward a year.
        let review_by = parse_date(REVIEW_BY);
        let now = xustive_core::now_unix();
        assert!(
            now < review_by,
            "fuel prices are past their review date ({REVIEW_BY}). Re-verify PRICES against the \
             ARH, update EFFECTIVE if they changed, then move REVIEW_BY forward. Do not simply \
             move the date: the table is probably wrong."
        );
    }

    #[test]
    fn the_review_date_is_after_the_effective_date() {
        // A review date already in the past when the table was written would make the guard above
        // fire immediately and train whoever hits it to just bump the constant.
        assert!(parse_date(REVIEW_BY) > parse_date(EFFECTIVE));
    }

    #[test]
    fn a_price_question_is_answered() {
        let out = value("prix carburant").expect("should answer");
        // A general question lists every grade, since a driver comparing them should not have to
        // ask three times.
        assert!(out.contains("47.00"), "got {out}");
        assert!(out.contains("31.00"), "got {out}");
        assert!(out.contains("12.00"), "got {out}");
    }

    #[test]
    fn naming_a_grade_answers_about_that_grade_only() {
        let out = value("prix gasoil").expect("should answer");
        assert!(out.contains("31.00"), "got {out}");
        assert!(!out.contains("47.00"), "should not list petrol too: {out}");
    }

    #[test]
    fn grades_are_named_in_the_readers_language() {
        // Answering "Essence sans plomb" to someone who asked in Arabic is answering a different
        // person's question.
        let ar = FuelTool.answer_in("سعر البنزين", "ar").expect("answer");
        assert!(ar.value.contains("بنزين"), "got {}", ar.value);
        assert!(
            ar.value.contains("دج/ل"),
            "unit not localised: {}",
            ar.value
        );

        let en = FuelTool.answer_in("diesel price", "en").expect("answer");
        assert!(en.value.contains("Diesel"), "got {}", en.value);

        // Darija reads Arabic script, so it takes the Arabic name rather than French.
        let ary = FuelTool.answer_in("chhal essence", "ary").expect("answer");
        assert!(ary.value.contains("بنزين"), "got {}", ary.value);
    }

    #[test]
    fn the_question_is_recognised_in_four_languages() {
        for q in [
            "prix du gasoil",
            "سعر البنزين",
            "fuel price algeria",
            "diesel cost",
            "chhal essence", // Darija, Latin script
        ] {
            assert!(FuelTool.answer(q).is_some(), "{q:?} should answer");
        }
    }

    #[test]
    fn mentioning_fuel_without_asking_a_price_answers_nothing() {
        // The precision that decides whether a tool feels helpful or intrusive. Each of these
        // mentions a fuel and none of them wants a price card.
        for q in [
            "station essence près de moi",
            "moteur diesel",
            "naftal recrutement",
            "pompe a essence alger",
            "بنزين ناقص",
        ] {
            assert!(
                FuelTool.answer(q).is_none(),
                "{q:?} should not show a price card, got {:?}",
                value(q)
            );
        }
    }

    #[test]
    fn the_answer_carries_its_authority_and_date() {
        // These are administered prices that change without notice, so a number with no date
        // beside it is a claim the reader has to take on faith.
        let answer = FuelTool.answer("prix carburant").expect("answer");
        assert!(answer.interpretation.contains(AUTHORITY));
        assert!(answer.interpretation.contains(EFFECTIVE));
        let detail = answer.detail.as_ref().unwrap();
        assert_eq!(detail["administered"], true);
        assert_eq!(detail["effective"], EFFECTIVE);
    }

    #[test]
    fn an_administered_price_carries_no_as_of() {
        // `as_of` means "when this was measured". An administered price is not measured, and
        // putting its effective date there would render as a freshness stamp — implying we
        // checked today, which we did not.
        assert!(FuelTool.answer("prix carburant").unwrap().as_of.is_none());
    }

    #[test]
    fn every_entry_records_what_it_replaced() {
        // The previous price is what lets the card show a change rather than a bare number, and
        // it is the only thing in the table that dates it independently of EFFECTIVE.
        for f in PRICES {
            assert!(
                f.price > 0.0 && f.previous > 0.0,
                "{} has a zero price",
                f.id
            );
            assert!(f.price != f.previous, "{} did not change", f.id);
            // Sanity bound. An administered price in dinars per litre is tens, not thousands —
            // a units slip is the most likely way this table goes wrong on an edit.
            assert!(f.price < 500.0, "{} looks like the wrong unit", f.id);
        }
    }

    /// `YYYY-MM-DD` to a Unix timestamp. Days-only precision is all these dates carry.
    fn parse_date(s: &str) -> i64 {
        let parts: Vec<i64> = s.split('-').map(|p| p.parse().unwrap()).collect();
        let (y, m, d) = (parts[0], parts[1], parts[2]);
        // Days since the Unix epoch, via the civil-from-days algorithm.
        let y = if m <= 2 { y - 1 } else { y };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        (era * 146_097 + doe - 719_468) * 86_400
    }

    #[test]
    fn the_date_parser_is_correct() {
        // The review guard depends on this, so it is checked against known epochs rather than
        // trusted — a parser that is off by a year would disable the guard silently.
        assert_eq!(parse_date("1970-01-01"), 0);
        assert_eq!(parse_date("2000-01-01"), 946_684_800);
        assert_eq!(parse_date("2026-01-01"), 1_767_225_600);
    }
}
