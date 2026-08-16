//! The identity pool: leasing, the pinning invariant, and the exhaustion guard ([[Session Manager]]
//! §3, §4.2, §7).
//!
//! A lease hands a caller an identity's **pinned** proxy and fingerprint. There is no setter for
//! either on the lease, so a connector physically cannot rotate them within an identity — the rule
//! most scrapers get wrong, and the reason rotation-heavy designs burn pools fast (§4.2). Rotation
//! happens by leasing a *different* identity.
//!
//! The other load-bearing rule: when every identity for a platform is unusable (all quarantined or
//! burned), acquisition **halts** that platform. It never degrades to an unpinned identity — a
//! request without the pinned proxy/fingerprint is the exact incoherence platforms flag on (§7).

use super::budget::{BudgetLimits, BudgetSpend};
use super::lifecycle::Lifecycle;
use super::{Capability, Platform};

/// A collection identity: an account plus everything pinned to it for life.
#[derive(Debug, Clone)]
pub struct Identity {
    pub id: String,
    pub platform: Platform,
    /// PINNED for the identity's life ([[Session Manager]] §4.2).
    pub proxy_id: String,
    /// PINNED.
    pub fingerprint_id: String,
    pub lifecycle: Lifecycle,
    pub spend: BudgetSpend,
    pub capabilities: Vec<Capability>,
}

impl Identity {
    /// Whether this identity can serve a request needing `cap`. `Anonymous` needs nothing; the
    /// others require the identity to actually hold that capability.
    pub fn can(&self, cap: &Capability) -> bool {
        match cap {
            Capability::Anonymous => true,
            other => self.capabilities.contains(other),
        }
    }
}

/// A lease: the identity to use and its pinned egress. Deliberately has **no** way to change the
/// proxy or fingerprint — that is the pinning invariant, enforced by the absence of a setter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLease {
    pub identity_id: String,
    pub proxy_id: String,
    pub fingerprint_id: String,
    pub capability: Capability,
}

/// Why a lease could not be granted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionPoolError {
    /// No collectable identity exists for the platform — all quarantined or burned. **Halt the
    /// platform**; do not fall back to an unpinned identity.
    #[error("the {0:?} identity pool is exhausted; collection halts for this platform")]
    Exhausted(Platform),
    /// Collectable identities exist, but none is eligible right now — out of budget, outside its
    /// active window, or lacking the capability. Back off and retry later; do **not** halt.
    #[error("no {0:?} identity available right now")]
    NoneAvailableNow(Platform),
}

/// The pool of identities.
#[derive(Debug, Default)]
pub struct SessionPool {
    identities: Vec<Identity>,
    limits: BudgetLimits,
    warming_ratio: f64,
}

impl SessionPool {
    pub fn new(limits: BudgetLimits, warming_ratio: f64) -> Self {
        Self {
            identities: Vec::new(),
            limits,
            warming_ratio,
        }
    }

    pub fn add(&mut self, identity: Identity) {
        self.identities.push(identity);
    }

    pub fn get(&self, id: &str) -> Option<&Identity> {
        self.identities.iter().find(|i| i.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Identity> {
        self.identities.iter_mut().find(|i| i.id == id)
    }

    /// Lease an identity for one unit of work on `platform` needing `cap`, at local hour
    /// `local_hour` (Africa/Algiers). `dice` in `0.0..1.0` picks among the eligible identities.
    ///
    /// Distinguishes the two "no identity" cases precisely, because they need opposite responses:
    /// an exhausted pool halts the platform, while a merely-quiet one waits.
    pub fn acquire(
        &self,
        platform: Platform,
        cap: &Capability,
        local_hour: u8,
        dice: f64,
    ) -> Result<SessionLease, SessionPoolError> {
        // Collectable at all? If nothing on this platform is warming/mature, the pool is exhausted.
        let any_collectable = self
            .identities
            .iter()
            .any(|i| i.platform == platform && i.lifecycle.tier().is_collectable());
        if !any_collectable {
            return Err(SessionPoolError::Exhausted(platform));
        }

        // Eligible right now: collectable, holds the capability, inside its active window, and has
        // budget left at its tier's scaled limits.
        let mut eligible: Vec<&Identity> = self
            .identities
            .iter()
            .filter(|i| i.platform == platform && i.lifecycle.tier().is_collectable())
            .filter(|i| i.can(cap))
            .filter(|i| self.limits.is_active_hour(local_hour))
            .filter(|i| {
                let ratio = i.lifecycle.tier().budget_ratio(self.warming_ratio);
                let (h, d) = self.limits.scaled(ratio);
                i.spend.can_spend(h, d)
            })
            .collect();
        eligible.sort_by(|a, b| a.id.cmp(&b.id));

        if eligible.is_empty() {
            return Err(SessionPoolError::NoneAvailableNow(platform));
        }
        let idx = ((dice.clamp(0.0, 1.0) * eligible.len() as f64) as usize).min(eligible.len() - 1);
        let chosen = eligible[idx];
        Ok(SessionLease {
            identity_id: chosen.id.clone(),
            proxy_id: chosen.proxy_id.clone(),
            fingerprint_id: chosen.fingerprint_id.clone(),
            capability: cap.clone(),
        })
    }

    /// Record one request spent against an identity's budget.
    pub fn record_spend(&mut self, identity_id: &str) {
        if let Some(i) = self.get_mut(identity_id) {
            i.spend.used_hour += 1;
            i.spend.used_day += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::lifecycle::Tier;
    use super::*;

    fn mature(id: &str) -> Identity {
        let mut life = Lifecycle::fresh(3);
        life.finish_warmup();
        life.mature();
        Identity {
            id: id.into(),
            platform: Platform::Instagram,
            proxy_id: format!("proxy-for-{id}"),
            fingerprint_id: format!("fp-for-{id}"),
            lifecycle: life,
            spend: BudgetSpend::default(),
            capabilities: vec![Capability::LoggedIn],
        }
    }

    fn pool() -> SessionPool {
        // A window covering the whole day so tests are not hour-sensitive unless they mean to be.
        let limits = BudgetLimits {
            active_start_hour: 0,
            active_len_hours: 24,
            ..BudgetLimits::instagram()
        };
        SessionPool::new(limits, 0.25)
    }

    #[test]
    fn a_lease_always_carries_the_identitys_pinned_proxy_and_fingerprint() {
        let mut p = pool();
        p.add(mature("ig-1"));
        for _ in 0..10 {
            let lease = p
                .acquire(Platform::Instagram, &Capability::LoggedIn, 12, 0.0)
                .unwrap();
            assert_eq!(lease.identity_id, "ig-1");
            assert_eq!(
                lease.proxy_id, "proxy-for-ig-1",
                "the pinned proxy, every time"
            );
            assert_eq!(lease.fingerprint_id, "fp-for-ig-1");
        }
    }

    #[test]
    fn an_exhausted_pool_halts_the_platform() {
        let mut p = pool();
        let mut burned = mature("ig-1");
        // Burn it: three challenges.
        for _ in 0..3 {
            burned.lifecycle.challenge();
        }
        assert_eq!(burned.lifecycle.tier(), Tier::Burned);
        p.add(burned);
        assert_eq!(
            p.acquire(Platform::Instagram, &Capability::Anonymous, 12, 0.5),
            Err(SessionPoolError::Exhausted(Platform::Instagram))
        );
    }

    #[test]
    fn a_quarantined_pool_with_budget_elsewhere_is_exhaustion_not_availability() {
        let mut p = pool();
        let mut q = mature("ig-1");
        q.lifecycle.challenge(); // → quarantined, not collectable
        p.add(q);
        // Only identity is quarantined → the whole platform is exhausted.
        assert_eq!(
            p.acquire(Platform::Instagram, &Capability::Anonymous, 12, 0.5),
            Err(SessionPoolError::Exhausted(Platform::Instagram))
        );
    }

    #[test]
    fn a_spent_budget_is_none_available_now_not_exhaustion() {
        let mut p = pool();
        let mut i = mature("ig-1");
        i.spend = BudgetSpend {
            used_hour: 60,
            used_day: 100,
        }; // hourly cap hit
        p.add(i);
        // The identity is collectable (mature), just out of budget → wait, do not halt.
        assert_eq!(
            p.acquire(Platform::Instagram, &Capability::Anonymous, 12, 0.5),
            Err(SessionPoolError::NoneAvailableNow(Platform::Instagram))
        );
    }

    #[test]
    fn a_logged_in_request_skips_an_anonymous_only_identity() {
        let mut p = pool();
        let mut anon = mature("ig-anon");
        anon.capabilities = vec![]; // anonymous-only
        p.add(anon);
        // Collectable but cannot serve LoggedIn → none available now.
        assert_eq!(
            p.acquire(Platform::Instagram, &Capability::LoggedIn, 12, 0.5),
            Err(SessionPoolError::NoneAvailableNow(Platform::Instagram))
        );
        // But it can serve an anonymous request.
        assert!(p
            .acquire(Platform::Instagram, &Capability::Anonymous, 12, 0.5)
            .is_ok());
    }

    #[test]
    fn recording_spend_walks_an_identity_up_to_its_cap() {
        let mut p = pool();
        p.add(mature("ig-1"));
        for _ in 0..60 {
            assert!(p
                .acquire(Platform::Instagram, &Capability::Anonymous, 12, 0.0)
                .is_ok());
            p.record_spend("ig-1");
        }
        // 60 hourly requests spent → capped.
        assert_eq!(
            p.acquire(Platform::Instagram, &Capability::Anonymous, 12, 0.0),
            Err(SessionPoolError::NoneAvailableNow(Platform::Instagram))
        );
    }
}
