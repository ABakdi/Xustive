//! The shared error taxonomy.
//!
//! Every crate defines its own `thiserror` enum, but each variant maps to exactly one
//! [`ErrorClass`]. The retry layer switches on the class — **never on a string**, and never on the
//! concrete error type. That is what stops a 404 from being retried four times.

use std::fmt;

/// How the system should react to a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorClass {
    /// Will probably succeed if tried again: 503, connection reset, timeout.
    Transient,
    /// Explicitly rate limited. Retry, but slower, and open a circuit breaker.
    Throttled,
    /// Will never succeed: 404, 410, malformed input, unsupported codec.
    Permanent,
    /// Reliably breaks the processor. Goes to a dead-letter queue and becomes a test fixture.
    Poison,
    /// An optional part failed. Shed it and serve the rest.
    Degraded,
    /// Unrecoverable at startup: missing model file, invalid config. Exit non-zero.
    Fatal,
}

impl ErrorClass {
    /// Whether the retry layer should try again at all.
    ///
    /// Note that `Poison` and `Permanent` are both non-retryable but differ in disposal:
    /// permanent errors are dropped, poison errors are preserved for investigation.
    pub const fn is_retryable(self) -> bool {
        matches!(self, Self::Transient | Self::Throttled)
    }

    /// Whether the message should be preserved in a dead-letter queue.
    pub const fn is_dead_letter(self) -> bool {
        matches!(self, Self::Poison)
    }

    /// Whether the process should stop.
    pub const fn is_fatal(self) -> bool {
        matches!(self, Self::Fatal)
    }

    /// Stable label for metrics. Bounded cardinality by construction.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Transient => "transient",
            Self::Throttled => "throttled",
            Self::Permanent => "permanent",
            Self::Poison => "poison",
            Self::Degraded => "degraded",
            Self::Fatal => "fatal",
        }
    }
}

impl fmt::Display for ErrorClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Implemented by every error enum in the workspace.
pub trait Classify {
    fn class(&self) -> ErrorClass;

    fn is_retryable(&self) -> bool {
        self.class().is_retryable()
    }
}

/// Classify an HTTP status the way the fetch and index layers agree on.
///
/// The `429` and `404` rows are the ones that matter: retrying either is actively harmful.
pub fn class_for_status(status: u16) -> ErrorClass {
    match status {
        200..=299 | 304 => ErrorClass::Degraded, // caller should not be classifying success
        408 | 425 => ErrorClass::Transient,
        429 => ErrorClass::Throttled,
        400..=499 => ErrorClass::Permanent,
        500..=599 => ErrorClass::Transient,
        _ => ErrorClass::Permanent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_policy_matches_the_spec() {
        assert!(ErrorClass::Transient.is_retryable());
        assert!(ErrorClass::Throttled.is_retryable());
        assert!(!ErrorClass::Permanent.is_retryable());
        assert!(!ErrorClass::Poison.is_retryable());
        assert!(!ErrorClass::Fatal.is_retryable());
    }

    #[test]
    fn permanent_statuses_are_never_retried() {
        for s in [400, 401, 403, 404, 410, 451] {
            assert_eq!(class_for_status(s), ErrorClass::Permanent, "status {s}");
            assert!(
                !class_for_status(s).is_retryable(),
                "status {s} must not retry"
            );
        }
    }

    #[test]
    fn rate_limit_is_throttled_not_transient() {
        // The distinction matters: throttled opens a circuit breaker, transient does not.
        assert_eq!(class_for_status(429), ErrorClass::Throttled);
    }

    #[test]
    fn server_errors_retry() {
        for s in [500, 502, 503, 504] {
            assert_eq!(class_for_status(s), ErrorClass::Transient, "status {s}");
        }
    }

    #[test]
    fn timeout_statuses_retry() {
        assert_eq!(class_for_status(408), ErrorClass::Transient);
    }

    #[test]
    fn only_poison_goes_to_dlq() {
        assert!(ErrorClass::Poison.is_dead_letter());
        assert!(!ErrorClass::Permanent.is_dead_letter());
    }
}
