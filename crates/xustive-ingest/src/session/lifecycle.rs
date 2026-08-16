//! Identity lifecycle and quarantine/recovery ([[Session Manager]] §4.3, §4.7).
//!
//! An identity moves `fresh → warming → mature`, and a challenge or ban drops it to `quarantined`.
//! Recovery returns it to `warming` at a reduced budget; a third quarantine burns it for good. The
//! cooldown between quarantine and a recovery attempt doubles, so an identity that keeps tripping
//! rests longer each time rather than being retried into a permanent ban.
//!
//! The tier is what gates budget: `fresh` scrapes nothing (warm-up traffic only), `warming` gets a
//! fraction, `mature` gets the full allowance. Enforcing that here — not in each connector — is what
//! stops a day-one account issuing 400 requests an hour and looking exactly like what it is (§4.4).

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Lifecycle tier ([[Session Manager]] §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Just acquired — warm-up traffic only, zero collection budget.
    Fresh,
    /// Warmed up, or recovered — a fraction of the mature budget, on low-value sources.
    Warming,
    /// Full budget, primary collection.
    Mature,
    /// Cooling down after a challenge — zero budget.
    Quarantined,
    /// Retired permanently; credentials revoked. Never reused.
    Burned,
}

impl Tier {
    /// The fraction of the mature request budget this tier may use.
    pub fn budget_ratio(self, warming_ratio: f64) -> f64 {
        match self {
            Tier::Mature => 1.0,
            Tier::Warming => warming_ratio.clamp(0.0, 1.0),
            // Fresh, quarantined and burned collect nothing.
            _ => 0.0,
        }
    }

    /// Whether an identity in this tier may be leased for collection at all.
    pub fn is_collectable(self) -> bool {
        matches!(self, Tier::Warming | Tier::Mature)
    }
}

/// Quarantine cooldown schedule ([[Session Manager]] §4.7, §5): doubles from `initial` to `max`.
pub fn quarantine_cooldown(initial: Duration, max: Duration, quarantine_number: u32) -> Duration {
    // `quarantine_number` is 1-based (the first quarantine is number 1).
    let level = quarantine_number.saturating_sub(1);
    let secs = initial
        .as_secs()
        .saturating_mul(1u64.checked_shl(level).unwrap_or(u64::MAX));
    Duration::from_secs(secs.min(max.as_secs()))
}

/// The lifecycle state of one identity: its tier and how many times it has been quarantined.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lifecycle {
    tier: Tier,
    /// Quarantines so far. Burn happens when this reaches the configured limit.
    quarantines: u32,
    /// Burn after this many quarantines (§5, default 3).
    burn_after: u32,
}

impl Lifecycle {
    /// A newly acquired identity: fresh, never quarantined.
    pub fn fresh(burn_after: u32) -> Self {
        Self {
            tier: Tier::Fresh,
            quarantines: 0,
            burn_after: burn_after.max(1),
        }
    }

    pub fn tier(&self) -> Tier {
        self.tier
    }

    pub fn quarantines(&self) -> u32 {
        self.quarantines
    }

    /// Warm-up finished: `fresh → warming`. A no-op from any other tier — you cannot warm up an
    /// identity that is already collecting or burned.
    pub fn finish_warmup(&mut self) {
        if self.tier == Tier::Fresh {
            self.tier = Tier::Warming;
        }
    }

    /// Warming period completed with no trouble: `warming → mature`.
    pub fn mature(&mut self) {
        if self.tier == Tier::Warming {
            self.tier = Tier::Mature;
        }
    }

    /// A challenge or ban hit this identity. Quarantines it, or **burns** it if this is the
    /// quarantine that reaches the limit. Returns the new tier so the caller can act on a burn.
    /// A burned identity stays burned — a late outcome report cannot resurrect it.
    pub fn challenge(&mut self) -> Tier {
        if self.tier == Tier::Burned {
            return Tier::Burned;
        }
        self.quarantines += 1;
        self.tier = if self.quarantines >= self.burn_after {
            Tier::Burned
        } else {
            Tier::Quarantined
        };
        self.tier
    }

    /// A recovery attempt succeeded: `quarantined → warming` at reduced budget (§4.7). Only a
    /// quarantined identity can recover; a burned one cannot.
    pub fn recover(&mut self) {
        if self.tier == Tier::Quarantined {
            self.tier = Tier::Warming;
        }
    }

    /// The cooldown before this identity's *current* quarantine may attempt recovery.
    pub fn cooldown(&self, initial: Duration, max: Duration) -> Option<Duration> {
        (self.tier == Tier::Quarantined)
            .then(|| quarantine_cooldown(initial, max, self.quarantines))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: u64 = 3600;

    #[test]
    fn a_fresh_identity_collects_nothing_until_warmed() {
        let mut life = Lifecycle::fresh(3);
        assert_eq!(life.tier(), Tier::Fresh);
        assert!(!life.tier().is_collectable());
        assert_eq!(life.tier().budget_ratio(0.25), 0.0);

        life.finish_warmup();
        assert_eq!(life.tier(), Tier::Warming);
        assert!(life.tier().is_collectable());
        assert_eq!(life.tier().budget_ratio(0.25), 0.25);

        life.mature();
        assert_eq!(life.tier().budget_ratio(0.25), 1.0);
    }

    #[test]
    fn three_quarantines_burn_the_identity() {
        let mut life = Lifecycle::fresh(3);
        life.finish_warmup();
        life.mature();

        assert_eq!(life.challenge(), Tier::Quarantined);
        life.recover();
        assert_eq!(life.tier(), Tier::Warming);

        assert_eq!(life.challenge(), Tier::Quarantined);
        life.recover();

        // Third quarantine → burned, and it stays burned.
        assert_eq!(life.challenge(), Tier::Burned);
        assert_eq!(life.quarantines(), 3);
        life.recover();
        assert_eq!(
            life.tier(),
            Tier::Burned,
            "a burned identity never recovers"
        );
    }

    #[test]
    fn cooldown_doubles_and_only_applies_while_quarantined() {
        let (init, max) = (Duration::from_secs(6 * H), Duration::from_secs(72 * H));
        let mut life = Lifecycle::fresh(5);
        life.finish_warmup();
        life.mature();

        life.challenge(); // 1st quarantine
        assert_eq!(life.cooldown(init, max), Some(Duration::from_secs(6 * H)));
        life.recover();
        assert_eq!(
            life.cooldown(init, max),
            None,
            "not quarantined → no cooldown"
        );

        life.challenge(); // 2nd
        assert_eq!(life.cooldown(init, max), Some(Duration::from_secs(12 * H)));
        life.recover();

        life.challenge(); // 3rd
        assert_eq!(life.cooldown(init, max), Some(Duration::from_secs(24 * H)));
    }

    #[test]
    fn cooldown_is_capped_at_the_maximum() {
        let (init, max) = (Duration::from_secs(6 * H), Duration::from_secs(72 * H));
        // A high quarantine number would double past the cap; it must clamp.
        assert_eq!(quarantine_cooldown(init, max, 10), max);
        assert_eq!(quarantine_cooldown(init, max, 1000), max);
    }

    #[test]
    fn a_burned_identity_ignores_further_challenges() {
        let mut life = Lifecycle::fresh(1);
        life.finish_warmup();
        // burn_after = 1 → first challenge burns.
        assert_eq!(life.challenge(), Tier::Burned);
        assert_eq!(life.challenge(), Tier::Burned);
        assert_eq!(
            life.quarantines(),
            1,
            "a late report does not tick the count past burn"
        );
    }
}
