//! Weather for the 58 wilayas — and, since 2026-08-29, for the world cities in
//! [`xustive_tools::city`], on a slower schedule ([[Instant Answers]] §weather).
//!
//! From Open-Meteo: CC-BY licensed, no API key, and — the reason it was chosen over the
//! alternatives — no per-user call. We fetch 58 places every thirty minutes on a fixed schedule,
//! which is 116 requests an hour and reveals nothing: the pattern is identical whether one person
//! searched for weather or a million did.
//!
//! A provider requiring a key per request would have made every weather search a disclosure to
//! them of what somebody looked up and roughly when.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use xustive_tools::place::Place;
use xustive_tools::wilaya::WILAYAS;

use crate::validate::{self, Rejected};
use crate::{Cached, Dataset, FetchError};

pub struct Weather;

impl Dataset for Weather {
    /// Bumped to `v2` for M8-T05.2, which added the hourly series and the sixth and seventh days.
    /// A version rather than a migration: entries written by the older build simply stop being
    /// read and age out, which is cheaper and safer than teaching the reader two shapes.
    fn key_prefix(&self) -> &'static str {
        "tool:weather:v2"
    }

    fn cadence(&self) -> Duration {
        Duration::from_secs(30 * 60)
    }

    fn staleness_limit(&self) -> Duration {
        Duration::from_secs(3 * 3600)
    }
}

/// One place's forecast, as the card needs it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Forecast {
    /// The place key ([`Place::key`]): a wilaya code, or `c-<slug>` for a world city.
    pub place: String,
    pub temperature_c: f64,
    pub feels_like_c: f64,
    /// WMO weather code. Mapped to an icon and a label by the client rather than here, so the
    /// wording stays with the translations.
    pub code: u8,
    pub wind_kmh: f64,
    pub humidity: f64,
    pub days: Vec<Day>,
    /// The next 48 hours. Defaulted so a `v1` entry still deserialises during a rollback.
    #[serde(default)]
    pub hours: Vec<Hour>,
}

/// One hour of the near-term series (M8-T05.2), which is what the graph is drawn from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Hour {
    /// `YYYY-MM-DDTHH:MM`, local to Algiers like every other time here.
    pub time: String,
    pub temperature_c: f64,
    /// Probability of precipitation, 0-100. The second series on the graph, and the one people
    /// actually decide things on.
    pub precipitation_chance: f64,
    pub code: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Day {
    /// `YYYY-MM-DD`.
    pub date: String,
    pub high_c: f64,
    pub low_c: f64,
    pub code: u8,
}

pub fn key(prefix: &str, place: &str) -> String {
    format!("{prefix}:{place}")
}

/// Bounds for Algeria.
///
/// Wide enough for a Saharan summer and a Djurdjura winter, narrow enough that a sensor fault or
/// a Fahrenheit/Celsius confusion is caught. The record extremes are roughly -14 °C and 51 °C.
const MIN_C: f64 = -25.0;
const MAX_C: f64 = 58.0;

/// The world is colder and hotter than Algeria. Wide enough for Yakutsk and Kuwait, narrow
/// enough that a Fahrenheit reading (a summer 100 °F) is still caught.
const WORLD_MIN_C: f64 = -70.0;
const WORLD_MAX_C: f64 = 60.0;

/// How much of the hourly series to keep. Two days: beyond that an hourly figure is false
/// precision, and a graph with 168 points is a smear rather than a forecast.
const HOURS_KEPT: usize = 48;

/// The Open-Meteo response, as much of it as we use.
#[derive(Debug, Deserialize)]
struct ApiResponse {
    /// The offset of the place's own clock, which `timezone=auto` makes vary per city. Without
    /// it every non-Algerian reading looked hours old and was rejected as implausible.
    #[serde(default = "algiers_offset")]
    utc_offset_seconds: i64,
    current: Current,
    daily: Daily,
    #[serde(default)]
    hourly: Option<Hourly>,
}

/// What Algeria's clock is, for a response that did not say.
fn algiers_offset() -> i64 {
    3_600
}

#[derive(Debug, Deserialize)]
struct Hourly {
    time: Vec<String>,
    temperature_2m: Vec<f64>,
    precipitation_probability: Vec<Option<f64>>,
    weather_code: Vec<u8>,
}

#[derive(Debug, Deserialize)]
struct Current {
    time: String,
    temperature_2m: f64,
    apparent_temperature: f64,
    relative_humidity_2m: f64,
    weather_code: u8,
    wind_speed_10m: f64,
}

#[derive(Debug, Deserialize)]
struct Daily {
    time: Vec<String>,
    temperature_2m_max: Vec<f64>,
    temperature_2m_min: Vec<f64>,
    weather_code: Vec<u8>,
}

/// Fetch one place.
pub async fn fetch(
    client: &reqwest::Client,
    place: &Place,
    now: i64,
    previous: Option<&Forecast>,
) -> Result<Cached<Forecast>, FetchError> {
    let (latitude, longitude) = place.coordinates();
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={:.4}&longitude={:.4}\
         &current=temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m\
         &daily=weather_code,temperature_2m_max,temperature_2m_min\
         &hourly=temperature_2m,precipitation_probability,weather_code\
         &timezone=auto&forecast_days=7",
        latitude, longitude
    );

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| FetchError::Http(e.to_string()))?;
    if !response.status().is_success() {
        return Err(FetchError::Http(format!("status {}", response.status())));
    }
    let body: ApiResponse = response
        .json()
        .await
        .map_err(|e| FetchError::Parse(e.to_string()))?;

    build(body, place, now, previous)
}

/// Turn a response into a validated cache entry.
///
/// Separated from the request so every check below is testable without a network — which matters,
/// because these are the checks that only ever run against data we did not write.
fn build(
    body: ApiResponse,
    place: &Place,
    now: i64,
    previous: Option<&Forecast>,
) -> Result<Cached<Forecast>, FetchError> {
    let reject = |r: Rejected| FetchError::Rejected(r.to_string());
    // Algeria's bounds are tight enough to catch a sensor fault; a city elsewhere needs the
    // world's, or every Moscow winter would be rejected as implausible.
    let (min_c, max_c) = if place.is_wilaya() {
        (MIN_C, MAX_C)
    } else {
        (WORLD_MIN_C, WORLD_MAX_C)
    };

    // The publisher's own observation time, not ours.
    let observed_at =
        parse_local_time(&body.current.time, body.utc_offset_seconds).ok_or_else(|| {
            reject(Rejected::Missing {
                field: "current.time".into(),
            })
        })?;
    validate::timestamp(observed_at, now, Duration::from_secs(6 * 3600)).map_err(reject)?;

    validate::bounded("temperature", body.current.temperature_2m, min_c, max_c).map_err(reject)?;
    validate::bounded(
        "apparent_temperature",
        body.current.apparent_temperature,
        min_c,
        max_c,
    )
    .map_err(reject)?;
    validate::bounded("humidity", body.current.relative_humidity_2m, 0.0, 100.0).map_err(reject)?;
    validate::bounded("wind", body.current.wind_speed_10m, 0.0, 250.0).map_err(reject)?;

    // Absolute, not fractional: near 0 °C a fractional guard divides by almost nothing and
    // rejects every ordinary reading. Twenty-five degrees in half an hour is a sensor fault
    // anywhere on earth.
    if let Some(previous) = previous {
        let jump = (body.current.temperature_2m - previous.temperature_c).abs();
        if jump > 25.0 {
            return Err(reject(Rejected::Moved {
                field: "temperature".into(),
                from: previous.temperature_c,
                to: body.current.temperature_2m,
            }));
        }
    }

    let days: Vec<Day> = body
        .daily
        .time
        .iter()
        .zip(body.daily.temperature_2m_max.iter())
        .zip(body.daily.temperature_2m_min.iter())
        .zip(body.daily.weather_code.iter())
        .map(|(((date, high), low), code)| Day {
            date: date.clone(),
            high_c: *high,
            low_c: *low,
            code: *code,
        })
        .collect();

    if days.is_empty() {
        return Err(reject(Rejected::Missing {
            field: "daily".into(),
        }));
    }
    for day in &days {
        validate::bounded("daily high", day.high_c, min_c, max_c).map_err(reject)?;
        validate::bounded("daily low", day.low_c, min_c, max_c).map_err(reject)?;
        // A low above its high means the two series are misaligned, which would render as a
        // forecast that is merely odd rather than obviously broken.
        if day.low_c > day.high_c {
            return Err(reject(Rejected::OutOfBounds {
                field: format!("{} low above high", day.date),
                value: day.low_c,
            }));
        }
    }

    // The hourly series is capped at two days: beyond that an hourly number is false precision,
    // and the graph it feeds is unreadable at 168 points anyway.
    let hours: Vec<Hour> = match &body.hourly {
        Some(h) => h
            .time
            .iter()
            .zip(h.temperature_2m.iter())
            .zip(h.weather_code.iter())
            .enumerate()
            .take(HOURS_KEPT)
            .map(|(i, ((time, temp), code))| Hour {
                time: time.clone(),
                temperature_c: *temp,
                // Open-Meteo sends null past the model horizon rather than omitting the entry, so
                // a missing probability is "not forecast", which is 0 rather than a rejection.
                precipitation_chance: h
                    .precipitation_probability
                    .get(i)
                    .copied()
                    .flatten()
                    .unwrap_or(0.0),
                code: *code,
            })
            .collect(),
        None => Vec::new(),
    };
    for hour in &hours {
        validate::bounded("hourly temperature", hour.temperature_c, min_c, max_c)
            .map_err(reject)?;
        validate::bounded(
            "precipitation chance",
            hour.precipitation_chance,
            0.0,
            100.0,
        )
        .map_err(reject)?;
    }

    Ok(Cached {
        fetched_at: now,
        observed_at,
        source: "open-meteo".into(),
        licence: "CC-BY-4.0".into(),
        payload: Forecast {
            place: place.key(),
            temperature_c: body.current.temperature_2m,
            feels_like_c: body.current.apparent_temperature,
            code: body.current.weather_code,
            wind_kmh: body.current.wind_speed_10m,
            humidity: body.current.relative_humidity_2m,
            days,
            hours,
        },
    })
}

/// Parse Open-Meteo's local timestamp, `YYYY-MM-DDTHH:MM`.
///
/// The response is in Africa/Algiers because we asked for it, and Algeria is UTC+1 year-round
/// with no daylight saving — so one fixed offset is correct rather than a simplification.
fn parse_local_time(text: &str, utc_offset_seconds: i64) -> Option<i64> {
    let (date, time) = text.split_once('T')?;
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;

    let mut hm = time.split(':');
    let hour: i64 = hm.next()?.parse().ok()?;
    let minute: i64 = hm.next()?.parse().ok()?;

    let days = xustive_tools::datetime::days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3_600 + minute * 60 - utc_offset_seconds)
}

/// Every wilaya, for the scheduler to walk on each pass.
pub fn targets() -> Vec<Place> {
    WILAYAS.iter().map(Place::Wilaya).collect()
}

/// The world cities, walked on a slower schedule ([`WORLD_EVERY`]): they change no faster than
/// Algeria's weather, but they are the part of the list that could grow without limit, and the
/// publisher's free tier is a real ceiling.
pub fn world_targets() -> Vec<Place> {
    xustive_tools::city::CITIES
        .iter()
        .map(Place::City)
        .collect()
}

/// How many wilaya passes go by between two world passes. At the 30-minute cadence that is a
/// world refresh every two hours, inside the three-hour staleness limit.
pub const WORLD_EVERY: u64 = 4;

#[cfg(test)]
mod tests {
    use super::*;

    fn algiers() -> Place {
        Place::Wilaya(xustive_tools::wilaya::by_code(16).unwrap())
    }

    fn response(temp: f64, time: &str) -> ApiResponse {
        ApiResponse {
            utc_offset_seconds: 3_600,
            current: Current {
                time: time.into(),
                temperature_2m: temp,
                apparent_temperature: temp + 1.0,
                relative_humidity_2m: 55.0,
                weather_code: 1,
                wind_speed_10m: 12.0,
            },
            daily: Daily {
                time: vec!["2026-08-07".into(), "2026-08-08".into()],
                temperature_2m_max: vec![34.0, 35.0],
                temperature_2m_min: vec![22.0, 23.0],
                weather_code: vec![0, 1],
            },
            hourly: Some(Hourly {
                time: vec!["2026-08-07T11:00".into(), "2026-08-07T12:00".into()],
                temperature_2m: vec![temp, temp + 0.5],
                precipitation_probability: vec![Some(10.0), None],
                weather_code: vec![1, 2],
            }),
        }
    }

    /// 7 August 2026, 12:00 Algiers time.
    fn now() -> i64 {
        parse_local_time("2026-08-07T12:00", 3_600).unwrap()
    }

    #[test]
    fn the_hourly_series_is_kept_and_a_null_probability_reads_as_not_forecast() {
        // Open-Meteo sends null past the model horizon rather than omitting the entry. That is
        // "not forecast", which is 0 — rejecting the whole response over it would lose a valid
        // forecast to a field nobody reads.
        let cached = build(response(31.0, "2026-08-07T11:30"), &algiers(), now(), None).unwrap();
        assert_eq!(cached.payload.hours.len(), 2);
        assert_eq!(cached.payload.hours[0].precipitation_chance, 10.0);
        assert_eq!(cached.payload.hours[1].precipitation_chance, 0.0);
    }

    #[test]
    fn an_absent_hourly_block_is_not_a_rejection() {
        // The card's headline is the current reading; the graph is an addition. A publisher that
        // stopped sending hourly data should cost the graph, not the weather.
        let mut body = response(31.0, "2026-08-07T11:30");
        body.hourly = None;
        let cached = build(body, &algiers(), now(), None).unwrap();
        assert!(cached.payload.hours.is_empty());
        assert_eq!(cached.payload.temperature_c, 31.0);
    }

    #[test]
    fn an_impossible_hourly_temperature_is_refused_like_any_other() {
        // The hourly series gets the same bounds as everything else — it is the same sensor.
        let mut body = response(31.0, "2026-08-07T11:30");
        body.hourly.as_mut().unwrap().temperature_2m = vec![31.0, 900.0];
        assert!(build(body, &algiers(), now(), None).is_err());
    }

    #[test]
    fn a_plausible_response_is_accepted_with_the_publishers_time() {
        let cached = build(response(31.0, "2026-08-07T11:30"), &algiers(), now(), None).unwrap();
        assert_eq!(cached.payload.temperature_c, 31.0);
        assert_eq!(cached.payload.place, "16");
        assert_eq!(cached.source, "open-meteo");
        // Observed half an hour before we asked, and the age reflects that rather than the fetch.
        assert_eq!(cached.fetched_at, now());
        assert_eq!(cached.age(now()).as_secs(), 1_800);
    }

    #[test]
    fn an_impossible_temperature_is_refused() {
        // A sensor fault, or Fahrenheit arriving where Celsius was expected. Either way it must
        // never reach a card, because the card cannot tell.
        assert!(build(response(300.0, "2026-08-07T11:30"), &algiers(), now(), None).is_err());
        assert!(build(response(-80.0, "2026-08-07T11:30"), &algiers(), now(), None).is_err());
    }

    #[test]
    fn a_sudden_jump_is_held_against_the_previous_reading() {
        let first = build(response(31.0, "2026-08-07T11:00"), &algiers(), now(), None).unwrap();
        // Thirty degrees colder half an hour later is a fault anywhere on earth.
        let second = build(
            response(1.0, "2026-08-07T11:30"),
            &algiers(),
            now(),
            Some(&first.payload),
        );
        assert!(second.is_err(), "a 30° swing should be held");

        // An ordinary change passes.
        let normal = build(
            response(28.0, "2026-08-07T11:30"),
            &algiers(),
            now(),
            Some(&first.payload),
        );
        assert!(normal.is_ok());
    }

    #[test]
    fn the_movement_guard_works_near_freezing() {
        // A fractional guard divides by almost nothing at 0 °C and rejects every ordinary
        // reading, which is why this one is absolute.
        let first = build(response(0.5, "2026-08-07T11:00"), &algiers(), now(), None).unwrap();
        let second = build(
            response(2.5, "2026-08-07T11:30"),
            &algiers(),
            now(),
            Some(&first.payload),
        );
        assert!(second.is_ok(), "two degrees near zero is ordinary weather");
    }

    #[test]
    fn a_future_observation_is_refused() {
        // Trusting it would make the entry look permanently fresh and silence every staleness
        // check that follows.
        assert!(build(response(31.0, "2026-08-08T12:00"), &algiers(), now(), None).is_err());
    }

    #[test]
    fn misaligned_daily_series_are_caught() {
        // A low above its high means the arrays are out of step, which renders as a forecast
        // that looks merely odd rather than obviously broken.
        let mut body = response(31.0, "2026-08-07T11:30");
        body.daily.temperature_2m_min = vec![40.0, 41.0];
        assert!(build(body, &algiers(), now(), None).is_err());
    }

    #[test]
    fn an_empty_forecast_is_refused() {
        let mut body = response(31.0, "2026-08-07T11:30");
        body.daily.time.clear();
        body.daily.temperature_2m_max.clear();
        body.daily.temperature_2m_min.clear();
        body.daily.weather_code.clear();
        assert!(build(body, &algiers(), now(), None).is_err());
    }

    #[test]
    fn local_time_parsing_accounts_for_the_algerian_offset() {
        // Algeria is UTC+1 all year with no daylight saving, so one fixed offset is correct
        // rather than a simplification.
        let noon_local = parse_local_time("2026-08-07T12:00", 3_600).unwrap();
        let eleven_utc = xustive_tools::datetime::days_from_civil(2026, 8, 7) * 86_400 + 11 * 3600;
        assert_eq!(noon_local, eleven_utc);
        // The publisher's clock, not ours: with `timezone=auto` a Tokyo reading arrives in
        // Tokyo time, and reading it as Algiers time made it look nine hours old — which is how
        // every world city came back "implausible" the first time (2026-08-29).
        let tokyo = parse_local_time("2026-08-07T20:00", 9 * 3_600).unwrap();
        let algiers = parse_local_time("2026-08-07T12:00", 3_600).unwrap();
        assert_eq!(tokyo, algiers, "the same instant, two clocks");

        assert!(parse_local_time("nonsense", 3_600).is_none());
        assert!(parse_local_time("2026-08-07", 3_600).is_none());
    }

    #[test]
    fn every_wilaya_is_a_target() {
        assert_eq!(targets().len(), 58);
        assert!(world_targets().len() > 50, "the world list is worth having");
        assert!(
            targets().iter().all(|p| p.is_wilaya()),
            "the every-pass list is Algeria's"
        );
    }

    #[test]
    fn the_cadence_is_a_fixed_schedule_not_a_per_request_cost() {
        // 58 places every 30 minutes is 116 requests an hour — trivial for the publisher, and
        // identical whether one person searched or a million did. That is what makes a weather
        // search reveal nothing.
        let per_hour = 3600 / Weather.cadence().as_secs() * targets().len() as u64;
        assert_eq!(per_hour, 116);
        // The world list rides along every fourth pass, so its cost is a quarter of its size.
        let world_per_hour =
            3600 / (Weather.cadence().as_secs() * WORLD_EVERY) * world_targets().len() as u64;
        assert!(
            per_hour + world_per_hour < 400,
            "still trivial for the publisher: {per_hour} + {world_per_hour} an hour"
        );
        assert!(Weather.staleness_limit() > Weather.cadence() * 2);
    }
}
