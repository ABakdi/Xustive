//! Unit-aware evaluation (M8-T07).
//!
//! The decimal calculator answers arithmetic exactly and the unit converter answers single
//! conversions, and there was no bridge between them: `5 km + 3 miles` was not a question either
//! could take, and `20 eur + 5 usd in dzd` was not a question anyone could ask.
//!
//! `fend-core` is the engine that closes that gap — arbitrary precision, unit-aware, MIT, and with
//! no dependencies of its own, which is why it clears `cargo deny` without argument.
//!
//! **It evaluates; it does not decide.** Matching, confidence and localisation stay where they
//! were, because those are the parts that decide whether a card appears at all and they are
//! already well tested. This is a fallback: the decimal parser runs first and keeps its exact
//! results, and only what it cannot handle reaches here.
//!
//! Bounded on purpose. A calculator is an arbitrary-expression evaluator facing the open internet,
//! so it gets an interrupt, a length cap, and no access to anything but arithmetic.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Longest expression worth evaluating.
///
/// A person types tens of characters. Hundreds is either a paste that is not a question or an
/// attempt to find a pathological input, and neither deserves the CPU.
pub const MAX_LEN: usize = 200;

/// How long an evaluation may run before it is cut off.
///
/// fend is fast on anything a person types; this exists for the expressions nobody types, like a
/// tower of exponents that would otherwise compute until the request times out.
const BUDGET: std::time::Duration = std::time::Duration::from_millis(120);

/// An exchange rate, as fend asks for it: units of the currency per one US dollar.
pub type Rate = Box<dyn Fn(&str) -> Option<f64> + Send + Sync>;

/// Adapts our simple lookup to the trait fend asks for.
///
/// The `options.is_preview()` hint is ignored on purpose: every rate here is already a cached
/// value read out of Redis by the caller, so there is no slow path for a preview to avoid.
struct Rates(Rate);

impl fend_core::ExchangeRateFnV2 for Rates {
    fn relative_to_base_currency(
        &self,
        currency: &str,
        _options: &fend_core::ExchangeRateFnV2Options,
    ) -> Result<f64, Box<dyn std::error::Error + Send + Sync + 'static>> {
        (self.0)(currency).ok_or_else(|| format!("no rate for {currency}").into())
    }
}

/// Stops evaluation once the budget is spent.
struct Deadline {
    stop: Arc<AtomicBool>,
}

impl fend_core::Interrupt for Deadline {
    fn should_interrupt(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
}

/// Evaluate an expression, optionally with currency support.
///
/// Returns `None` for anything fend cannot make sense of, which is the common case — most queries
/// are not expressions, and a calculator that answers them anyway is the false activation the
/// matcher corpus exists to prevent.
pub fn evaluate(expr: &str, rates: Option<Rate>) -> Option<String> {
    let expr = expr.trim();
    if expr.is_empty() || expr.len() > MAX_LEN {
        return None;
    }

    let mut context = fend_core::Context::new();
    // No ambient state: every evaluation starts clean, so one reader's expression cannot define a
    // variable another reader's expression then sees.
    if let Some(rates) = rates {
        context.set_exchange_rate_handler_v2(Rates(rates));
    }

    let stop = Arc::new(AtomicBool::new(false));
    let interrupt = Deadline { stop: stop.clone() };
    // A plain timer thread rather than an async timeout: `evaluate_with_interrupt` is synchronous,
    // and this is the mechanism fend itself documents for bounding it.
    let timer = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(BUDGET);
            stop.store(true, Ordering::Relaxed);
        })
    };

    let result = fend_core::evaluate_with_interrupt(expr, &mut context, &interrupt);
    stop.store(true, Ordering::Relaxed);
    let _ = timer.join();

    let result = result.ok()?;
    let main = result.get_main_result().trim().to_string();
    if main.is_empty() {
        return None;
    }
    // fend echoes an input it could not reduce — `hello` evaluates to `hello`. That is not an
    // answer, and rendering it would put a card under every one-word search.
    if main.eq_ignore_ascii_case(expr) {
        return None;
    }
    // Complex results are refused, upholding a decision the decimal calculator already made:
    // `sqrt(-4)` answered nothing rather than `2i`, because for a general-audience search engine
    // an imaginary number is not an answer — it is a different question. A test pinned that, and
    // adopting a more capable engine is not a reason to quietly change it.
    if is_complex(&main) {
        return None;
    }
    Some(main)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_bridges_units_the_way_neither_existing_tool_could() {
        // The whole reason for adopting an engine: arithmetic across mixed units.
        let out = evaluate("5 km + 3 miles in m", None).unwrap();
        assert!(out.starts_with("9828"), "got {out}");
    }

    #[test]
    fn it_handles_bases_and_constants() {
        assert_eq!(evaluate("0x1f in decimal", None).as_deref(), Some("31"));
        assert!(evaluate("c in m/s", None).is_some());
    }

    #[test]
    fn it_keeps_precision_where_a_float_would_not() {
        // The property the decimal parser was chosen for, preserved by the fallback.
        assert_eq!(evaluate("0.1 + 0.2", None).as_deref(), Some("0.3"));
    }

    #[test]
    fn currency_works_only_when_rates_are_supplied() {
        // Without a rate handler the engine must not invent one. A made-up exchange rate is the
        // exact failure Instant Answers §2 exists to prevent.
        assert!(evaluate("20 EUR to USD", None).is_none());

        let rates: Rate = Box::new(|code| match code {
            "EUR" => Some(0.9),
            "DZD" => Some(133.0),
            _ => None,
        });
        let out = evaluate("20 EUR to DZD", Some(rates)).unwrap();
        // fend marks a non-terminating result as approximate, which is honest and is kept.
        assert!(out.contains("2955"), "got {out}");
        assert!(out.contains("DZD"), "the unit must survive: got {out}");
    }

    #[test]
    fn an_input_it_cannot_reduce_is_not_an_answer() {
        // fend echoes what it cannot evaluate. Returning that would put a calculator card under
        // every one-word search.
        assert!(evaluate("hello", None).is_none());
        assert!(evaluate("oran", None).is_none());
    }

    #[test]
    fn a_complex_result_is_refused_because_the_calculator_already_decided_that() {
        // sqrt(-4) answered nothing before this engine arrived, deliberately: for a
        // general-audience search engine an imaginary number is a different question, not an
        // answer. Adopting a more capable engine is not a reason to change that quietly.
        assert!(evaluate("sqrt(-4)", None).is_none());
        assert!(evaluate("(-1)^0.5", None).is_none());

        // The guard is narrow: it must not eat ordinary results that merely contain the letter.
        assert!(is_complex("approx. 0 + 2i"));
        assert!(is_complex("2i"));
        assert!(is_complex("approx. 0 + i"));
        assert!(is_complex("i"));
        assert!(!is_complex("3.218688 km"));
        assert!(!is_complex("5 in"));
        assert!(!is_complex("12 min"));
        assert!(!is_complex("1.5 mi"));
        assert!(evaluate("2 mi in km", None).is_some());
    }

    #[test]
    fn an_over_long_expression_is_refused_before_it_is_parsed() {
        assert!(evaluate(&"1+".repeat(200), None).is_none());
    }

    #[test]
    fn a_pathological_expression_is_cut_off_rather_than_running_forever() {
        // The reason the interrupt exists. Whatever this returns, it must return.
        let started = std::time::Instant::now();
        let _ = evaluate("10^10^10^10", None);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(3),
            "evaluation was not bounded"
        );
    }

    #[test]
    fn each_evaluation_starts_clean() {
        // No ambient state between calls: one reader's expression must not define a variable that
        // another reader's expression can then see.
        let _ = evaluate("x = 5", None);
        assert!(evaluate("x + 1", None).is_none());
    }
}

/// Whether fend rendered a complex number.
///
/// Four shapes to catch, because fend omits a unit coefficient of one: `2i`, `approx. 0 + 2i`,
/// `approx. 0 + i`, and a bare `i`. Detected as an `i` that both ends a term and begins one —
/// preceded by a digit or by nothing but a separator. No unit name is a bare `i`, which is what
/// keeps this from eating `5 in`, `12 min` or `1.5 mi`.
fn is_complex(rendered: &str) -> bool {
    let bytes = rendered.as_bytes();
    rendered.char_indices().any(|(idx, ch)| {
        if ch != 'i' {
            return false;
        }
        let ends_term = bytes
            .get(idx + 1)
            .is_none_or(|b| !b.is_ascii_alphanumeric());
        if !ends_term {
            return false;
        }
        match idx.checked_sub(1).map(|i| bytes[i]) {
            None => true,
            Some(b) if b.is_ascii_digit() => true,
            // `0 + i`: a space is only a term boundary when what precedes it is an operator, so
            // `5 in` and `1.5 mi` — where the letter belongs to a unit name — stay untouched.
            Some(b' ') => matches!(bytes.get(idx.wrapping_sub(2)), Some(b'+' | b'-')),
            _ => false,
        }
    })
}
