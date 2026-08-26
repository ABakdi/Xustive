//! The weather answer.
//!
//! The detector in `xustive_tools::weather` is pure and decides *whether* a query is about
//! weather. This fills in the answer from the cache — which is the serving plane's job, because
//! it is the only side with a cache handle and because a matcher that did I/O would put a Redis
//! round trip on every search that is not about weather.
//!
//! **Cache only, never a fetch.** The serving plane has no route to the internet, and that
//! constraint is worth more than any card. A cold cache means no weather card, which is correct.

use std::time::Duration;

use xustive_toold::weather::{key, Forecast, Weather};
use xustive_toold::Dataset;

use crate::state::AppState;

/// Build a weather answer, or nothing.
pub async fn answer(
    state: &AppState,
    query: &str,
    ui_lang: &str,
    peer: Option<std::net::SocketAddr>,
) -> Option<xustive_tools::Answer> {
    let mut request = xustive_tools::weather::detect(query)?;
    let cache = state.tool_cache.as_ref()?;

    // Nobody named a place, so guess one from where the reader is connecting from — a local
    // database lookup that never leaves the process, and whose result is coarsened to a wilaya
    // before it travels anywhere ([[ADR-0020]]). Naming a place always wins over guessing.
    let assumed = !request.named;
    if !request.named {
        if let Some(geo) = state.geoip.as_ref() {
            if let Some(w) = crate::ratelimit::client_ip(peer).and_then(|ip| geo.wilaya_of(ip)) {
                request.wilaya = w;
            }
        }
    }

    let cached = cache
        .get::<Forecast>(&key(Weather.key_prefix(), request.wilaya.code))
        .await
        .ok()
        .flatten()?;

    let now = xustive_core::now_unix();
    // Past the limit the value is withheld entirely rather than shown with a caveat. Three hours
    // is already generous for a current temperature, and a card that shows yesterday's weather
    // with a note is still a card showing yesterday's weather.
    if cached.is_stale(now, Weather.staleness_limit()) {
        tracing::debug!(
            wilaya = request.wilaya.code,
            "weather is stale; withholding"
        );
        return None;
    }

    let f = &cached.payload;
    let place = match ui_lang {
        "fr" | "en" => request.wilaya.name_fr,
        _ => request.wilaya.name_ar,
    };

    let days: Vec<serde_json::Value> = f
        .days
        .iter()
        .map(|d| {
            serde_json::json!({
                "date": d.date,
                "high": d.high_c.round(),
                "low": d.low_c.round(),
                "code": d.code,
            })
        })
        .collect();

    // The hourly series, rounded on the way out. The card draws it as a graph; a client that
    // ignores it loses nothing, which is why an absent series is not an error anywhere.
    let hours: Vec<serde_json::Value> = f
        .hours
        .iter()
        .map(|h| {
            serde_json::json!({
                "time": h.time,
                "temperature": h.temperature_c.round(),
                "precipitation_chance": h.precipitation_chance.round(),
                "code": h.code,
            })
        })
        .collect();

    Some(xustive_tools::Answer {
        tool: "weather",
        confidence: request.confidence,
        interpretation: place.to_string(),
        value: format!("{}°", f.temperature_c.round()),
        detail: Some(serde_json::json!({
            "wilaya": { "code": request.wilaya.code, "ar": request.wilaya.name_ar, "fr": request.wilaya.name_fr },
            "temperature": f.temperature_c.round(),
            "feels_like": f.feels_like_c.round(),
            "code": f.code,
            "wind_kmh": f.wind_kmh.round(),
            "humidity": f.humidity.round(),
            // Whether the place was named or guessed. The card says which, because a wrong city
            // stated confidently is the failure this whole path has to avoid (M8-T05.6).
            "assumed_place": assumed,
            "days": days,
            "hours": hours,
            // Carried into the card. A reader cannot judge a number we did not measure ourselves
            // without knowing who did.
            "source": cached.source,
            "licence": cached.licence,
            "age_seconds": cached.age(now).as_secs(),
        })),
        // The publisher's measurement time, not our fetch. A value fetched a minute ago and
        // measured two hours ago is two hours old.
        as_of: Some(cached.observed_at),
    })
}

/// How long a weather answer may be before it is withheld. Exposed for tests and the admin page.
pub fn staleness_limit() -> Duration {
    Weather.staleness_limit()
}
