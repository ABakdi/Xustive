//! The Proxy Manager (M2-T07): outbound network identity — pools, health, placement, pinning,
//! ban attribution, and cost.
//!
//! Two jobs share this component ([[Proxy Manager]] §1). **Open web** wants rate distribution and
//! reliability, and most traffic goes out `direct`. **Platforms** are different: the IP's reputation
//! *is* an authentication factor, a datacentre address is classified on sight, and the response to a
//! block is graded rather than a flat halt ([[ADR-0009]]). The one commitment that does not bend is
//! the open-web posture — a small Algerian site that asks us not to crawl a path is still obeyed,
//! and a 403 there halts and flags rather than retrying through another pool.
//!
//! This module is the **decision logic**: health scoring, state transitions, selection weighting,
//! failure attribution, pinning, and the placement caps. The parts that need real infrastructure —
//! provider credentials, actual egress, measured bandwidth — sit at the edges and feed this core the
//! outcomes it scores. Getting the logic right and tested is what stops the classic failure where
//! one dead host quarantines a whole pool (§4.5).

mod attribution;
mod health;
mod ladder;
mod placement;

pub use attribution::{attribute, Blame, FailureEvent};
pub use health::{Health, ProxyState, MIN_LATENCY_MS};
pub use ladder::{on_blocked, Action, BlockSignal};
pub use placement::{PlacementError, PlacementLedger};

use serde::{Deserialize, Serialize};

/// Which pool an outbound request goes through ([[Proxy Manager]] §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PoolKind {
    /// Free, best latency — the default, and where all open-web `.dz` crawling goes.
    Direct,
    /// Flat/per-IP, good latency — high-volume open web that hits per-host limits on one IP.
    Datacenter,
    /// Per-GB, added latency — Instagram, Facebook. Residential bandwidth is the largest variable
    /// cost in the system, so routing open-web crawling here by mistake is a budget fire (§4.1).
    Residential,
    /// Per-GB and highest cost — Facebook where residential is insufficient; highest-trust IPs.
    Mobile,
}

impl PoolKind {
    /// Whether this pool serves platform collection, where a block is handled by the graded ladder
    /// and a fallback to `direct` is forbidden — a datacentre IP after a residential one is an
    /// obvious tell (§7).
    pub const fn is_platform(self) -> bool {
        matches!(self, Self::Residential | Self::Mobile | Self::Datacenter)
    }

    /// Whether losing every proxy in this pool must **halt** rather than degrade. Platform pools
    /// halt; `direct` cannot "run out".
    pub const fn halts_when_empty(self) -> bool {
        self.is_platform()
    }
}

/// Per-source-class routing policy ([[Proxy Manager]] §4.1), set in the data sources registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolPolicy {
    pub kind: PoolKind,
    /// Preferred egress country, e.g. `"DZ"`. Platform collection prefers `DZ`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geo: Option<String>,
    /// Pin the proxy to the identity for its lifetime — required for platform pools (§4.3).
    #[serde(default)]
    pub sticky: bool,
}

impl PoolPolicy {
    /// The default open-web policy: `direct`, no geo, not sticky.
    pub fn direct() -> Self {
        Self {
            kind: PoolKind::Direct,
            geo: None,
            sticky: false,
        }
    }
}

/// What happened on a request through a proxy. `report` is mandatory ([[Proxy Manager]] §3): a lease
/// dropped without one is counted as a `Timeout`, or a leaked lease would silently corrupt every
/// health score in the pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Ok,
    Timeout,
    Refused,
    /// An anti-bot block (403 / challenge page) attributable to the IP.
    Blocked,
    /// A captcha/checkpoint served — the identity is likely already classified.
    Challenged,
    /// A hard ban on the address.
    Banned,
}

impl Outcome {
    /// Whether this outcome is a success for health purposes. Only `Ok` is.
    pub const fn is_success(self) -> bool {
        matches!(self, Self::Ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_platform_pools_halt_when_empty() {
        assert!(!PoolKind::Direct.halts_when_empty());
        assert!(PoolKind::Datacenter.halts_when_empty());
        assert!(PoolKind::Residential.halts_when_empty());
        assert!(PoolKind::Mobile.halts_when_empty());
    }

    #[test]
    fn pool_policy_round_trips_and_defaults_sensibly() {
        let p = PoolPolicy::direct();
        assert_eq!(p.kind, PoolKind::Direct);
        assert!(!p.sticky);
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(PoolPolicy::direct(), serde_json::from_str(&json).unwrap());
    }
}
