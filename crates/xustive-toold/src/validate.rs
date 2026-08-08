//! Plausibility checks on fetched data.
//!
//! A bad write poisons every subsequent answer silently. The tool card has no way to know its
//! input was nonsense, and a temperature of 300 °C or a dinar rate of 4 to the euro renders with
//! exactly the same confidence as a correct one.
//!
//! Publishers do emit garbage — a sensor fault, a decimal-point slip, a maintenance page served
//! with a 200. So nothing is written without passing here, and a rejection **keeps the previous
//! value** rather than clearing it: slightly old and correct beats fresh and wrong.

use std::time::Duration;

/// Why a fetched value was refused.
#[derive(Debug, Clone, PartialEq)]
pub enum Rejected {
    /// Outside what the quantity can physically be.
    OutOfBounds {
        field: String,
        value: f64,
    },
    /// A plausible value that moved implausibly far since the last one.
    Moved {
        field: String,
        from: f64,
        to: f64,
    },
    /// The publisher's own timestamp is in the future or absurdly old.
    BadTimestamp {
        observed_at: i64,
    },
    Missing {
        field: String,
    },
}

impl Rejected {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OutOfBounds { .. } => "out_of_bounds",
            Self::Moved { .. } => "moved_too_far",
            Self::BadTimestamp { .. } => "bad_timestamp",
            Self::Missing { .. } => "missing_field",
        }
    }
}

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfBounds { field, value } => write!(f, "{field} = {value} is out of bounds"),
            Self::Moved { field, from, to } => write!(f, "{field} moved {from} → {to}"),
            Self::BadTimestamp { observed_at } => {
                write!(f, "observed_at {observed_at} is implausible")
            }
            Self::Missing { field } => write!(f, "{field} is missing"),
        }
    }
}

/// Bounds a value must fall inside.
pub fn bounded(field: &str, value: f64, min: f64, max: f64) -> Result<(), Rejected> {
    if !value.is_finite() || value < min || value > max {
        return Err(Rejected::OutOfBounds {
            field: field.to_string(),
            value,
        });
    }
    Ok(())
}

/// Reject a value that moved further than `max_fraction` since the last one.
///
/// Real moves that large exist — a currency can genuinely jump. So can a decimal-point error, and
/// the two are indistinguishable in a single reading. The cost of a slightly late correct value
/// is far below the cost of a confident wrong one, so the guard errs towards holding.
///
/// Skipped when there is no previous value, since the first reading has nothing to move from.
pub fn movement(
    field: &str,
    previous: Option<f64>,
    current: f64,
    max_fraction: f64,
) -> Result<(), Rejected> {
    let Some(previous) = previous else {
        return Ok(());
    };
    if previous == 0.0 || !previous.is_finite() {
        return Ok(());
    }
    let change = ((current - previous) / previous).abs();
    if change > max_fraction {
        return Err(Rejected::Moved {
            field: field.to_string(),
            from: previous,
            to: current,
        });
    }
    Ok(())
}

/// The publisher's own timestamp must be sane.
///
/// A future timestamp means a clock problem somewhere, and trusting it would make the value look
/// permanently fresh — the failure that hides every subsequent staleness check.
pub fn timestamp(observed_at: i64, now: i64, max_age: Duration) -> Result<(), Rejected> {
    // A few minutes of skew between machines is ordinary and not worth rejecting over.
    const SKEW: i64 = 300;
    if observed_at > now + SKEW || observed_at < now - max_age.as_secs() as i64 {
        return Err(Rejected::BadTimestamp { observed_at });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plausible_values_pass_and_absurd_ones_do_not() {
        assert!(bounded("temperature", 34.0, -20.0, 55.0).is_ok());
        // A sensor fault, or a unit confusion. Either way it must not reach a card.
        assert!(bounded("temperature", 300.0, -20.0, 55.0).is_err());
        assert!(bounded("temperature", -80.0, -20.0, 55.0).is_err());
    }

    #[test]
    fn nan_and_infinity_are_refused() {
        // JSON cannot carry them, but a computed field can produce one, and every comparison
        // against NaN is false — so a naive range check passes it.
        assert!(bounded("x", f64::NAN, 0.0, 100.0).is_err());
        assert!(bounded("x", f64::INFINITY, 0.0, 100.0).is_err());
    }

    #[test]
    fn a_decimal_point_slip_is_held() {
        // 145 → 1450 is a tenfold move. It is also exactly what a misplaced decimal looks like,
        // and a rate an order of magnitude wrong is the single most damaging thing this system
        // could display.
        assert!(movement("eur", Some(145.0), 1450.0, 0.25).is_err());
        // An ordinary daily move passes.
        assert!(movement("eur", Some(145.0), 152.0, 0.25).is_ok());
    }

    #[test]
    fn the_first_reading_has_nothing_to_move_from() {
        assert!(movement("eur", None, 145.0, 0.25).is_ok());
        // And a previous value of zero cannot be a denominator.
        assert!(movement("eur", Some(0.0), 145.0, 0.25).is_ok());
    }

    #[test]
    fn a_future_timestamp_is_refused() {
        // Trusting it would make the value look permanently fresh, which hides every subsequent
        // staleness check rather than triggering one.
        let now = 1_786_000_000;
        assert!(timestamp(now + 86_400, now, Duration::from_secs(7_200)).is_err());
        // A few minutes of clock skew between machines is ordinary.
        assert!(timestamp(now + 120, now, Duration::from_secs(7_200)).is_ok());
    }

    #[test]
    fn an_ancient_timestamp_is_refused() {
        let now = 1_786_000_000;
        assert!(timestamp(now - 86_400, now, Duration::from_secs(7_200)).is_err());
        assert!(timestamp(now - 3_600, now, Duration::from_secs(7_200)).is_ok());
    }

    #[test]
    fn every_rejection_has_a_stable_label_for_metrics() {
        // Labelled by reason, so a fetcher failing on bounds is distinguishable from one whose
        // publisher went down — those need different responses.
        let cases = [
            Rejected::OutOfBounds {
                field: "t".into(),
                value: 1.0,
            },
            Rejected::Moved {
                field: "t".into(),
                from: 1.0,
                to: 2.0,
            },
            Rejected::BadTimestamp { observed_at: 0 },
            Rejected::Missing { field: "t".into() },
        ];
        let labels: Vec<&str> = cases.iter().map(Rejected::as_str).collect();
        assert_eq!(labels.len(), 4);
        for label in &labels {
            assert!(!label.is_empty() && !label.contains(' '), "{label:?}");
        }
        // And each renders a message naming what went wrong.
        for case in &cases {
            assert!(!case.to_string().is_empty());
        }
    }
}
