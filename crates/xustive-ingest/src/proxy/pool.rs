//! The in-memory proxy pool: selection, pinning, and the platform-halt guard ([[Proxy Manager]]
//! §3, §4.3, §7).
//!
//! This holds the live proxies and answers the two acquisition questions. `acquire` picks a proxy
//! for an open-web or one-off request, weighted by health so traffic spreads across the pool rather
//! than hammering the single best IP. `acquire_pinned` is the platform path: an identity's proxy is
//! fixed for its lifetime, so this returns the *same* proxy every time or refuses — it never quietly
//! reassigns, because a platform account whose egress IP moved is a flagged account.
//!
//! The load-bearing guard: when a **platform** pool has no eligible proxy, acquisition **halts**. It
//! must never fall back to `direct` — a residential request suddenly leaving from the crawler's own
//! datacentre address is the most obvious tell there is (§7).

use std::collections::HashMap;

use super::health::Health;
use super::{Outcome, PoolKind, PoolPolicy};

/// A proxy in the pool.
#[derive(Debug, Clone)]
pub struct Proxy {
    pub id: String,
    pub kind: PoolKind,
    pub asn: String,
    /// The /24 the proxy sits in, for placement accounting.
    pub subnet: String,
    pub geo: Option<String>,
    pub health: Health,
}

impl Proxy {
    pub fn new(id: &str, kind: PoolKind, asn: &str, subnet: &str, geo: Option<&str>) -> Self {
        Self {
            id: id.to_string(),
            kind,
            asn: asn.to_string(),
            subnet: subnet.to_string(),
            geo: geo.map(String::from),
            health: Health::default(),
        }
    }
}

/// Why an acquisition failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PoolError {
    /// A platform pool had no eligible proxy. The caller **halts the platform** — it does not fall
    /// back to `direct`.
    #[error("no eligible proxy in the {0:?} pool; platform collection halts")]
    PlatformHalt(PoolKind),
    /// An open-web request found no datacenter proxy; the caller may fall back to `direct`.
    #[error("no eligible proxy in the {0:?} pool")]
    NoProxy(PoolKind),
    /// The identity has no pin yet.
    #[error("identity {0} has no pinned proxy")]
    NoPin(String),
    /// The pinned proxy is dead and needs reassignment through the cooldown, not here.
    #[error("identity {0}'s pinned proxy is dead; reassignment required")]
    PinnedProxyDead(String),
    /// A pin was attempted to a proxy not fit to be pinned (not healthy).
    #[error("proxy {0} is not healthy enough to pin a new identity to")]
    NotPinnable(String),
}

/// The reserved id for the direct (no-proxy) egress path.
pub const DIRECT: &str = "direct";

#[derive(Debug, Default)]
pub struct Pool {
    proxies: HashMap<String, Proxy>,
    /// identity → proxy id. A pin is for the identity's lifetime.
    pins: HashMap<String, String>,
}

impl Pool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, proxy: Proxy) {
        self.proxies.insert(proxy.id.clone(), proxy);
    }

    pub fn get(&self, id: &str) -> Option<&Proxy> {
        self.proxies.get(id)
    }

    /// Fold a request outcome into a proxy's health. The mandatory `report` (§3).
    pub fn report(&mut self, proxy_id: &str, outcome: Outcome, latency_ms: f64) {
        if let Some(p) = self.proxies.get_mut(proxy_id) {
            p.health.observe(outcome, latency_ms);
        }
    }

    /// Acquire a proxy for a non-pinned request under `policy`. `dice` is a value in `0.0..1.0` used
    /// to spread traffic proportionally to health — the caller passes a random draw; a test passes a
    /// fixed point. `Direct` needs no proxy and always succeeds.
    pub fn acquire(&self, policy: &PoolPolicy, dice: f64) -> Result<String, PoolError> {
        if policy.kind == PoolKind::Direct {
            return Ok(DIRECT.to_string());
        }
        match self.select(policy.kind, policy.geo.as_deref(), dice) {
            Some(id) => Ok(id),
            None if policy.kind.halts_when_empty() => Err(PoolError::PlatformHalt(policy.kind)),
            None => Err(PoolError::NoProxy(policy.kind)),
        }
    }

    /// The pinned proxy for an identity — the same one every time (§4.3). Errors rather than
    /// reassigning: a dead pin is handled by the reassignment cooldown, not silently here.
    pub fn acquire_pinned(&self, identity: &str) -> Result<String, PoolError> {
        let proxy_id = self
            .pins
            .get(identity)
            .ok_or_else(|| PoolError::NoPin(identity.to_string()))?;
        match self.proxies.get(proxy_id) {
            Some(p) if p.health.state() != super::ProxyState::Dead => Ok(proxy_id.clone()),
            // A missing or dead pinned proxy is the reassignment case, not a silent swap.
            _ => Err(PoolError::PinnedProxyDead(identity.to_string())),
        }
    }

    /// Pin `identity` to a proxy chosen for it. Only a **healthy** proxy may take a new pin, and the
    /// proxy must exist and match the kind. Idempotent for an existing identical pin.
    pub fn pin(&mut self, identity: &str, proxy_id: &str) -> Result<(), PoolError> {
        let proxy = self
            .proxies
            .get(proxy_id)
            .ok_or_else(|| PoolError::NotPinnable(proxy_id.to_string()))?;
        if !proxy.health.is_pinnable() {
            return Err(PoolError::NotPinnable(proxy_id.to_string()));
        }
        self.pins.insert(identity.to_string(), proxy_id.to_string());
        Ok(())
    }

    /// Release an identity's pin — on reassignment or when the identity is burned.
    pub fn unpin(&mut self, identity: &str) {
        self.pins.remove(identity);
    }

    pub fn pinned_proxy(&self, identity: &str) -> Option<&str> {
        self.pins.get(identity).map(String::as_str)
    }

    /// The selection weights for the eligible proxies of `kind` (and `geo`, when set). Healthy
    /// proxies weigh their full score; degraded ones weigh a fraction, so they take a trickle of
    /// traffic to keep being probed without carrying the pool. Pure and returned sorted by id, so
    /// the weighting is testable directly.
    pub fn select_weights(&self, kind: PoolKind, geo: Option<&str>) -> Vec<(String, f64)> {
        let mut out: Vec<(String, f64)> = self
            .proxies
            .values()
            .filter(|p| p.kind == kind && p.health.is_eligible())
            .filter(|p| geo.is_none_or(|g| p.geo.as_deref() == Some(g)))
            .map(|p| {
                let base = p.health.score();
                let weight = if p.health.is_pinnable() {
                    base
                } else {
                    base * DEGRADED_WEIGHT_FACTOR
                };
                (p.id.clone(), weight)
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Pick a proxy weighted by health. `dice` in `0.0..1.0` selects a point in the cumulative
    /// weight; the caller supplies a random draw in production and a fixed value in a test.
    fn select(&self, kind: PoolKind, geo: Option<&str>, dice: f64) -> Option<String> {
        let weights = self.select_weights(kind, geo);
        let total: f64 = weights.iter().map(|(_, w)| w).sum();
        if total <= 0.0 {
            return None;
        }
        let mut target = dice.clamp(0.0, 1.0) * total;
        for (id, w) in &weights {
            target -= w;
            if target < 0.0 {
                return Some(id.clone());
            }
        }
        // Floating-point slack at dice≈1.0: fall back to the last candidate.
        weights.last().map(|(id, _)| id.clone())
    }
}

/// How much of its score a degraded proxy weighs relative to a healthy one — a trickle, not a share.
const DEGRADED_WEIGHT_FACTOR: f64 = 0.2;

#[cfg(test)]
mod tests {
    use super::*;

    fn healthy(id: &str, kind: PoolKind, geo: Option<&str>) -> Proxy {
        Proxy::new(id, kind, "as1", "41.200.10", geo)
    }

    #[test]
    fn direct_needs_no_proxy_and_always_succeeds() {
        let pool = Pool::new();
        assert_eq!(pool.acquire(&PoolPolicy::direct(), 0.5).unwrap(), DIRECT);
    }

    #[test]
    fn a_platform_pool_with_no_eligible_proxy_halts_and_never_uses_direct() {
        let pool = Pool::new();
        let policy = PoolPolicy {
            kind: PoolKind::Residential,
            geo: Some("DZ".into()),
            sticky: true,
        };
        let err = pool.acquire(&policy, 0.5).unwrap_err();
        assert_eq!(err, PoolError::PlatformHalt(PoolKind::Residential));
    }

    #[test]
    fn all_residential_proxies_down_halts_rather_than_falling_back() {
        // The T07.11 guard, end to end: every residential proxy quarantined, acquisition must halt.
        let mut pool = Pool::new();
        pool.add(healthy("r1", PoolKind::Residential, Some("DZ")));
        for _ in 0..3 {
            pool.report("r1", Outcome::Refused, 5000.0);
        }
        let policy = PoolPolicy {
            kind: PoolKind::Residential,
            geo: Some("DZ".into()),
            sticky: true,
        };
        assert_eq!(
            pool.acquire(&policy, 0.5).unwrap_err(),
            PoolError::PlatformHalt(PoolKind::Residential)
        );
    }

    #[test]
    fn a_pin_returns_the_same_proxy_every_time() {
        let mut pool = Pool::new();
        pool.add(healthy("r1", PoolKind::Residential, Some("DZ")));
        pool.add(healthy("r2", PoolKind::Residential, Some("DZ")));
        pool.pin("acct", "r1").unwrap();
        for _ in 0..10 {
            assert_eq!(pool.acquire_pinned("acct").unwrap(), "r1");
        }
    }

    #[test]
    fn a_new_pin_is_refused_to_a_non_healthy_proxy() {
        let mut pool = Pool::new();
        pool.add(healthy("r1", PoolKind::Residential, Some("DZ")));
        // Degrade it out of healthy without killing it.
        for _ in 0..6 {
            pool.report("r1", Outcome::Blocked, 200.0);
            pool.report("r1", Outcome::Blocked, 200.0);
            pool.report("r1", Outcome::Ok, 200.0);
        }
        assert!(!pool.get("r1").unwrap().health.is_pinnable());
        assert!(matches!(
            pool.pin("new", "r1"),
            Err(PoolError::NotPinnable(_))
        ));
    }

    #[test]
    fn a_dead_pinned_proxy_is_a_reassignment_not_a_silent_swap() {
        let mut pool = Pool::new();
        pool.add(healthy("r1", PoolKind::Residential, Some("DZ")));
        pool.add(healthy("r2", PoolKind::Residential, Some("DZ")));
        pool.pin("acct", "r1").unwrap();
        // Kill r1: five quarantines.
        for _ in 0..5 {
            pool.report("r1", Outcome::Refused, 5000.0);
            pool.report("r1", Outcome::Refused, 5000.0);
            pool.report("r1", Outcome::Refused, 5000.0);
            pool.proxies.get_mut("r1").unwrap().health.begin_probe();
        }
        assert_eq!(
            pool.get("r1").unwrap().health.state(),
            super::super::ProxyState::Dead
        );
        // It must NOT quietly return r2 — reassignment is a deliberate, cooled-down action.
        assert_eq!(
            pool.acquire_pinned("acct").unwrap_err(),
            PoolError::PinnedProxyDead("acct".into())
        );
    }

    #[test]
    fn selection_weights_prefer_healthy_over_degraded_and_respect_geo() {
        let mut pool = Pool::new();
        pool.add(healthy("dz1", PoolKind::Datacenter, Some("DZ")));
        pool.add(healthy("fr1", PoolKind::Datacenter, Some("FR")));
        // Only the DZ proxy is eligible for a DZ policy.
        let w = pool.select_weights(PoolKind::Datacenter, Some("DZ"));
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].0, "dz1");
        // Acquire returns it deterministically.
        let policy = PoolPolicy {
            kind: PoolKind::Datacenter,
            geo: Some("DZ".into()),
            sticky: false,
        };
        assert_eq!(pool.acquire(&policy, 0.99).unwrap(), "dz1");
    }
}
