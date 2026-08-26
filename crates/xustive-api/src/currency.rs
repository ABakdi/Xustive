//! The currency card (M8-T06).
//!
//! Detection is pure and lives in `xustive-tools`; the answer is built here, where the rate cache
//! is. The same split as weather, and for the same reason: a matcher that reached for Redis would
//! put a round trip on every search that is not about money.
//!
//! **Cache only, never a fetch.** The serving plane has no route to the internet, and a cold cache
//! means no card, which is correct rather than unfortunate.

use xustive_toold::rates::{self, RateTable, Rates};
use xustive_toold::Dataset;

use crate::state::AppState;

/// Build a currency answer, or nothing.
pub async fn answer(state: &AppState, query: &str, ui_lang: &str) -> Option<xustive_tools::Answer> {
    let request = xustive_tools::currency::detect(query)?;
    let cache = state.tool_cache.as_ref()?;

    let cached = cache
        .get::<RateTable>(&rates::key(Rates.key_prefix()))
        .await
        .ok()
        .flatten()?;

    let now = xustive_core::now_unix();
    // Past the limit the rate is withheld rather than shown aged. A stale rate presented as
    // current is the exact failure [[Instant Answers]] §2 exists to prevent, and unlike a stale
    // temperature it is one someone might act on with money.
    if cached.is_stale(now, Rates.staleness_limit()) {
        tracing::debug!("rates are stale; withholding");
        return None;
    }

    let table = &cached.payload;
    let converted = table.convert(request.amount, &request.from, &request.to)?;

    // Two decimals for a normal amount; more when the result is small enough that two would round
    // it to nothing. `1 DZD in EUR` is 0.0075, and `0.01` is not an answer.
    let decimals = if converted.abs() >= 1.0 { 2 } else { 6 };
    let value = format!("{converted:.decimals$}");
    let unit_rate = table.convert(1.0, &request.from, &request.to)?;

    Some(xustive_tools::Answer {
        tool: "currency",
        confidence: request.confidence,
        interpretation: format!(
            "{} {} → {}",
            trim_amount(request.amount),
            request.from,
            request.to
        ),
        value: format!("{value} {}", request.to),
        detail: Some(serde_json::json!({
            "amount": request.amount,
            "from": request.from,
            "to": request.to,
            "converted": converted,
            // The single-unit rate, so a reader can sanity-check the arithmetic without doing it.
            "unit_rate": unit_rate,
            // Named as official, and paired with the note below. The card must never let a reader
            // assume this is what a bureau on the street would give them.
            "rate_kind": "official",
            "parallel_available": false,
            "source": cached.source,
            "licence": cached.licence,
            "ui_lang": ui_lang,
        })),
        as_of: Some(cached.observed_at),
    })
}

/// `20` rather than `20.00`, but `1.5` rather than `2`.
fn trim_amount(n: f64) -> String {
    if (n.fract()).abs() < f64::EPSILON {
        format!("{n:.0}")
    } else {
        format!("{n}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_amount_renders_without_pointless_decimals() {
        assert_eq!(trim_amount(20.0), "20");
        assert_eq!(trim_amount(1.5), "1.5");
        assert_eq!(trim_amount(1500.0), "1500");
    }
}
