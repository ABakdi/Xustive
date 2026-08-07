//! Per-route rate limiting.
//!
//! # Why the keys look like this
//!
//! A rate limiter is a log of who asked for what and when. Kept naively it is a browsing-history
//! database that nobody decided to build — and one that would survive a subpoena, a breach, or a
//! change of ownership.
//!
//! So buckets are keyed on `HMAC(salt, ip/24)`, where the salt is generated at boot and rotated
//! daily and never leaves memory ([[Security and Privacy]] P5). Three properties follow:
//!
//! - **Truncation to /24** means a key identifies a neighbourhood, not a person. It also matches
//!   how mobile carriers in Algeria hand out addresses, where one subscriber's address changes
//!   more often than the network they are on.
//! - **HMAC with a secret salt** means possessing the table does not let you test whether a given
//!   IP is in it, which a plain hash would.
//! - **Rotation** bounds how far back any correlation can reach to one day, and process restarts
//!   cut it shorter.
//!
//! The cost is that a limiter cannot recognise a client across a rotation. That is the intended
//! trade: an attacker gains one extra window per day, and we give up the ability to build a
//! profile at all.
//!
//! # Why not a token bucket
//!
//! Fixed windows are less smooth than a token bucket and allow a 2× burst across a boundary.
//! They are also a counter and an instant per key, where a token bucket needs a float and a
//! timestamp updated on every request. At 512 concurrent requests the difference in contention
//! matters more than the burst does, and the burst is bounded and harmless.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// A limit for one route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limit {
    pub requests: u32,
    pub window_secs: u64,
}

impl Limit {
    pub const fn new(requests: u32, window_secs: u64) -> Self {
        Self {
            requests,
            window_secs,
        }
    }

    fn window(&self) -> Duration {
        Duration::from_secs(self.window_secs)
    }
}

/// Defaults from [[API Contract]] §9.
///
/// Suggest is five times search because it fires per keystroke; a limit that a normal typist
/// trips is a limit that only affects real users.
pub const SEARCH: Limit = Limit::new(60, 60);
pub const SUGGEST: Limit = Limit::new(300, 60);
pub const SUMMARY: Limit = Limit::new(20, 60);
pub const MEDIA: Limit = Limit::new(10, 60);
pub const SOURCES: Limit = Limit::new(5, 3600);

/// What the limiter decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub allowed: bool,
    pub remaining: u32,
    /// Seconds until this bucket resets. Sent as `Retry-After` on a refusal.
    pub retry_after: u64,
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    count: u32,
    started: Instant,
}

/// Fixed-window counters keyed by anonymised client and route.
pub struct RateLimiter {
    buckets: Mutex<HashMap<(u64, &'static str), Bucket>>,
    salt: Mutex<Salt>,
}

struct Salt {
    value: [u8; 32],
    rotated: Instant,
}

/// How long a salt lives. Shorter than a day would cost accuracy for no privacy gain; longer
/// would extend how far correlation can reach.
const SALT_TTL: Duration = Duration::from_secs(24 * 3600);

/// Cap on tracked buckets.
///
/// A limiter with unbounded state is a memory-exhaustion vector aimed at itself: an attacker
/// spraying source addresses grows the map faster than any window expires it.
const MAX_BUCKETS: usize = 200_000;

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
            salt: Mutex::new(Salt {
                value: random_salt(),
                rotated: Instant::now(),
            }),
        }
    }

    /// Count a request against `route` and say whether it may proceed.
    pub fn check(&self, peer: Option<IpAddr>, route: &'static str, limit: Limit) -> Decision {
        let key = (self.client_key(peer), route);
        let now = Instant::now();

        let mut buckets = match self.buckets.lock() {
            Ok(b) => b,
            Err(poisoned) => poisoned.into_inner(),
        };

        // Sweep before inserting, so the map cannot grow past the cap by one request at a time.
        if buckets.len() >= MAX_BUCKETS {
            buckets.retain(|(_, r), b| {
                now.duration_since(b.started) < Duration::from_secs(3600).min(route_window(r))
            });
            // Still full: every bucket is live, which means real load rather than a leak.
            // Allowing through beats refusing everyone on a full table.
            if buckets.len() >= MAX_BUCKETS {
                return Decision {
                    allowed: true,
                    remaining: 0,
                    retry_after: 0,
                };
            }
        }

        let bucket = buckets.entry(key).or_insert(Bucket {
            count: 0,
            started: now,
        });

        let elapsed = now.duration_since(bucket.started);
        if elapsed >= limit.window() {
            bucket.count = 0;
            bucket.started = now;
        }

        bucket.count += 1;
        let allowed = bucket.count <= limit.requests;
        let remaining = limit.requests.saturating_sub(bucket.count);
        let retry_after = limit
            .window()
            .saturating_sub(now.duration_since(bucket.started))
            .as_secs()
            .max(1);

        Decision {
            allowed,
            remaining,
            retry_after,
        }
    }

    /// Anonymise a client address.
    ///
    /// An absent address — which happens behind a proxy that strips connection info — collapses
    /// everyone into one bucket. That is deliberately the *strict* failure: a limiter that stops
    /// limiting when it cannot identify anyone is not a limiter.
    fn client_key(&self, peer: Option<IpAddr>) -> u64 {
        let mut salt = match self.salt.lock() {
            Ok(s) => s,
            Err(poisoned) => poisoned.into_inner(),
        };
        if salt.rotated.elapsed() >= SALT_TTL {
            salt.value = random_salt();
            salt.rotated = Instant::now();
        }

        let network = match peer {
            // /24 for IPv4, /48 for IPv6 — the smallest block a single subscriber is routinely
            // given. Anything narrower identifies a household.
            Some(IpAddr::V4(v4)) => {
                let o = v4.octets();
                vec![o[0], o[1], o[2]]
            }
            Some(IpAddr::V6(v6)) => v6.octets()[..6].to_vec(),
            None => Vec::new(),
        };

        // Keyed hash, not a plain one: with a plain hash, anyone holding the table can test
        // whether a particular address is in it, which is the property we are trying to remove.
        let mut hasher = blake3::Hasher::new_keyed(&salt.value);
        hasher.update(&network);
        let mut out = [0u8; 8];
        hasher.finalize_xof().fill(&mut out);
        u64::from_le_bytes(out)
    }

    #[cfg(test)]
    fn force_rotate(&self) {
        let mut salt = self.salt.lock().unwrap();
        salt.value = random_salt();
        salt.rotated = Instant::now();
    }

    pub fn tracked(&self) -> usize {
        self.buckets.lock().map(|b| b.len()).unwrap_or(0)
    }
}

fn route_window(route: &str) -> Duration {
    match route {
        "/sources" => SOURCES.window(),
        _ => SEARCH.window(),
    }
}

/// A salt from the OS.
///
/// `getrandom` via `blake3`'s dependency chain is not guaranteed, so this reads `/dev/urandom`
/// directly and falls back to process entropy. The fallback is weaker but still unpredictable to
/// a remote caller, which is the only threat model that matters for a bucket key.
fn random_salt() -> [u8; 32] {
    let mut buf = [0u8; 32];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut buf))
        .is_ok()
    {
        return buf;
    }

    let mut hasher = blake3::Hasher::new();
    hasher.update(&std::process::id().to_le_bytes());
    hasher.update(
        &std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
            .to_le_bytes(),
    );
    hasher.update(&(&buf as *const _ as usize).to_le_bytes());
    hasher.finalize().as_bytes()[..32].try_into().unwrap_or(buf)
}

/// Extract the client address for limiting.
///
/// The connection's peer address, never a forwarded header. `X-Forwarded-For` is attacker-
/// controlled, and trusting it lets any client pick its own bucket — which is worse than no limit
/// at all, because it looks like one. When a reverse proxy is added, it must be the thing
/// applying limits, or this must learn which proxies it trusts.
pub fn client_ip(peer: Option<SocketAddr>) -> Option<IpAddr> {
    peer.map(|s| s.ip())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> Option<IpAddr> {
        Some(IpAddr::V4(Ipv4Addr::new(a, b, c, d)))
    }

    #[test]
    fn requests_are_allowed_up_to_the_limit_then_refused() {
        let rl = RateLimiter::new();
        let limit = Limit::new(3, 60);
        for i in 1..=3 {
            let d = rl.check(ip(1, 2, 3, 4), "/search", limit);
            assert!(d.allowed, "request {i} should be allowed");
        }
        let d = rl.check(ip(1, 2, 3, 4), "/search", limit);
        assert!(!d.allowed);
        assert!(d.retry_after >= 1, "a refusal must say when to come back");
    }

    #[test]
    fn addresses_in_the_same_slash_24_share_a_bucket() {
        // Truncation is what makes a key a neighbourhood rather than a person.
        let rl = RateLimiter::new();
        let limit = Limit::new(2, 60);
        assert!(rl.check(ip(1, 2, 3, 4), "/search", limit).allowed);
        assert!(rl.check(ip(1, 2, 3, 250), "/search", limit).allowed);
        assert!(
            !rl.check(ip(1, 2, 3, 99), "/search", limit).allowed,
            "the third request from the same /24 should be refused"
        );
    }

    #[test]
    fn different_networks_do_not_share_a_bucket() {
        let rl = RateLimiter::new();
        let limit = Limit::new(1, 60);
        assert!(rl.check(ip(1, 2, 3, 4), "/search", limit).allowed);
        assert!(
            rl.check(ip(1, 2, 9, 4), "/search", limit).allowed,
            "a different /24 must have its own budget"
        );
    }

    #[test]
    fn routes_have_independent_budgets() {
        // Exhausting suggest must not lock a user out of search. They are different costs and
        // different behaviours.
        let rl = RateLimiter::new();
        let limit = Limit::new(1, 60);
        assert!(rl.check(ip(1, 2, 3, 4), "/suggest", limit).allowed);
        assert!(!rl.check(ip(1, 2, 3, 4), "/suggest", limit).allowed);
        assert!(rl.check(ip(1, 2, 3, 4), "/search", limit).allowed);
    }

    #[test]
    fn the_key_is_not_derivable_without_the_salt() {
        // The property that keeps this from being a browsing-history table: two limiters with
        // different salts produce different keys for the same address, so possessing the map
        // does not let anyone test whether an address is in it.
        let a = RateLimiter::new();
        let b = RateLimiter::new();
        assert_ne!(
            a.client_key(ip(1, 2, 3, 4)),
            b.client_key(ip(1, 2, 3, 4)),
            "keys must depend on a secret, not just the address"
        );
    }

    #[test]
    fn rotating_the_salt_breaks_correlation() {
        let rl = RateLimiter::new();
        let before = rl.client_key(ip(1, 2, 3, 4));
        rl.force_rotate();
        assert_ne!(
            before,
            rl.client_key(ip(1, 2, 3, 4)),
            "after rotation the same client must be unrecognisable"
        );
    }

    #[test]
    fn an_unknown_address_still_gets_limited() {
        // Failing open on a missing peer address would let anyone behind a stripping proxy
        // bypass the limiter entirely.
        let rl = RateLimiter::new();
        let limit = Limit::new(1, 60);
        assert!(rl.check(None, "/search", limit).allowed);
        assert!(!rl.check(None, "/search", limit).allowed);
    }

    #[test]
    fn ipv6_is_truncated_too() {
        let rl = RateLimiter::new();
        let limit = Limit::new(2, 60);
        let a: IpAddr = "2001:db8:1::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1::ffff".parse().unwrap();
        assert!(rl.check(Some(a), "/search", limit).allowed);
        assert!(rl.check(Some(b), "/search", limit).allowed);
        assert!(
            !rl.check(Some(a), "/search", limit).allowed,
            "the same /48 must share a bucket"
        );
    }

    #[test]
    fn the_window_resets() {
        let rl = RateLimiter::new();
        // A zero-length window expires immediately, which is the cheapest way to observe a reset
        // without sleeping through a real one.
        let limit = Limit::new(1, 0);
        assert!(rl.check(ip(1, 2, 3, 4), "/search", limit).allowed);
        assert!(
            rl.check(ip(1, 2, 3, 4), "/search", limit).allowed,
            "an expired window must start a fresh count"
        );
    }

    #[test]
    fn the_client_address_comes_only_from_the_connection() {
        // `client_ip` takes a `SocketAddr`, never headers — the signature is the guarantee, and
        // this pins it so a future change to accept `X-Forwarded-For` has to delete a test that
        // says why not. A client that can pick its own bucket is worse than no limiter: it
        // looks like one to everybody except the person attacking it.
        let addr: SocketAddr = "203.0.113.7:44321".parse().unwrap();
        assert_eq!(client_ip(Some(addr)), Some(addr.ip()));
        assert_eq!(client_ip(None), None);
    }
}
