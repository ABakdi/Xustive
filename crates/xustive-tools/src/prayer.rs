//! Prayer times.
//!
//! Computed from coordinates and date — no network, no cache, nothing that can go stale. That
//! matters more here than for most tools: a wrong prayer time is not an inconvenience, and a
//! stale one served from a cache would be wrong every day after the first.
//!
//! # Why the method is shown on the card
//!
//! Fajr and Isha depend on a twilight angle that different authorities set differently, and the
//! results genuinely differ — often by fifteen minutes or more. Algerian mosques do not all
//! follow one authority.
//!
//! A card that displays times without naming its method makes an ordinary disagreement look like
//! a defect. Naming it turns "this is wrong" into "this is a different reckoning from my mosque",
//! which is true and actionable.
//!
//! # What this is not
//!
//! Not an authority. It is arithmetic anyone can check, offered so that a search does not have to
//! leave for a site that will track you to answer it. Where it disagrees with a local mosque, the
//! mosque is right.

use crate::{Answer, Tool};

/// A calculation method: the twilight angles it uses for Fajr and Isha.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Umm al-Qura, Mecca. Isha is a fixed 90 minutes after Maghrib rather than an angle.
    UmmAlQura,
    /// Muslim World League. Widely used across the Maghreb.
    Mwl,
    /// Egyptian General Authority of Survey.
    Egyptian,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UmmAlQura => "umm-al-qura",
            Self::Mwl => "muslim-world-league",
            Self::Egyptian => "egyptian",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::UmmAlQura => "أم القرى",
            Self::Mwl => "رابطة العالم الإسلامي",
            Self::Egyptian => "الهيئة المصرية العامة للمساحة",
        }
    }

    /// Sun depression below the horizon at Fajr, in degrees.
    fn fajr_angle(self) -> f64 {
        match self {
            Self::UmmAlQura => 18.5,
            Self::Mwl => 18.0,
            Self::Egyptian => 19.5,
        }
    }

    /// Isha as an angle, or `None` when the method uses a fixed interval after Maghrib.
    fn isha_angle(self) -> Option<f64> {
        match self {
            // Umm al-Qura sets Isha 90 minutes after Maghrib and does not use an angle at all.
            // Treating it as one would produce a plausible time that is simply not the method's.
            Self::UmmAlQura => None,
            Self::Mwl => Some(17.0),
            Self::Egyptian => Some(17.5),
        }
    }
}

/// Which shadow ratio marks Asr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsrRule {
    /// Shafi'i, Maliki, Hanbali — shadow equal to the object's length. The Maliki school is
    /// dominant in Algeria, so this is the default.
    Standard,
    /// Hanafi — shadow twice the object's length, roughly an hour later.
    Hanafi,
}

/// Times for one day, as minutes after local midnight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Times {
    pub fajr: f64,
    pub sunrise: f64,
    pub dhuhr: f64,
    pub asr: f64,
    pub maghrib: f64,
    pub isha: f64,
}

impl Times {
    pub fn named(&self) -> [(&'static str, &'static str, f64); 6] {
        [
            ("fajr", "الفجر", self.fajr),
            ("sunrise", "الشروق", self.sunrise),
            ("dhuhr", "الظهر", self.dhuhr),
            ("asr", "العصر", self.asr),
            ("maghrib", "المغرب", self.maghrib),
            ("isha", "العشاء", self.isha),
        ]
    }
}

/// Compute times for a date and place.
///
/// `days` is days since the Unix epoch; `timezone` is the offset in hours (Algeria is UTC+1 all
/// year, with no daylight saving).
///
/// Returns `None` at extreme latitudes where the sun does not reach the required depression and
/// the times are undefined. Algeria never triggers that, but a caller passing arbitrary
/// coordinates deserves an honest absence rather than a fabricated time.
pub fn times(
    days: i64,
    latitude: f64,
    longitude: f64,
    timezone: f64,
    method: Method,
    asr: AsrRule,
) -> Option<Times> {
    if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
        return None;
    }

    // Julian day at noon, then centuries since J2000 — the standard basis for solar position.
    let jd = days as f64 + 2_440_587.5 + 0.5;
    let d = jd - 2_451_545.0;

    // Mean solar longitude and anomaly, in degrees.
    let g = (357.529 + 0.985_600_28 * d).rem_euclid(360.0);
    let q = (280.459 + 0.985_647_36 * d).rem_euclid(360.0);
    let l =
        (q + 1.915 * g.to_radians().sin() + 0.020 * (2.0 * g).to_radians().sin()).rem_euclid(360.0);

    // Obliquity of the ecliptic.
    let e = 23.439 - 0.000_000_36 * d;

    // Declination, and the equation of time in minutes.
    let declination = (e.to_radians().sin() * l.to_radians().sin()).asin();
    let right_ascension = (e.to_radians().cos() * l.to_radians().sin())
        .atan2(l.to_radians().cos())
        .to_degrees()
        .rem_euclid(360.0);
    let equation_of_time = (q - right_ascension + 180.0).rem_euclid(360.0) - 180.0;
    let equation_of_time = equation_of_time * 4.0; // degrees to minutes

    let lat = latitude.to_radians();

    // Solar noon, adjusted for longitude within the timezone and for the equation of time.
    let dhuhr = 12.0 + timezone - longitude / 15.0 - equation_of_time / 60.0;

    // Hour angle for a given depression below the horizon.
    let hour_angle = |angle_deg: f64| -> Option<f64> {
        let numerator = (-angle_deg.to_radians()).sin() - lat.sin() * declination.sin();
        let denominator = lat.cos() * declination.cos();
        let cosine = numerator / denominator;
        // Outside [-1, 1] the sun never reaches that depression on this day at this latitude.
        (-1.0..=1.0)
            .contains(&cosine)
            .then(|| cosine.acos().to_degrees() / 15.0)
    };

    // Sunrise and sunset use 0.833°, which accounts for atmospheric refraction and the sun's
    // apparent radius — the disc's upper edge, not its centre, defines both.
    let sun = hour_angle(0.833)?;
    let sunrise = dhuhr - sun;
    let maghrib = dhuhr + sun;

    let fajr = dhuhr - hour_angle(method.fajr_angle())?;

    let isha = match method.isha_angle() {
        Some(angle) => dhuhr + hour_angle(angle)?,
        // Umm al-Qura: a fixed 90 minutes after Maghrib.
        None => maghrib + 1.5,
    };

    // Asr: when an object's shadow reaches `factor` times its length, plus its noon shadow.
    let factor = match asr {
        AsrRule::Standard => 1.0,
        AsrRule::Hanafi => 2.0,
    };
    let asr_altitude = (1.0 / (factor + (lat - declination).abs().tan())).atan();
    // `hour_angle` takes a depression *below* the horizon, and Asr is an altitude *above* it —
    // so the sign flips. Passing `90 - altitude` (the zenith angle) instead put the sun on the
    // wrong side of the horizon: Asr landed after Maghrib, and for most methods the cosine went
    // out of range and the whole day returned nothing.
    let asr_time = dhuhr + hour_angle(-asr_altitude.to_degrees())?;

    Some(Times {
        fajr: fajr * 60.0,
        sunrise: sunrise * 60.0,
        dhuhr: dhuhr * 60.0,
        asr: asr_time * 60.0,
        maghrib: maghrib * 60.0,
        isha: isha * 60.0,
    })
}

pub fn format_minutes(minutes: f64) -> String {
    let total = minutes.round().rem_euclid(1440.0) as i64;
    format!("{:02}:{:02}", total / 60, total % 60)
}

pub struct PrayerTool;

impl Tool for PrayerTool {
    fn name(&self) -> &'static str {
        "prayer-times"
    }

    fn keyword(&self) -> &'static str {
        "salat"
    }

    fn answer(&self, query: &str) -> Option<Answer> {
        const TRIGGERS: &[&str] = &[
            "مواقيت الصلاة",
            "اوقات الصلاة",
            "أوقات الصلاة",
            "وقت الصلاة",
            "heure priere",
            "heures priere",
            "heure de priere",
            "horaires priere",
            "prayer times",
            "salat times",
        ];

        // Same folding as the wilaya lookup: `heure priere` and `heure prière` are the same
        // request, and only one of them is what anybody actually types.
        let folded = crate::wilaya::fold_for_match(query);
        let matched = TRIGGERS
            .iter()
            .find(|t| folded.contains(&crate::wilaya::fold_for_match(t)))?;

        // The wilaya named in the query, or the capital. Never geolocation — a search box must
        // not ask for a location permission to answer a question about times.
        let wilaya = crate::wilaya::find(query).unwrap_or_else(crate::wilaya::default_wilaya);

        // Today, in Algeria. Passed as days since epoch so the computation stays testable.
        let days = xustive_core::now_unix().div_euclid(86_400);
        let times = times(
            days,
            wilaya.latitude,
            wilaya.longitude,
            1.0, // Algeria is UTC+1 all year, with no daylight saving.
            Method::UmmAlQura,
            AsrRule::Standard,
        )?;

        let listed: Vec<String> = times
            .named()
            .iter()
            .filter(|(key, ..)| *key != "sunrise")
            .map(|(_, label, minutes)| format!("{label} {}", format_minutes(*minutes)))
            .collect();

        Some(Answer {
            tool: self.name(),
            // Keyed: an explicit trigger phrase. Not structural, because "prayer times" is also
            // a thing somebody might be searching articles about.
            confidence: if matched.len() > 12 { 0.92 } else { 0.85 },
            interpretation: format!("{} · {}", wilaya.name_ar, Method::UmmAlQura.label()),
            value: listed.join(" · "),
            detail: Some(serde_json::json!({
                "wilaya": { "code": wilaya.code, "ar": wilaya.name_ar, "fr": wilaya.name_fr },
                "times": times.named().iter().map(|(k, ar, m)| serde_json::json!({
                    "key": k, "label_ar": ar, "time": format_minutes(*m),
                })).collect::<Vec<_>>(),
                // Surfaced so the card can state it. Times differing from a local mosque by a few
                // minutes is normal; a card that hides its method makes that look like an error.
                "method": Method::UmmAlQura.as_str(),
                "method_label": Method::UmmAlQura.label(),
                "asr_rule": "standard",
            })),
            // Computed, not fetched. There is nothing to be stale.
            as_of: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Days since epoch for a civil date.
    fn days(y: i64, m: u32, d: u32) -> i64 {
        crate::datetime::days_from_civil(y, m, d)
    }

    /// Algiers.
    const ALGIERS: (f64, f64) = (36.7538, 3.0588);

    #[test]
    fn times_are_ordered_through_the_day() {
        // The property that catches almost every arithmetic error: the sun rises before noon,
        // noon precedes afternoon, and night follows sunset.
        for day in [
            days(2026, 1, 15),
            days(2026, 3, 21),
            days(2026, 6, 21),
            days(2026, 9, 23),
            days(2026, 12, 21),
        ] {
            let t = times(
                day,
                ALGIERS.0,
                ALGIERS.1,
                1.0,
                Method::Mwl,
                AsrRule::Standard,
            )
            .expect("Algiers always has prayer times");
            assert!(
                t.fajr < t.sunrise,
                "fajr {} !< sunrise {}",
                t.fajr,
                t.sunrise
            );
            assert!(t.sunrise < t.dhuhr);
            assert!(t.dhuhr < t.asr);
            assert!(t.asr < t.maghrib);
            assert!(t.maghrib < t.isha);
        }
    }

    #[test]
    fn algiers_times_are_close_to_published_ones() {
        // 15 June 2026, Algiers. Published tables put Dhuhr near 12:50 and Maghrib near 20:15.
        // A twenty-minute tolerance: the point is to catch an implementation that is an hour or
        // a hemisphere out, not to claim agreement to the minute with any particular table.
        let t = times(
            days(2026, 6, 15),
            ALGIERS.0,
            ALGIERS.1,
            1.0,
            Method::Mwl,
            AsrRule::Standard,
        )
        .unwrap();
        let dhuhr_h = t.dhuhr / 60.0;
        let maghrib_h = t.maghrib / 60.0;
        assert!(
            (dhuhr_h - 12.83).abs() < 0.34,
            "dhuhr {}",
            format_minutes(t.dhuhr)
        );
        assert!(
            (maghrib_h - 20.25).abs() < 0.34,
            "maghrib {}",
            format_minutes(t.maghrib)
        );
    }

    #[test]
    fn the_method_changes_fajr_and_isha_and_nothing_else() {
        // Methods differ only in twilight angles. If Dhuhr moves when the method changes, the
        // angle has leaked into the solar calculation.
        let day = days(2026, 4, 10);
        let mwl = times(
            day,
            ALGIERS.0,
            ALGIERS.1,
            1.0,
            Method::Mwl,
            AsrRule::Standard,
        )
        .unwrap();
        let egy = times(
            day,
            ALGIERS.0,
            ALGIERS.1,
            1.0,
            Method::Egyptian,
            AsrRule::Standard,
        )
        .unwrap();

        assert_eq!(mwl.dhuhr, egy.dhuhr);
        assert_eq!(mwl.maghrib, egy.maghrib);
        assert_eq!(mwl.asr, egy.asr);
        // Egyptian uses a deeper Fajr angle, so its Fajr is earlier.
        assert!(egy.fajr < mwl.fajr, "egyptian fajr should be earlier");
    }

    #[test]
    fn umm_al_qura_puts_isha_ninety_minutes_after_maghrib() {
        // It uses a fixed interval, not an angle. Treating it as an angle would give a plausible
        // time that is simply not this method's.
        let t = times(
            days(2026, 4, 10),
            ALGIERS.0,
            ALGIERS.1,
            1.0,
            Method::UmmAlQura,
            AsrRule::Standard,
        )
        .unwrap();
        assert!(
            (t.isha - t.maghrib - 90.0).abs() < 0.001,
            "got {}",
            t.isha - t.maghrib
        );
    }

    #[test]
    fn hanafi_asr_is_later_than_standard() {
        // Twice the shadow length, roughly an hour later. The two schools genuinely differ and
        // the choice is the user's.
        let day = days(2026, 4, 10);
        let standard = times(
            day,
            ALGIERS.0,
            ALGIERS.1,
            1.0,
            Method::Mwl,
            AsrRule::Standard,
        )
        .unwrap();
        let hanafi = times(day, ALGIERS.0, ALGIERS.1, 1.0, Method::Mwl, AsrRule::Hanafi).unwrap();
        assert!(hanafi.asr > standard.asr);
        assert!(
            hanafi.asr - standard.asr > 20.0,
            "only {} minutes apart",
            hanafi.asr - standard.asr
        );
    }

    #[test]
    fn southern_wilayas_differ_from_the_capital() {
        // Tamanrasset is 1 500 km south of Algiers. If the coordinates were being ignored, the
        // times would be identical — which is a bug that looks like everything working.
        let day = days(2026, 6, 21);
        let algiers = times(day, 36.75, 3.06, 1.0, Method::Mwl, AsrRule::Standard).unwrap();
        let tam = times(day, 22.78, 5.52, 1.0, Method::Mwl, AsrRule::Standard).unwrap();
        assert!(
            (algiers.maghrib - tam.maghrib).abs() > 30.0,
            "coordinates are being ignored"
        );
    }

    #[test]
    fn extreme_latitudes_return_nothing_rather_than_a_fabricated_time() {
        // At midsummer inside the Arctic circle the sun never reaches the Fajr depression, so the
        // time is undefined. Returning a number anyway would be inventing one.
        assert!(times(
            days(2026, 6, 21),
            78.0,
            15.0,
            1.0,
            Method::Mwl,
            AsrRule::Standard
        )
        .is_none());
    }

    #[test]
    fn invalid_coordinates_are_refused() {
        assert!(times(
            days(2026, 6, 21),
            200.0,
            0.0,
            1.0,
            Method::Mwl,
            AsrRule::Standard
        )
        .is_none());
        assert!(times(
            days(2026, 6, 21),
            0.0,
            900.0,
            1.0,
            Method::Mwl,
            AsrRule::Standard
        )
        .is_none());
    }

    #[test]
    fn formatting_wraps_and_pads() {
        assert_eq!(format_minutes(0.0), "00:00");
        assert_eq!(format_minutes(65.4), "01:05");
        assert_eq!(
            format_minutes(1439.6),
            "00:00",
            "rounds up past midnight and wraps"
        );
    }

    #[test]
    fn the_tool_answers_arabic_and_french_triggers() {
        for q in [
            "مواقيت الصلاة",
            "مواقيت الصلاة وهران",
            "heure priere Setif",
            "prayer times",
        ] {
            let answer = PrayerTool.answer(q);
            assert!(answer.is_some(), "{q:?} should answer");
            let answer = answer.unwrap();
            assert!(answer.value.contains("الفجر"), "got {}", answer.value);
            // The method is on the card, not buried in settings.
            assert!(
                answer.interpretation.contains("أم القرى"),
                "got {}",
                answer.interpretation
            );
        }
    }

    #[test]
    fn a_named_wilaya_changes_the_answer() {
        let algiers = PrayerTool.answer("مواقيت الصلاة").unwrap();
        let tam = PrayerTool.answer("مواقيت الصلاة تمنراست").unwrap();
        assert_ne!(algiers.value, tam.value, "the named wilaya must be used");
    }

    #[test]
    fn ordinary_queries_do_not_trigger_it() {
        for q in [
            "الجزائر",
            "صلاة الجمعة خطبة",
            "mosque architecture",
            "prayer",
        ] {
            assert!(PrayerTool.answer(q).is_none(), "{q:?} should not answer");
        }
    }
}
