//! Exchange rates (M8-T06.1).
//!
//! [[Instant Answers]] §5.1 calls currency Tier 1 and §7 calls it the single most Algeria-specific
//! thing the product does. It has never existed.
//!
//! ## Why not the ECB
//!
//! The obvious source is the European Central Bank's daily reference rates — authoritative, free,
//! self-hostable through Frankfurter, and the thing a European product would use without thinking.
//! **The ECB does not publish the dinar.** An Algerian search engine whose converter cannot do
//! `20 eur dzd` is a converter nobody here needs, so the ECB is the wrong primary no matter how
//! good it is.
//!
//! What is used instead publishes the dinar and the majors together, keyless, on one daily
//! timestamp. One source and one timestamp is also why there are no derived cross-rates here: a
//! dinar rate computed by multiplying two other rates carries the error of both and the `as_of` of
//! neither, and would have to be labelled as something other than what the card claims it is.
//!
//! ## What is deliberately absent
//!
//! The **parallel ("square") market rate**. [[Milestone 1B - Frontend and Instant Answers|M1B-T06.7]]
//! settled the rule: if no honest source exists, it ships disabled rather than invented. None does
//! — the square rate is quoted by no publisher we can verify — so the card names its rate as
//! official and says the other is missing for want of a source. A confident wrong number is the
//! failure [[Instant Answers]] §2 exists to prevent.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::validate::{self, Rejected};
use crate::{Cached, Dataset, FetchError};

pub struct Rates;

impl Dataset for Rates {
    fn key_prefix(&self) -> &'static str {
        "tool:rates:v1"
    }

    /// Reference rates are published once a working day. Fetching hourly would be twenty-three
    /// requests a day to be told the same number.
    fn cadence(&self) -> Duration {
        Duration::from_secs(6 * 3600)
    }

    /// Two days, so a weekend or a publisher holiday does not blank the card. Past that the rate
    /// is withheld rather than shown aged — the weather rule, for the same reason.
    fn staleness_limit(&self) -> Duration {
        Duration::from_secs(48 * 3600)
    }
}

/// The currencies worth storing.
///
/// A curated list rather than everything the publisher sends: the dinar, what Algerians actually
/// hold or receive, the neighbours, and the majors. Storing all 160 would multiply the entry for
/// currencies nobody here converts.
pub const CURRENCIES: &[&str] = &[
    "DZD", "EUR", "USD", "GBP", "CHF", "CAD", "TND", "MAD", "EGP", "SAR", "AED", "QAR", "KWD",
    "TRY", "CNY", "JPY", "RUB", "SEK", "NOK", "AUD",
];

/// Every stored rate, expressed per one unit of [`BASE`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RateTable {
    pub base: String,
    /// `(code, units of that currency per one base unit)`, in [`CURRENCIES`] order.
    pub rates: Vec<(String, f64)>,
}

/// The publisher quotes against the dollar, and the table is stored as it arrives. Converting to a
/// different base at write time would bake one rounding in for every reader; the card divides.
pub const BASE: &str = "USD";

impl RateTable {
    pub fn get(&self, code: &str) -> Option<f64> {
        self.rates
            .iter()
            .find(|(c, _)| c.eq_ignore_ascii_case(code))
            .map(|(_, v)| *v)
    }

    /// How many `to` one `from` buys.
    ///
    /// Both legs come from the same table and the same timestamp, so the cross-rate is exact
    /// arithmetic on one observation rather than two observations multiplied together.
    pub fn convert(&self, amount: f64, from: &str, to: &str) -> Option<f64> {
        let from_rate = self.get(from)?;
        let to_rate = self.get(to)?;
        if from_rate <= 0.0 {
            return None;
        }
        Some(amount / from_rate * to_rate)
    }
}

pub fn key(prefix: &str) -> String {
    prefix.to_string()
}

/// The open, keyless endpoint. No account, no per-request identification of who asked.
const ENDPOINT: &str = "https://open.er-api.com/v6/latest/USD";

#[derive(Debug, Deserialize)]
struct ApiResponse {
    result: String,
    #[serde(default)]
    time_last_update_unix: i64,
    base_code: String,
    rates: std::collections::HashMap<String, f64>,
}

pub async fn fetch(
    client: &reqwest::Client,
    now: i64,
    previous: Option<&RateTable>,
) -> Result<Cached<RateTable>, FetchError> {
    let response = client
        .get(ENDPOINT)
        .send()
        .await
        .map_err(|e| FetchError::Http(e.without_url().to_string()))?;
    if !response.status().is_success() {
        return Err(FetchError::Http(format!("status {}", response.status())));
    }
    let body: ApiResponse = response
        .json()
        .await
        .map_err(|e| FetchError::Parse(e.without_url().to_string()))?;
    build(body, now, previous)
}

/// Validate a response into a cache entry.
///
/// Separated from the request so every check is testable without a network — these are the checks
/// that only ever run against data we did not write.
fn build(
    body: ApiResponse,
    now: i64,
    previous: Option<&RateTable>,
) -> Result<Cached<RateTable>, FetchError> {
    let reject = |r: Rejected| FetchError::Rejected(r.to_string());

    if body.result != "success" {
        return Err(reject(Rejected::Missing {
            field: format!("result={}", body.result),
        }));
    }
    if !body.base_code.eq_ignore_ascii_case(BASE) {
        // The base silently changing would invert every rate on the card while every number still
        // looked plausible.
        return Err(reject(Rejected::Missing {
            field: format!("base_code={} (expected {BASE})", body.base_code),
        }));
    }

    let observed_at = body.time_last_update_unix;
    validate::timestamp(observed_at, now, Duration::from_secs(7 * 24 * 3600)).map_err(reject)?;

    let mut rates = Vec::with_capacity(CURRENCIES.len());
    for code in CURRENCIES {
        let Some(rate) = body.rates.get(*code) else {
            // A missing major is a broken response, not a thin one. Half a table renders as a
            // converter that silently cannot do the pair someone asked for.
            return Err(reject(Rejected::Missing {
                field: (*code).to_string(),
            }));
        };
        // No currency on earth trades at more than a few tens of thousands to the dollar, and none
        // trades at zero. A decimal slip lands outside this; an ordinary rate never does.
        validate::bounded(code, *rate, f64::MIN_POSITIVE, 100_000.0).map_err(reject)?;
        rates.push(((*code).to_string(), *rate));
    }

    // Fractional here, unlike temperature: a rate is a ratio with no meaningful zero, so a
    // proportional guard is the one that makes sense. Ten per cent in a day is a devaluation —
    // rare, real, and worth a human looking rather than a silent write.
    if let Some(previous) = previous {
        for (code, rate) in &rates {
            let Some(before) = previous.get(code) else {
                continue;
            };
            if before > 0.0 && ((rate - before).abs() / before) > 0.10 {
                return Err(reject(Rejected::Moved {
                    field: code.clone(),
                    from: before,
                    to: *rate,
                }));
            }
        }
    }

    Ok(Cached {
        fetched_at: now,
        observed_at,
        source: "exchangerate-api.com (open access)".into(),
        licence: "attribution required".into(),
        payload: RateTable {
            base: BASE.to_string(),
            rates,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(dzd: f64, at: i64) -> ApiResponse {
        let mut rates: std::collections::HashMap<String, f64> =
            CURRENCIES.iter().map(|c| ((*c).to_string(), 1.0)).collect();
        rates.insert("DZD".into(), dzd);
        rates.insert("EUR".into(), 0.92);
        rates.insert("USD".into(), 1.0);
        ApiResponse {
            result: "success".into(),
            time_last_update_unix: at,
            base_code: "USD".into(),
            rates,
        }
    }

    const NOW: i64 = 1_787_000_000;

    #[test]
    fn a_plausible_table_is_accepted_with_the_publishers_timestamp() {
        let cached = build(response(133.05, NOW - 3600), NOW, None).unwrap();
        assert_eq!(cached.observed_at, NOW - 3600);
        assert_eq!(cached.payload.get("DZD"), Some(133.05));
    }

    #[test]
    fn a_cross_rate_is_arithmetic_on_one_observation_not_two_multiplied() {
        // Both legs come from the same table and the same timestamp, which is the reason this
        // product does not derive the dinar from two separate publishers.
        let table = build(response(133.05, NOW), NOW, None).unwrap().payload;
        // 20 EUR at 0.92 EUR/USD and 133.05 DZD/USD.
        let dzd = table.convert(20.0, "EUR", "DZD").unwrap();
        assert!((dzd - 20.0 / 0.92 * 133.05).abs() < 1e-9);
    }

    #[test]
    fn a_missing_major_is_a_broken_response_rather_than_a_thin_one() {
        // Half a table renders as a converter that silently cannot do the pair someone asked for.
        let mut body = response(133.05, NOW);
        body.rates.remove("GBP");
        assert!(build(body, NOW, None).is_err());
    }

    #[test]
    fn a_changed_base_is_refused_because_every_rate_would_invert_while_looking_plausible() {
        let mut body = response(133.05, NOW);
        body.base_code = "EUR".into();
        assert!(build(body, NOW, None).is_err());
    }

    #[test]
    fn a_decimal_slip_is_out_of_bounds() {
        assert!(build(response(0.0, NOW), NOW, None).is_err());
        assert!(build(response(1_330_500.0, NOW), NOW, None).is_err());
    }

    #[test]
    fn a_ten_per_cent_overnight_move_is_held_for_a_human() {
        // Fractional rather than absolute, unlike temperature: a rate is a ratio with no
        // meaningful zero. A devaluation this size is real, rare, and worth looking at rather
        // than writing silently.
        let previous = build(response(133.0, NOW), NOW, None).unwrap().payload;
        assert!(build(response(160.0, NOW), NOW, Some(&previous)).is_err());
        // An ordinary daily drift passes.
        assert!(build(response(134.0, NOW), NOW, Some(&previous)).is_ok());
    }

    #[test]
    fn an_unsuccessful_result_field_is_refused_even_with_a_full_body() {
        // The publisher answers 200 with `result: "error"` rather than a status code.
        let mut body = response(133.05, NOW);
        body.result = "error".into();
        assert!(build(body, NOW, None).is_err());
    }

    #[test]
    fn an_unknown_currency_converts_to_nothing_rather_than_guessing() {
        let table = build(response(133.05, NOW), NOW, None).unwrap().payload;
        assert!(table.convert(10.0, "XYZ", "DZD").is_none());
        assert!(table.convert(10.0, "DZD", "XYZ").is_none());
    }
}
