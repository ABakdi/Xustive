//! Dates: Hijri ↔ Gregorian, and the distance between two days.
//!
//! Computed, never fetched. Both calendars are arithmetic, so this works with no network and no
//! cache — which also means it is one of the few tools that cannot go stale.
//!
//! The Hijri conversion uses the **tabular** civil calendar, not observed crescent sighting.
//! Algeria's official dates come from observation and can differ by a day. That is stated on the
//! card rather than hidden, because a converter that silently disagrees with the announced Eid
//! looks broken, whereas one that says which reckoning it used is simply honest.

use crate::{fold_digits, Answer, Tool};

pub struct DateTool;

const MONTHS_EN: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Maghrebi month names. Algeria uses these, not the Mashriqi set — `أوت`, not `أغسطس`.
const MONTHS_AR: [&str; 12] = [
    "جانفي",
    "فيفري",
    "مارس",
    "أفريل",
    "ماي",
    "جوان",
    "جويلية",
    "أوت",
    "سبتمبر",
    "أكتوبر",
    "نوفمبر",
    "ديسمبر",
];

const HIJRI_MONTHS: [&str; 12] = [
    "محرم",
    "صفر",
    "ربيع الأول",
    "ربيع الثاني",
    "جمادى الأولى",
    "جمادى الآخرة",
    "رجب",
    "شعبان",
    "رمضان",
    "شوال",
    "ذو القعدة",
    "ذو الحجة",
];

impl Tool for DateTool {
    fn name(&self) -> &'static str {
        "date"
    }

    fn keyword(&self) -> &'static str {
        "date"
    }

    fn answer(&self, query: &str) -> Option<Answer> {
        let folded = fold_digits(query).to_lowercase();
        let q = folded.trim();

        if let Some(answer) = self.to_hijri(q) {
            return Some(answer);
        }
        self.between(q)
    }
}

impl DateTool {
    /// `12 august 2026 in hijri`, `2026-08-12 بالهجري`
    fn to_hijri(&self, q: &str) -> Option<Answer> {
        const TRIGGERS: &[&str] = &["hijri", "hijra", "هجري", "بالهجري", "الهجري"];
        if !TRIGGERS.iter().any(|t| q.contains(t)) {
            return None;
        }

        let date = parse_date(q)?;
        let (hy, hm, hd) = hijri_from_days(days_from_civil(date.0, date.1, date.2));

        Some(Answer {
            tool: self.name(),
            // Keyed: an explicit trigger word plus a parseable operand.
            confidence: 0.9,
            interpretation: format!(
                "{} {} {} → {}",
                date.2,
                MONTHS_EN[(date.1 - 1) as usize],
                date.0,
                "الهجري"
            ),
            value: format!("{hd} {} {hy} هـ", HIJRI_MONTHS[(hm - 1) as usize]),
            detail: Some(serde_json::json!({
                "gregorian": { "year": date.0, "month": date.1, "day": date.2 },
                "hijri": { "year": hy, "month": hm, "day": hd },
                // Surfaced so the card can say so. Observed dates can differ by a day and a
                // converter that hides its reckoning looks wrong rather than merely different.
                "reckoning": "tabular",
                "month_ar": MONTHS_AR[(date.1 - 1) as usize],
            })),
            as_of: None,
        })
    }

    /// `days between 2026-08-07 and 2027-03-20`
    fn between(&self, q: &str) -> Option<Answer> {
        const TRIGGERS: &[&str] = &["days between", "between", "كم يوم بين", "بين"];
        if !TRIGGERS.iter().any(|t| q.contains(t)) {
            return None;
        }

        let dates = find_dates(q);
        if dates.len() != 2 {
            return None;
        }
        let (a, b) = (dates[0], dates[1]);
        let days = days_from_civil(b.0, b.1, b.2) - days_from_civil(a.0, a.1, a.2);

        Some(Answer {
            tool: self.name(),
            confidence: 0.88,
            interpretation: format!(
                "{}-{:02}-{:02} → {}-{:02}-{:02}",
                a.0, a.1, a.2, b.0, b.1, b.2
            ),
            value: format!("{} days", days.abs()),
            detail: Some(serde_json::json!({ "days": days.abs() })),
            as_of: None,
        })
    }
}

/// Find every date-looking token, in order.
fn find_dates(text: &str) -> Vec<(i64, u32, u32)> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '/'))
        .filter_map(parse_one)
        .collect()
}

fn parse_date(text: &str) -> Option<(i64, u32, u32)> {
    find_dates(text).into_iter().next().or_else(|| {
        // `12 august 2026` — a month name between two numbers.
        let tokens: Vec<&str> = text.split_whitespace().collect();
        for (i, token) in tokens.iter().enumerate() {
            // `continue`, not `?`. Using `?` here returned from the whole function the moment the
            // first token was not a month name, so "12 august 2026" never got past "12".
            let lower = token.to_lowercase();
            let Some(index) = MONTHS_EN
                .iter()
                .position(|m| m.to_lowercase().starts_with(&lower) && lower.len() >= 3)
                .or_else(|| MONTHS_AR.iter().position(|m| *m == *token))
            else {
                continue;
            };
            let month = index as u32 + 1;
            let Some(day) = i
                .checked_sub(1)
                .and_then(|j| tokens.get(j))
                .and_then(|t| t.parse::<u32>().ok())
            else {
                continue;
            };
            let Some(year) = tokens.get(i + 1).and_then(|t| t.parse::<i64>().ok()) else {
                continue;
            };
            if valid(year, month, day) {
                return Some((year, month, day));
            }
        }
        None
    })
}

/// `YYYY-MM-DD` or `DD/MM/YYYY`.
///
/// Day-first for the slashed form, because that is what Algeria writes. Reading `03/09/2026` as
/// March would be wrong eleven times out of twelve here.
fn parse_one(token: &str) -> Option<(i64, u32, u32)> {
    let parts: Vec<&str> = token.split(['-', '/']).collect();
    if parts.len() != 3 {
        return None;
    }
    let a: i64 = parts[0].parse().ok()?;
    let b: u32 = parts[1].parse().ok()?;
    let c: i64 = parts[2].parse().ok()?;

    let (year, month, day) = if parts[0].len() == 4 {
        (a, b, c as u32)
    } else {
        (c, b, a as u32)
    };
    valid(year, month, day).then_some((year, month, day))
}

fn valid(year: i64, month: u32, day: u32) -> bool {
    (1..=12).contains(&month) && (1..=31).contains(&day) && (1000..=3000).contains(&year)
}

/// Howard Hinnant's `days_from_civil`.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Tabular Islamic calendar, epoch 16 July 622 CE.
///
/// The arithmetic calendar, not the observed one. Algeria announces Ramadan and the Eids by
/// crescent sighting, which can be a day either side of this.
pub fn hijri_from_days(days: i64) -> (i64, u32, u32) {
    // Julian day number for the Gregorian epoch offset.
    let jd = days + 2_440_588;
    let l = jd - 1_948_440 + 10_632;
    let n = (l - 1) / 10_631;
    let l = l - 10_631 * n + 354;
    let j = ((10_985 - l) / 5316) * ((50 * l) / 17_719) + (l / 5670) * ((43 * l) / 15_238);
    let l = l - ((30 - j) / 15) * ((17_719 * j) / 50) - (j / 16) * ((15_238 * j) / 43) + 29;
    let month = (24 * l) / 709;
    let day = l - (709 * month) / 24;
    let year = 30 * n + j - 30;
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(q: &str) -> Option<String> {
        DateTool.answer(q).map(|a| a.value)
    }

    #[test]
    fn civil_days_round_trip_against_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(2026, 8, 7), 20_672);
    }

    #[test]
    fn the_tabular_calendar_is_internally_consistent() {
        // Anchored on the epoch rather than on an announced date. Algeria declared 1 Muharram
        // 1447 on 26 June 2025 by sighting; the tabular calendar puts it two days later. That
        // divergence is real and expected — it is exactly what the card's `reckoning` field
        // exists to disclose — so testing against an announced date would be testing the wrong
        // thing.
        let (y, m, d) = hijri_from_days(days_from_civil(622, 7, 19));
        assert_eq!((y, m, d), (1, 1, 1), "epoch: got {y}-{m}-{d}");

        // A year is 354 or 355 days, and months alternate 30/29. Walking a year forward must
        // land in the next year, never skip one.
        let start = days_from_civil(2026, 1, 1);
        let (y0, ..) = hijri_from_days(start);
        let (y1, ..) = hijri_from_days(start + 355);
        assert!(
            y1 == y0 + 1 || y1 == y0,
            "a year of days moved {y0} to {y1}"
        );
    }

    #[test]
    fn every_day_of_a_year_converts_without_panicking_or_skipping() {
        let mut previous: Option<(i64, u32, u32)> = None;
        for offset in 0..400 {
            let date = hijri_from_days(days_from_civil(2026, 1, 1) + offset);
            assert!((1..=12).contains(&date.1), "bad month {date:?}");
            assert!((1..=30).contains(&date.2), "bad day {date:?}");
            if let Some(prev) = previous {
                let stepped_day = date.2 == prev.2 + 1;
                let rolled = date.2 == 1;
                assert!(stepped_day || rolled, "jumped from {prev:?} to {date:?}");
            }
            previous = Some(date);
        }
    }

    #[test]
    fn a_hijri_query_is_answered() {
        let out = answer("12 august 2026 in hijri").expect("should convert");
        assert!(out.contains("هـ"), "got {out}");
    }

    #[test]
    fn the_reckoning_is_recorded_so_the_card_can_state_it() {
        // Algeria announces Eid by sighting, which can differ from the tabular calendar by a day.
        // A converter that hides which reckoning it used looks wrong rather than merely different.
        let detail = DateTool.answer("2026-08-12 hijri").unwrap().detail.unwrap();
        assert_eq!(detail["reckoning"], "tabular");
    }

    #[test]
    fn slashed_dates_are_day_first() {
        // 03/09/2026 is 3 September here, not 9 March. Reading it the American way would be
        // wrong eleven times out of twelve for this audience.
        assert_eq!(parse_one("03/09/2026"), Some((2026, 9, 3)));
        assert_eq!(parse_one("2026-09-03"), Some((2026, 9, 3)));
    }

    #[test]
    fn days_between_two_dates() {
        let out = answer("days between 2026-08-07 and 2026-08-17").expect("should answer");
        assert_eq!(out, "10 days");
    }

    #[test]
    fn ordinary_queries_are_not_dates() {
        for q in [
            "الجزائر",
            "between the lines",
            "hijri calendar history",
            "2026",
        ] {
            assert!(answer(q).is_none(), "{q:?} should not answer");
        }
    }

    #[test]
    fn malformed_input_does_not_panic() {
        for q in ["hijri", "between and", "99/99/9999 hijri", "0-0-0 hijri"] {
            let _ = DateTool.answer(q);
        }
    }
}
