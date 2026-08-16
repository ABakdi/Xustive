//! Geographic and ASN placement caps ([[Proxy Manager]] §4.2).
//!
//! Algerian content viewed from Algerian addresses is unremarkable; the same requests concentrated
//! in one ASN and one /16 correlate trivially. Two structural rules keep the pool from being a
//! cluster signal:
//!
//! - **At most `max_per_subnet` identities share a /24.** Fifty accounts behind one address is the
//!   clearest possible cluster.
//! - **At least `min_distinct_asns` ASNs are in use.** A pool that has drifted onto one ASN is one
//!   provider outage — or one ASN-level block — away from losing everything at once.
//!
//! This ledger is the accountant, not the assigner: it records identity → (subnet, ASN) and refuses
//! an assignment that would breach the /24 cap, and it reports whether the ASN spread is healthy so
//! the caller can widen the pool before pinning more identities.

use std::collections::{HashMap, HashSet};

/// Default caps ([[Proxy Manager]] §5).
pub const MAX_IDENTITIES_PER_SUBNET: usize = 3;
pub const MIN_DISTINCT_ASNS: usize = 4;

/// Why an assignment was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlacementError {
    #[error("subnet {subnet} already holds {count} identities (cap {cap})")]
    SubnetFull {
        subnet: String,
        count: usize,
        cap: usize,
    },
    #[error("identity {0} is already placed; release it before reassigning")]
    AlreadyPlaced(String),
}

/// One identity's placement.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Placement {
    /// The /24 the identity's proxy sits in, e.g. `"41.200.10"`.
    subnet: String,
    asn: String,
}

/// Records identity placements and enforces the caps.
#[derive(Debug, Clone)]
pub struct PlacementLedger {
    by_identity: HashMap<String, Placement>,
    max_per_subnet: usize,
    min_asns: usize,
}

impl Default for PlacementLedger {
    fn default() -> Self {
        Self::new(MAX_IDENTITIES_PER_SUBNET, MIN_DISTINCT_ASNS)
    }
}

impl PlacementLedger {
    pub fn new(max_per_subnet: usize, min_asns: usize) -> Self {
        Self {
            by_identity: HashMap::new(),
            max_per_subnet: max_per_subnet.max(1),
            min_asns: min_asns.max(1),
        }
    }

    /// Place `identity` on the /24 of `ip` within `asn`. Refuses if the /24 is already at the cap,
    /// or if the identity is already placed (reassignment goes through `release` first, so a silent
    /// double-placement cannot slip the caps).
    pub fn assign(&mut self, identity: &str, ip: &str, asn: &str) -> Result<(), PlacementError> {
        if self.by_identity.contains_key(identity) {
            return Err(PlacementError::AlreadyPlaced(identity.to_string()));
        }
        let subnet = subnet_24(ip);
        let count = self.subnet_count(&subnet);
        if count >= self.max_per_subnet {
            return Err(PlacementError::SubnetFull {
                subnet,
                count,
                cap: self.max_per_subnet,
            });
        }
        self.by_identity.insert(
            identity.to_string(),
            Placement {
                subnet,
                asn: asn.to_string(),
            },
        );
        Ok(())
    }

    /// Release an identity's placement — on reassignment or when it is burned.
    pub fn release(&mut self, identity: &str) {
        self.by_identity.remove(identity);
    }

    fn subnet_count(&self, subnet: &str) -> usize {
        self.by_identity
            .values()
            .filter(|p| p.subnet == subnet)
            .count()
    }

    /// The distinct ASNs currently in use.
    pub fn distinct_asns(&self) -> usize {
        self.by_identity
            .values()
            .map(|p| p.asn.as_str())
            .collect::<HashSet<_>>()
            .len()
    }

    /// Whether the ASN spread meets the floor. False means the pool is too concentrated and the
    /// caller should widen it before pinning more identities.
    pub fn asn_spread_ok(&self) -> bool {
        self.distinct_asns() >= self.min_asns
    }

    pub fn placed(&self) -> usize {
        self.by_identity.len()
    }
}

/// The /24 of an IPv4 address — the first three octets. A non-IPv4 string returns itself, so a
/// malformed entry is its own bucket rather than silently colliding with a real subnet.
fn subnet_24(ip: &str) -> String {
    let octets: Vec<&str> = ip.split('.').collect();
    if octets.len() == 4 && octets.iter().all(|o| o.parse::<u8>().is_ok()) {
        format!("{}.{}.{}", octets[0], octets[1], octets[2])
    } else {
        ip.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fourth_identity_on_one_slash_24_is_refused() {
        let mut led = PlacementLedger::new(3, 4);
        assert!(led.assign("id1", "41.200.10.5", "AT").is_ok());
        assert!(led.assign("id2", "41.200.10.9", "AT").is_ok());
        assert!(led.assign("id3", "41.200.10.40", "AT").is_ok());
        // Same /24, cap reached.
        let err = led.assign("id4", "41.200.10.200", "AT").unwrap_err();
        assert!(matches!(err, PlacementError::SubnetFull { cap: 3, .. }));
        // A different /24 is fine.
        assert!(led.assign("id4", "41.200.11.5", "AT").is_ok());
    }

    #[test]
    fn asn_spread_needs_four_distinct_asns() {
        let mut led = PlacementLedger::new(3, 4);
        led.assign("a", "41.200.1.1", "AlgerieTelecom").unwrap();
        led.assign("b", "41.201.1.1", "Mobilis").unwrap();
        led.assign("c", "41.202.1.1", "Djezzy").unwrap();
        assert!(
            !led.asn_spread_ok(),
            "three ASNs is below the floor of four"
        );
        led.assign("d", "41.203.1.1", "Ooredoo").unwrap();
        assert!(led.asn_spread_ok());
        assert_eq!(led.distinct_asns(), 4);
    }

    #[test]
    fn reassignment_must_release_first() {
        let mut led = PlacementLedger::new(3, 4);
        led.assign("id", "41.200.10.5", "AT").unwrap();
        assert!(matches!(
            led.assign("id", "41.200.20.5", "AT"),
            Err(PlacementError::AlreadyPlaced(_))
        ));
        led.release("id");
        assert!(led.assign("id", "41.200.20.5", "AT").is_ok());
    }

    #[test]
    fn subnet_is_the_first_three_octets() {
        let mut led = PlacementLedger::new(1, 1);
        led.assign("a", "10.0.0.1", "x").unwrap();
        // Same /24, different host octet → refused at cap 1.
        assert!(led.assign("b", "10.0.0.254", "x").is_err());
        // Different third octet → different /24.
        assert!(led.assign("b", "10.0.1.1", "x").is_ok());
    }
}
