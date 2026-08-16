//! Proxy health scoring and the state machine ([[Proxy Manager]] §4.4).
//!
//! Each proxy carries EWMA rates for the four things that predict whether it will work: success,
//! latency, blocks, challenges. The composite score decides eligibility; consecutive failures and
//! quarantines decide when to pull it. The point of the EWMA rather than raw counts is that a proxy
//! recovers: a bad hour a week ago should not exclude it today.

use serde::{Deserialize, Serialize};

use super::Outcome;

/// EWMA smoothing factor. A new sample is 25 % of the updated rate, so roughly the last handful of
/// requests dominate — responsive enough to catch a proxy going bad within a few calls, damped
/// enough that one blip does not quarantine a good proxy.
const ALPHA: f64 = 0.25;

/// Latency at or below this is treated as ideal (normalised 0). Below the added-latency budget for
/// residential (§8), so a healthy proxy scores full marks on latency.
pub const MIN_LATENCY_MS: f64 = 200.0;
/// Latency at or above this is treated as worst (normalised 1). Past this a proxy is effectively
/// timing out for scoring purposes even if the request eventually returns.
const MAX_LATENCY_MS: f64 = 5_000.0;

/// Consecutive outright failures that force a quarantine (§4.4).
const QUARANTINE_AFTER_FAILURES: u32 = 3;
/// Consecutive quarantines that kill a proxy for good (§4.4).
const DEAD_AFTER_QUARANTINES: u32 = 5;

/// Eligibility state ([[Proxy Manager]] §4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProxyState {
    /// Score ≥ 0.7 — eligible for everything, including new pinning.
    Healthy,
    /// Score 0.4–0.7 — eligible at reduced weight, but **not for new pinning**.
    Degraded,
    /// Score < 0.4 or a failure streak — excluded, then probed after the cooldown.
    Quarantined,
    /// Five consecutive quarantines — removed; alert.
    Dead,
}

/// The rolling health of one proxy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Health {
    success_rate: f64,
    /// Normalised latency in `0.0..=1.0` (0 = fast, 1 = slow). Stored normalised so the score is a
    /// plain weighted sum.
    latency_norm: f64,
    block_rate: f64,
    challenge_rate: f64,
    /// Failures since the last success. Resets to 0 on any `Ok`.
    consecutive_failures: u32,
    /// Quarantines since the last time the proxy was healthy. Resets when it recovers.
    consecutive_quarantines: u32,
    state: ProxyState,
}

impl Default for Health {
    /// A fresh proxy starts *healthy* with optimistic rates. It has to earn quarantine, not earn its
    /// way in — a new proxy no one has tried is eligible, and the first failures adjust it fast.
    fn default() -> Self {
        Self {
            success_rate: 1.0,
            latency_norm: 0.0,
            block_rate: 0.0,
            challenge_rate: 0.0,
            consecutive_failures: 0,
            consecutive_quarantines: 0,
            state: ProxyState::Healthy,
        }
    }
}

impl Health {
    /// The composite score in `0.0..=1.0` (§4.4):
    /// `0.40·success + 0.20·(1−latency) + 0.30·(1−block) + 0.10·(1−challenge)`.
    pub fn score(&self) -> f64 {
        0.40 * self.success_rate
            + 0.20 * (1.0 - self.latency_norm)
            + 0.30 * (1.0 - self.block_rate)
            + 0.10 * (1.0 - self.challenge_rate)
    }

    pub fn state(&self) -> ProxyState {
        self.state
    }

    /// Eligible to be selected at all — healthy or degraded, never quarantined or dead.
    pub fn is_eligible(&self) -> bool {
        matches!(self.state, ProxyState::Healthy | ProxyState::Degraded)
    }

    /// Eligible to receive a *new* pin. Only a healthy proxy is — a degraded one might be about to
    /// quarantine, and pinning an identity to it invites an immediate reassignment (§4.4).
    pub fn is_pinnable(&self) -> bool {
        self.state == ProxyState::Healthy
    }

    /// Fold one request outcome into the rolling health, then recompute the state. `latency_ms` is
    /// the observed round-trip; pass the timeout budget for a `Timeout`.
    pub fn observe(&mut self, outcome: Outcome, latency_ms: f64) {
        let success = if outcome.is_success() { 1.0 } else { 0.0 };
        let block = matches!(outcome, Outcome::Blocked | Outcome::Banned) as u8 as f64;
        let challenge = matches!(outcome, Outcome::Challenged) as u8 as f64;

        self.success_rate = ewma(self.success_rate, success);
        self.block_rate = ewma(self.block_rate, block);
        self.challenge_rate = ewma(self.challenge_rate, challenge);
        self.latency_norm = ewma(self.latency_norm, normalise_latency(latency_ms));

        if outcome.is_success() {
            self.consecutive_failures = 0;
        } else {
            self.consecutive_failures += 1;
        }

        self.recompute_state();
    }

    /// Return a quarantined proxy to service for a probe. Called after the quarantine cooldown; the
    /// probe's outcome then decides whether it recovers or quarantines again.
    pub fn begin_probe(&mut self) {
        if self.state == ProxyState::Quarantined {
            // Give it a clean streak to prove itself, but do not reset the *quarantine* count — five
            // in a row still kills it however many probes are interleaved.
            self.consecutive_failures = 0;
            self.state = ProxyState::Degraded;
        }
    }

    fn recompute_state(&mut self) {
        // A failure streak quarantines regardless of the smoothed score — three straight refusals is
        // a dead proxy the EWMA has not caught up to yet.
        let quarantine =
            self.score() < 0.4 || self.consecutive_failures >= QUARANTINE_AFTER_FAILURES;

        if quarantine {
            // Only count a *new* entry into quarantine, so a proxy sitting quarantined does not tick
            // toward dead on every further failure.
            if self.state != ProxyState::Quarantined && self.state != ProxyState::Dead {
                self.consecutive_quarantines += 1;
            }
            self.state = if self.consecutive_quarantines >= DEAD_AFTER_QUARANTINES {
                ProxyState::Dead
            } else {
                ProxyState::Quarantined
            };
            return;
        }

        // Recovered enough to be eligible again: reset the quarantine streak once genuinely healthy.
        self.state = if self.score() >= 0.7 {
            self.consecutive_quarantines = 0;
            ProxyState::Healthy
        } else {
            ProxyState::Degraded
        };
    }
}

fn ewma(current: f64, sample: f64) -> f64 {
    ALPHA * sample + (1.0 - ALPHA) * current
}

/// Map a latency in ms to `0.0..=1.0`. `MIN_LATENCY_MS` and below is 0; `MAX_LATENCY_MS` and above
/// is 1; linear between.
fn normalise_latency(ms: f64) -> f64 {
    ((ms - MIN_LATENCY_MS) / (MAX_LATENCY_MS - MIN_LATENCY_MS)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_proxy_is_healthy_and_pinnable() {
        let h = Health::default();
        assert_eq!(h.state(), ProxyState::Healthy);
        assert!(h.is_pinnable());
        assert!(h.score() > 0.95);
    }

    #[test]
    fn three_consecutive_failures_quarantine_even_before_the_ewma_catches_up() {
        let mut h = Health::default();
        h.observe(Outcome::Timeout, 5000.0);
        h.observe(Outcome::Refused, 5000.0);
        assert_ne!(
            h.state(),
            ProxyState::Quarantined,
            "two failures is not yet a quarantine"
        );
        h.observe(Outcome::Refused, 5000.0);
        assert_eq!(
            h.state(),
            ProxyState::Quarantined,
            "the third failure quarantines"
        );
        assert!(!h.is_eligible());
    }

    #[test]
    fn a_success_resets_the_failure_streak() {
        let mut h = Health::default();
        h.observe(Outcome::Timeout, 5000.0);
        h.observe(Outcome::Timeout, 5000.0);
        h.observe(Outcome::Ok, 150.0);
        h.observe(Outcome::Timeout, 5000.0);
        // Only one failure since the last success, so no quarantine from the streak.
        assert_ne!(h.state(), ProxyState::Quarantined);
    }

    #[test]
    fn a_high_block_rate_keeps_a_proxy_out_of_healthy_without_a_failure_streak() {
        let mut h = Health::default();
        // Two blocks then a success, repeating: the streak never reaches 3 (the success resets it),
        // so this isolates the *score* path — the block-rate weight alone must keep it below the
        // healthy threshold. A proxy failing two of every three requests is not one to pin to.
        for _ in 0..10 {
            h.observe(Outcome::Blocked, 200.0);
            h.observe(Outcome::Blocked, 200.0);
            h.observe(Outcome::Ok, 200.0);
        }
        assert_ne!(
            h.state(),
            ProxyState::Quarantined,
            "no 3-failure streak occurred"
        );
        assert!(
            h.score() < 0.7,
            "a two-in-three block rate must depress the score below healthy: {}",
            h.score()
        );
        assert!(
            !h.is_pinnable(),
            "and it must not be eligible for new pinning"
        );
    }

    #[test]
    fn five_quarantines_kill_the_proxy() {
        let mut h = Health::default();
        for _ in 0..DEAD_AFTER_QUARANTINES {
            // Drive into quarantine…
            h.observe(Outcome::Refused, 5000.0);
            h.observe(Outcome::Refused, 5000.0);
            h.observe(Outcome::Refused, 5000.0);
            assert!(matches!(
                h.state(),
                ProxyState::Quarantined | ProxyState::Dead
            ));
            // …then probe it back out, only to fail again next loop.
            h.begin_probe();
        }
        assert_eq!(h.state(), ProxyState::Dead);
        assert!(!h.is_eligible());
    }

    #[test]
    fn a_probe_makes_a_quarantined_proxy_eligible_again_at_reduced_standing() {
        let mut h = Health::default();
        h.observe(Outcome::Refused, 5000.0);
        h.observe(Outcome::Refused, 5000.0);
        h.observe(Outcome::Refused, 5000.0);
        assert_eq!(h.state(), ProxyState::Quarantined);
        h.begin_probe();
        assert!(h.is_eligible(), "a probing proxy is eligible");
        assert!(
            !h.is_pinnable(),
            "but not for new pinning until fully healthy"
        );
    }

    #[test]
    fn recovery_after_a_run_of_successes_returns_to_healthy_and_clears_the_streak() {
        let mut h = Health::default();
        h.observe(Outcome::Refused, 5000.0);
        h.observe(Outcome::Refused, 5000.0);
        h.observe(Outcome::Refused, 5000.0);
        h.begin_probe();
        for _ in 0..20 {
            h.observe(Outcome::Ok, 150.0);
        }
        assert_eq!(h.state(), ProxyState::Healthy);
        assert!(h.is_pinnable());
    }
}
