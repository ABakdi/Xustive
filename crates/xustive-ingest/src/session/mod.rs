//! The Session Manager (M2-T01a): the identities used for direct collection — accounts, cookies,
//! sessions, and the budgets that keep them alive.
//!
//! This is the component that decides whether collection works for months or for days. Every other
//! part of the scraping stack is replaceable; a burned account pool takes weeks to rebuild
//! ([[Session Manager]] §1). Two rules carry that weight, and both are enforced in code here rather
//! than left to a connector to remember:
//!
//! 1. **The pinning invariant** (§4.2): `account ↔ proxy ↔ fingerprint ↔ device` is stable for the
//!    life of the identity. A lease hands back the identity's *pinned* proxy and fingerprint; there
//!    is no setter, so a caller cannot rotate within an identity — rotation happens between
//!    identities, never inside one.
//! 2. **Anonymous first** (§3): an anonymous session risks only a proxy IP; a logged-in one risks an
//!    asset that took weeks to warm. So the cheapest capability that can do the job is preferred.
//!
//! This module holds the **decision logic** — lifecycle, detection, budgets, crypto — the same
//! "build the engine, defer the fuel" split as the [[proxy]] manager. Account acquisition, real
//! login flows, and warm-up browsing need real accounts and are out of scope here.

mod crypto;
mod detection;
mod lifecycle;

pub use crypto::{CookieCrypto, CryptoError};
pub use detection::{classify, Detection};
pub use lifecycle::{Lifecycle, Tier};

use serde::{Deserialize, Serialize};

/// A platform we collect from directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Instagram,
    Facebook,
    Tiktok,
}

impl Platform {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Instagram => "instagram",
            Self::Facebook => "facebook",
            Self::Tiktok => "tiktok",
        }
    }
}

/// What a piece of work needs from an identity. Requested cheapest-first (§3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    /// No login — risks only the proxy IP.
    Anonymous,
    /// A logged-in session — risks the account.
    LoggedIn,
    /// Membership of a specific group.
    GroupMember(String),
}

/// The kind of challenge a platform served.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChallengeKind {
    Captcha,
    Checkpoint,
    Twofa,
    SuspiciousLogin,
    /// A login wall on content that used to be reachable anonymously.
    LoginWall,
}

/// The outcome of one leased request — the feedback health scoring depends on entirely (§3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOutcome {
    Ok {
        bytes: u64,
        items: u32,
    },
    /// 200 OK with zero items — suspected cloaking, not a plain success (§4.6).
    Empty,
    RateLimited,
    Challenge(ChallengeKind),
    Banned,
    NetworkError,
}

impl SessionOutcome {
    /// Whether this outcome actually delivered content. `Empty` is **not** a success — that is the
    /// whole point of silent-cloaking detection.
    pub const fn is_content(self) -> bool {
        matches!(self, Self::Ok { items, .. } if items > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_not_counted_as_content() {
        assert!(SessionOutcome::Ok {
            bytes: 100,
            items: 5
        }
        .is_content());
        assert!(!SessionOutcome::Ok {
            bytes: 100,
            items: 0
        }
        .is_content());
        assert!(!SessionOutcome::Empty.is_content());
        assert!(!SessionOutcome::RateLimited.is_content());
    }

    #[test]
    fn capability_and_outcome_round_trip_where_serialised() {
        let cap = Capability::GroupMember("123".into());
        let json = serde_json::to_string(&cap).unwrap();
        assert_eq!(cap, serde_json::from_str(&json).unwrap());
    }
}
