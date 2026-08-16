//! Challenge and silent-cloaking detection ([[Session Manager]] §4.6).
//!
//! Platforms increasingly *cloak* rather than error: HTTP 200, valid HTML, zero results. A connector
//! that trusts status codes reports success while collecting nothing. So an outcome is classified
//! not only by what it says but against two pieces of context: how many empties this identity has
//! seen in a row, and whether the **canary** — a known-stable public object fetched by a low-value
//! identity — still returns content. The canary is the ground truth that separates "we are being
//! cloaked" (canary has content, we do not) from "the platform changed" (canary is empty too, and
//! it is a code problem, not a ban).

use super::{ChallengeKind, SessionOutcome};

/// What to do about an outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detection {
    /// Content delivered — carry on, and reset the empty streak.
    Healthy,
    /// A transient network error — not the identity's fault; retry.
    Transient,
    /// Quarantine this identity: an explicit rate limit, captcha, checkpoint, or ban.
    Quarantine,
    /// A soft ban — repeated empties while the canary still has content. Quarantine, but for the
    /// cloaking reason, which is worth telling apart from an explicit challenge.
    SoftBanQuarantine,
    /// A login wall on content that used to be anonymous — retry the work as a logged-in identity
    /// rather than quarantining a healthy one.
    DowngradeCapability,
    /// Empties *and* the canary is also empty: the platform changed. Alert a human — this is a code
    /// problem, and quarantining identities for it would burn a healthy pool for nothing.
    PlatformChanged,
    /// A single empty below the streak threshold — keep going, but the caller should note it.
    WatchEmpty,
}

/// Classify one outcome. `consecutive_empty` is the identity's empty streak **including this
/// outcome** if it is empty (the caller maintains it: reset on content, increment on empty).
/// `canary_has_content` is the latest canary reading for the platform; `empty_threshold` is the
/// `consecutive_empty` at which repeated empties become a soft-ban or platform-change signal.
pub fn classify(
    outcome: SessionOutcome,
    consecutive_empty: u32,
    canary_has_content: bool,
    empty_threshold: u32,
) -> Detection {
    match outcome {
        SessionOutcome::Ok { items, .. } if items > 0 => Detection::Healthy,
        SessionOutcome::NetworkError => Detection::Transient,
        SessionOutcome::RateLimited | SessionOutcome::Banned => Detection::Quarantine,
        SessionOutcome::Challenge(ChallengeKind::LoginWall) => Detection::DowngradeCapability,
        SessionOutcome::Challenge(_) => Detection::Quarantine,
        // Both `Empty` and a 200 with zero items are the cloaking signal.
        SessionOutcome::Empty | SessionOutcome::Ok { .. } => {
            if consecutive_empty >= empty_threshold {
                // Enough empties in a row to act. The canary decides which way.
                if canary_has_content {
                    Detection::SoftBanQuarantine
                } else {
                    Detection::PlatformChanged
                }
            } else {
                Detection::WatchEmpty
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_is_healthy() {
        assert_eq!(
            classify(
                SessionOutcome::Ok {
                    bytes: 10,
                    items: 5
                },
                0,
                true,
                3
            ),
            Detection::Healthy
        );
    }

    #[test]
    fn explicit_challenges_quarantine() {
        assert_eq!(
            classify(SessionOutcome::RateLimited, 0, true, 3),
            Detection::Quarantine
        );
        assert_eq!(
            classify(SessionOutcome::Banned, 0, true, 3),
            Detection::Quarantine
        );
        assert_eq!(
            classify(
                SessionOutcome::Challenge(ChallengeKind::Captcha),
                0,
                true,
                3
            ),
            Detection::Quarantine
        );
        assert_eq!(
            classify(
                SessionOutcome::Challenge(ChallengeKind::Checkpoint),
                0,
                true,
                3
            ),
            Detection::Quarantine
        );
    }

    #[test]
    fn a_login_wall_downgrades_rather_than_quarantines() {
        assert_eq!(
            classify(
                SessionOutcome::Challenge(ChallengeKind::LoginWall),
                0,
                true,
                3
            ),
            Detection::DowngradeCapability
        );
    }

    #[test]
    fn a_network_error_is_transient_not_the_identitys_fault() {
        assert_eq!(
            classify(SessionOutcome::NetworkError, 0, true, 3),
            Detection::Transient
        );
    }

    #[test]
    fn one_empty_below_the_threshold_is_only_watched() {
        assert_eq!(
            classify(SessionOutcome::Empty, 1, true, 3),
            Detection::WatchEmpty
        );
        assert_eq!(
            classify(SessionOutcome::Empty, 2, true, 3),
            Detection::WatchEmpty
        );
    }

    #[test]
    fn repeated_empties_with_a_live_canary_are_a_soft_ban() {
        // Three empties in a row, canary still returns content → we are being cloaked.
        assert_eq!(
            classify(SessionOutcome::Empty, 3, true, 3),
            Detection::SoftBanQuarantine
        );
        // A 200 with zero items counts the same as an explicit Empty.
        assert_eq!(
            classify(
                SessionOutcome::Ok {
                    bytes: 500,
                    items: 0
                },
                3,
                true,
                3
            ),
            Detection::SoftBanQuarantine
        );
    }

    #[test]
    fn repeated_empties_with_a_dead_canary_mean_the_platform_changed() {
        // Canary is empty too → not a ban, a code problem. Do not burn the pool for it.
        assert_eq!(
            classify(SessionOutcome::Empty, 5, false, 3),
            Detection::PlatformChanged
        );
    }
}
