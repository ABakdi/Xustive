//! Date extraction.
//!
//! The hardest part of parsing real pages, and the one most often quietly wrong. A publication
//! date that is silently the crawl date makes freshness ranking a lie.
//!
//! Two Algeria-specific requirements shape this:
//!
//! - **Maghrebi month names.** Algerian Arabic uses `أوت` and `جويلية`, not the Levantine
//!   `أغسطس` and `يوليو`. A parser that only knows the latter fails on most Algerian pages.
//! - **DD/MM, not MM/DD.** `04/08/2026` is 4 August. Resolving it the American way shifts a
//!   third of all dates by months.
//!
//! When nothing parses we record the crawl time with `DatePrecision::Unknown`, never a guess
//! dressed as fact.

use xustive_core::DatePrecision;

/// A parsed date and how much of it we actually know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedDate {
    pub unix: i64,
    pub precision: DatePrecision,
}

/// Arabic month names, including the Maghrebi forms that matter here.
const ARABIC_MONTHS: &[(&str, u32)] = &[
    ("جانفي", 1),
    ("يناير", 1),
    ("فيفري", 2),
    ("فبراير", 2),
    ("مارس", 3),
    ("أفريل", 4),
    ("افريل", 4),
    ("أبريل", 4),
    ("ابريل", 4),
    ("ماي", 5),
    ("مايو", 5),
    ("جوان", 6),
    ("يونيو", 6),
    ("جويلية", 7),
    ("يوليو", 7),
    ("جويليه", 7),
    ("أوت", 8),
    ("اوت", 8),
    ("أغسطس", 8),
    ("اغسطس", 8),
    ("سبتمبر", 9),
    ("أكتوبر", 10),
    ("اكتوبر", 10),
    ("نوفمبر", 11),
    ("ديسمبر", 12),
];

const FRENCH_MONTHS: &[(&str, u32)] = &[
    ("janvier", 1),
    ("fevrier", 2),
    ("février", 2),
    ("mars", 3),
    ("avril", 4),
    ("mai", 5),
    ("juin", 6),
    ("juillet", 7),
    ("aout", 8),
    ("août", 8),
    ("septembre", 9),
    ("octobre", 10),
    ("novembre", 11),
    ("decembre", 12),
    ("décembre", 12),
];

const ENGLISH_MONTHS: &[(&str, u32)] = &[
    ("january", 1),
    ("february", 2),
    ("march", 3),
    ("april", 4),
    ("may", 5),
    ("june", 6),
    ("july", 7),
    ("august", 8),
    ("september", 9),
    ("october", 10),
    ("november", 11),
    ("december", 12),
];

/// Algeria is UTC+1 year round, with no daylight saving.
const ALGIERS_OFFSET_SECONDS: i64 = 3600;

/// Try every strategy, best-precision first.
///
/// `now` is the reference for relative expressions and for rejecting implausible dates; it is
/// passed in rather than read from the clock so parsing is deterministic and testable.
pub fn parse(text: &str, now: i64) -> Option<ParsedDate> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }

    parse_iso8601(t)
        .or_else(|| parse_relative(t, now))
        .or_else(|| parse_named_month(t))
        .or_else(|| parse_numeric(t))
        .filter(|d| plausible(d.unix, now))
}

/// Reject dates that cannot be right: the far future, or before the web existed.
///
/// A day of tolerance for future dates covers timezone slop and scheduled posts; beyond that it
/// is a parse error, and treating it as real would put junk at the top of every recency sort.
fn plausible(unix: i64, now: i64) -> bool {
    const NINETEEN_NINETY_FIVE: i64 = 788_918_400;
    unix > NINETEEN_NINETY_FIVE && unix < now + 86_400
}

/// ISO 8601 / RFC 3339, which is what JSON-LD and `<meta property="article:published_time">` use.
fn parse_iso8601(s: &str) -> Option<ParsedDate> {
    let b = s.as_bytes();
    if b.len() < 10 {
        return None;
    }
    let year: i32 = s.get(0..4)?.parse().ok()?;
    if b[4] != b'-' {
        return None;
    }
    let month: u32 = s.get(5..7)?.parse().ok()?;
    if b[7] != b'-' {
        return None;
    }
    let day: u32 = s.get(8..10)?.parse().ok()?;

    let mut secs = civil_to_unix(year, month, day)?;
    let mut precision = DatePrecision::Day;

    // A time component, with or without a `T`.
    if b.len() >= 19 && (b[10] == b'T' || b[10] == b' ') {
        if let (Ok(h), Ok(mi), Ok(se)) = (
            s[11..13].parse::<i64>(),
            s[14..16].parse::<i64>(),
            s[17..19].parse::<i64>(),
        ) {
            secs += h * 3600 + mi * 60 + se;
            precision = DatePrecision::Second;

            // Offsets. `Z` is UTC; an explicit offset is subtracted to reach UTC. With neither,
            // assume the site is publishing in local time.
            let rest = &s[19..];
            if let Some(idx) = rest.find(['+', '-']) {
                let sign = if rest.as_bytes()[idx] == b'+' { 1 } else { -1 };
                let off = &rest[idx + 1..];
                if off.len() >= 2 {
                    let oh: i64 = off[0..2].parse().unwrap_or(0);
                    let om: i64 = off
                        .get(3..5)
                        .or_else(|| off.get(2..4))
                        .and_then(|m| m.parse().ok())
                        .unwrap_or(0);
                    secs -= sign * (oh * 3600 + om * 60);
                }
            } else if !rest.contains('Z') {
                secs -= ALGIERS_OFFSET_SECONDS;
            }
        }
    }

    Some(ParsedDate {
        unix: secs,
        precision,
    })
}

/// `منذ ساعتين`, `il y a 2 heures`, `2 hours ago`.
///
/// Common on social and forum pages, and useless without a reference point — which is why `now`
/// is threaded through rather than read from the clock.
fn parse_relative(s: &str, now: i64) -> Option<ParsedDate> {
    let lower = s.to_lowercase();

    // Arabic duals, which name a count without writing a number.
    for (needle, secs) in [
        ("منذ دقيقتين", 120i64),
        ("منذ ساعتين", 7200),
        ("منذ يومين", 172_800),
        ("قبل ساعتين", 7200),
        ("قبل يومين", 172_800),
        ("أمس", 86_400),
        ("امس", 86_400),
        ("اليوم", 0),
        ("hier", 86_400),
        ("yesterday", 86_400),
        ("today", 0),
        ("aujourd'hui", 0),
    ] {
        if lower.contains(needle) {
            return Some(ParsedDate {
                unix: now - secs,
                precision: DatePrecision::Day,
            });
        }
    }

    let has_marker = ["منذ", "قبل", "il y a", "ago", "depuis"]
        .iter()
        .any(|m| lower.contains(m));
    if !has_marker {
        return None;
    }

    let digits: String = lower
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let n: i64 = digits.parse().ok()?;

    let unit = [
        (["دقيقة", "دقائق", "minute", "min"].as_slice(), 60i64),
        (["ساعة", "ساعات", "heure", "hour"].as_slice(), 3600),
        (["يوم", "أيام", "ايام", "jour", "day"].as_slice(), 86_400),
        (["أسبوع", "اسبوع", "semaine", "week"].as_slice(), 604_800),
        (["شهر", "أشهر", "mois", "month"].as_slice(), 2_592_000),
        (
            ["سنة", "سنوات", "année", "annee", "year"].as_slice(),
            31_536_000,
        ),
    ]
    .iter()
    .find(|(names, _)| names.iter().any(|nm| lower.contains(nm)))
    .map(|(_, s)| *s)?;

    Some(ParsedDate {
        unix: now - n * unit,
        precision: DatePrecision::Day,
    })
}

/// `4 أوت 2026`, `4 août 2026`, `August 4, 2026`.
fn parse_named_month(s: &str) -> Option<ParsedDate> {
    let lower = s.to_lowercase();

    let month = ARABIC_MONTHS
        .iter()
        .chain(FRENCH_MONTHS.iter())
        .chain(ENGLISH_MONTHS.iter())
        .find(|(name, _)| lower.contains(name))
        .map(|(_, m)| *m)?;

    // Collect the numbers around the month name: one is the day, one the year.
    let mut numbers: Vec<u32> = Vec::new();
    let mut current = String::new();
    for ch in lower.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if !current.is_empty() {
            if let Ok(n) = current.parse() {
                numbers.push(n);
            }
            current.clear();
        }
    }
    if let Ok(n) = current.parse() {
        numbers.push(n);
    }

    let year = numbers
        .iter()
        .copied()
        .find(|n| (1995..=2100).contains(n))?;
    let day = numbers
        .iter()
        .copied()
        .find(|n| (1..=31).contains(n) && *n != year)
        .unwrap_or(1);

    let precision = if numbers.iter().any(|n| (1..=31).contains(n) && *n != year) {
        DatePrecision::Day
    } else {
        DatePrecision::Month
    };

    civil_to_unix(year as i32, month, day).map(|unix| ParsedDate { unix, precision })
}

/// `04/08/2026`, `2026-08-04`, `04.08.2026`.
///
/// **Day first.** `04/08/2026` is 4 August here, not 8 April. Reading it the American way would
/// shift a large share of Algerian dates by months, and the error is invisible in aggregate.
fn parse_numeric(s: &str) -> Option<ParsedDate> {
    let parts: Vec<&str> = s
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() < 3 {
        return None;
    }

    let (a, b, c): (u32, u32, u32) = (
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    );

    // Year-first (ISO-like) when the leading field is four digits.
    let (year, month, day) = if parts[0].len() == 4 {
        (a, b, c)
    } else if a > 12 {
        // Unambiguous: the first field cannot be a month.
        (c, b, a)
    } else if b > 12 {
        // The second field cannot be a month, so this one really is month-first.
        (c, a, b)
    } else {
        // Ambiguous. Algerian convention is day first.
        (c, b, a)
    };

    if !(1995..=2100).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    civil_to_unix(year as i32, month, day).map(|unix| ParsedDate {
        unix,
        precision: DatePrecision::Day,
    })
}

/// Days-from-civil (Howard Hinnant), then to seconds. Correct across leap years and centuries.
fn civil_to_unix(year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as i64;
    let m = month as i64;
    let d = day as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era as i64 * 146_097 + doe - 719_468;
    Some(days * 86_400)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-08-06T12:00:00Z
    const NOW: i64 = 1_786_017_600;

    fn ymd(p: ParsedDate) -> (i32, u32, u32) {
        let z = p.unix.div_euclid(86_400) + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        ((if m <= 2 { y + 1 } else { y }) as i32, m as u32, d as u32)
    }

    #[test]
    fn iso8601_with_timezone() {
        let p = parse("2026-08-04T14:30:00Z", NOW).unwrap();
        assert_eq!(ymd(p), (2026, 8, 4));
        assert_eq!(p.precision, DatePrecision::Second);
    }

    #[test]
    fn iso8601_date_only() {
        let p = parse("2026-08-04", NOW).unwrap();
        assert_eq!(ymd(p), (2026, 8, 4));
        assert_eq!(p.precision, DatePrecision::Day);
    }

    #[test]
    fn iso8601_with_explicit_offset() {
        let with = parse("2026-08-04T14:30:00+01:00", NOW).unwrap();
        let utc = parse("2026-08-04T13:30:00Z", NOW).unwrap();
        assert_eq!(with.unix, utc.unix, "the offset should be applied");
    }

    #[test]
    fn algerian_arabic_month_names() {
        // The names that matter: أوت and جويلية, not the Levantine forms.
        assert_eq!(ymd(parse("4 أوت 2026", NOW).unwrap()), (2026, 8, 4));
        assert_eq!(ymd(parse("12 جويلية 2026", NOW).unwrap()), (2026, 7, 12));
        assert_eq!(ymd(parse("3 جانفي 2026", NOW).unwrap()), (2026, 1, 3));
        assert_eq!(ymd(parse("20 فيفري 2026", NOW).unwrap()), (2026, 2, 20));
        assert_eq!(ymd(parse("15 أفريل 2026", NOW).unwrap()), (2026, 4, 15));
    }

    #[test]
    fn levantine_month_names_still_work() {
        // Syndicated content uses them, so both must parse.
        assert_eq!(ymd(parse("4 أغسطس 2026", NOW).unwrap()), (2026, 8, 4));
        assert_eq!(ymd(parse("12 يوليو 2026", NOW).unwrap()), (2026, 7, 12));
    }

    #[test]
    fn french_month_names_with_and_without_accents() {
        assert_eq!(ymd(parse("4 août 2026", NOW).unwrap()), (2026, 8, 4));
        assert_eq!(ymd(parse("4 aout 2026", NOW).unwrap()), (2026, 8, 4));
        // A past date: December 2026 is after NOW, and rejecting the future is
        // deliberate — see `future_dates_are_rejected`.
        assert_eq!(ymd(parse("12 décembre 2025", NOW).unwrap()), (2025, 12, 12));
    }

    #[test]
    fn english_month_names() {
        assert_eq!(ymd(parse("August 4, 2026", NOW).unwrap()), (2026, 8, 4));
    }

    #[test]
    fn ambiguous_numeric_dates_are_read_day_first() {
        // The single most consequential convention here. 04/08 is 4 August, not 8 April.
        assert_eq!(ymd(parse("04/08/2026", NOW).unwrap()), (2026, 8, 4));
        assert_eq!(ymd(parse("04.08.2026", NOW).unwrap()), (2026, 8, 4));
        assert_eq!(ymd(parse("04-08-2026", NOW).unwrap()), (2026, 8, 4));
    }

    #[test]
    fn unambiguous_numeric_dates_resolve_correctly() {
        // Day > 12 settles it regardless of convention.
        assert_eq!(ymd(parse("25/12/2025", NOW).unwrap()), (2025, 12, 25));
        // Year-first is recognised by the four-digit leading field.
        assert_eq!(ymd(parse("2026/08/04", NOW).unwrap()), (2026, 8, 4));
    }

    #[test]
    fn relative_dates_need_a_reference_point() {
        let two_hours = parse("منذ ساعتين", NOW).unwrap();
        assert_eq!(two_hours.unix, NOW - 7200);

        let three_days = parse("il y a 3 jours", NOW).unwrap();
        assert_eq!(three_days.unix, NOW - 3 * 86_400);

        assert_eq!(parse("2 hours ago", NOW).unwrap().unix, NOW - 7200);
        assert_eq!(parse("أمس", NOW).unwrap().unix, NOW - 86_400);
        assert_eq!(parse("hier", NOW).unwrap().unix, NOW - 86_400);
    }

    #[test]
    fn future_dates_are_rejected() {
        // A parse that yields tomorrow is a parse error, and treating it as real would put
        // junk at the top of every recency sort.
        assert!(parse("2099-01-01", NOW).is_none());
        assert!(parse("1 جانفي 2099", NOW).is_none());
    }

    #[test]
    fn pre_web_dates_are_rejected() {
        assert!(parse("1970-01-01", NOW).is_none());
        assert!(parse("01/01/1980", NOW).is_none());
    }

    #[test]
    fn a_small_future_offset_is_tolerated() {
        // Timezone slop and scheduled posts should not be discarded.
        let soon = NOW + 3600;
        let iso = "2026-08-06T13:30:00Z";
        assert!(parse(iso, soon - 7200).is_some());
    }

    #[test]
    fn leap_years_are_handled() {
        assert_eq!(ymd(parse("2024-02-29", NOW).unwrap()), (2024, 2, 29));
        assert_eq!(ymd(parse("2000-02-29", NOW).unwrap()), (2000, 2, 29));
    }

    #[test]
    fn month_only_dates_report_month_precision() {
        let p = parse("أوت 2026", NOW).unwrap();
        assert_eq!(p.precision, DatePrecision::Month);
        assert_eq!(ymd(p), (2026, 8, 1));
    }

    #[test]
    fn garbage_returns_none_rather_than_a_guess() {
        for s in ["", "   ", "hello world", "not a date", "12345", "///"] {
            assert!(parse(s, NOW).is_none(), "{s:?} should not parse");
        }
    }

    #[test]
    fn real_world_strings_from_algerian_pages() {
        let cases = [
            "2026-08-04T09:15:00+01:00",
            "الأربعاء 04 أوت 2026",
            "Publié le 4 août 2026 à 09:15",
            "04/08/2026 - 09:15",
        ];
        for c in cases {
            let p = parse(c, NOW).unwrap_or_else(|| panic!("failed to parse {c:?}"));
            assert_eq!(ymd(p).0, 2026, "wrong year for {c:?}");
            assert_eq!(ymd(p).1, 8, "wrong month for {c:?}");
        }
    }
}
