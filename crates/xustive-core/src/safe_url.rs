//! The SSRF guard.
//!
//! Every URL entering the fetch path — from a sitemap, a discovered outlink, or a public source
//! submission — is attacker-influenced. [`SafeUrl`] is a newtype whose constructor performs the
//! checks, so the HTTP client can only be handed a URL that has passed them.
//!
//! Two stages, because DNS is the interesting half:
//!
//! 1. [`SafeUrl::parse`] — structural: scheme, credentials, port, literal IP.
//! 2. [`SafeUrl::check_resolved`] — every address the host resolves to.
//!
//! **Both must run again on every redirect hop.** A host that resolves publicly and then 302s to
//! `http://169.254.169.254/` is the standard bypass.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use url::{Host, Url};

use crate::error::{Classify, ErrorClass};

/// Ports we are willing to talk to. Anything else is a service we have no business fetching.
pub const ALLOWED_PORTS: &[u16] = &[80, 443, 8080, 8443];

/// Maximum redirect hops. Each hop is re-validated.
pub const MAX_REDIRECTS: usize = 5;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UrlError {
    #[error("malformed url: {0}")]
    Malformed(String),
    #[error("scheme {0:?} not allowed (only http/https)")]
    Scheme(String),
    #[error("url must not contain credentials")]
    Credentials,
    #[error("no host in url")]
    NoHost,
    #[error("port {0} not allowed")]
    Port(u16),
    #[error("host resolves to a private or reserved address: {0}")]
    PrivateAddress(IpAddr),
    #[error("host is not a resolvable name or address")]
    UnsupportedHost,
    #[error("too many redirects")]
    TooManyRedirects,
}

impl Classify for UrlError {
    fn class(&self) -> ErrorClass {
        // Every one of these is a property of the URL itself. Retrying changes nothing, and
        // repeatedly hammering a host with requests we will reject looks like abuse.
        ErrorClass::Permanent
    }
}

/// A URL that has passed structural SSRF validation.
///
/// Construction is the only way to get one, so a `SafeUrl` in a function signature is a proof
/// obligation discharged at the boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeUrl(Url);

impl SafeUrl {
    /// Validate a URL's structure.
    ///
    /// This does **not** resolve DNS — call [`SafeUrl::check_resolved`] with the addresses the
    /// resolver returned, immediately before connecting.
    /// Whether loopback targets are permitted for the life of this process.
    ///
    /// Off, and it takes a deliberate call to turn on. It exists for exactly one purpose: the
    /// offline fixture site in `tests/fixtures/site/`, which serves the redirect chains, rate
    /// limits and crawler traps that cannot be tested against the live web. Without it the
    /// fixture is unreachable and the hostile cases go untested — which is worse than the risk
    /// of a switch that is off by default and never set in the server binaries.
    ///
    /// A process-wide flag rather than a parameter because the guard is called from a dozen
    /// places, several of them deep inside redirect handling, and threading a boolean through
    /// all of them invites exactly one of them to be missed.
    pub fn allow_loopback_for_testing() {
        ALLOW_LOOPBACK.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    fn loopback_allowed() -> bool {
        ALLOW_LOOPBACK.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn parse(input: &str) -> Result<Self, UrlError> {
        let url = Url::parse(input).map_err(|e| UrlError::Malformed(e.to_string()))?;
        Self::from_url(url)
    }

    /// Validate an already-parsed [`Url`]. Used for redirect targets, which arrive pre-parsed.
    pub fn from_url(url: Url) -> Result<Self, UrlError> {
        match url.scheme() {
            "http" | "https" => {}
            other => return Err(UrlError::Scheme(other.to_string())),
        }

        // Credentials in a URL are never legitimate for a crawler and are a classic way to
        // confuse naive host parsing (`http://evil.com@127.0.0.1/`).
        if !url.username().is_empty() || url.password().is_some() {
            return Err(UrlError::Credentials);
        }

        let host = url.host().ok_or(UrlError::NoHost)?;

        if let Some(port) = url.port() {
            // The port allowlist relaxes under the loopback escape hatch too. A fixture server
            // cannot bind 80 without root, so enforcing the allowlist there would make the
            // hatch useless while appearing to work.
            if !ALLOWED_PORTS.contains(&port) && !Self::loopback_allowed() {
                return Err(UrlError::Port(port));
            }
        }

        // A literal address can be rejected now, without waiting for resolution. The `url`
        // crate has already normalised decimal/octal/hex IPv4 forms into `Host::Ipv4`.
        match host {
            Host::Ipv4(ip) => {
                let ip = IpAddr::V4(ip);
                if !is_public(ip) && !(ip.is_loopback() && Self::loopback_allowed()) {
                    return Err(UrlError::PrivateAddress(ip));
                }
            }
            Host::Ipv6(ip) => {
                let ip = IpAddr::V6(ip);
                if !is_public(ip) && !(ip.is_loopback() && Self::loopback_allowed()) {
                    return Err(UrlError::PrivateAddress(ip));
                }
            }
            Host::Domain(d) => {
                if d.is_empty() {
                    return Err(UrlError::NoHost);
                }
                // `localhost` and friends may resolve to a public address in a hostile DNS
                // setup, but there is no legitimate reason to fetch them.
                let lower = d.to_ascii_lowercase();
                if (lower == "localhost"
                    || lower.ends_with(".localhost")
                    || lower.ends_with(".local"))
                    && !Self::loopback_allowed()
                {
                    return Err(UrlError::PrivateAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)));
                }
            }
        }

        Ok(Self(url))
    }

    /// Check the addresses the resolver actually returned.
    ///
    /// Rejects if **any** resolved address is non-public. Being strict here closes DNS
    /// round-robin attacks where one of several A records points inside the network.
    pub fn check_resolved(&self, addrs: &[IpAddr]) -> Result<(), UrlError> {
        for &ip in addrs {
            if ip.is_loopback() && Self::loopback_allowed() {
                continue;
            }
            if !is_public(ip) {
                return Err(UrlError::PrivateAddress(ip));
            }
        }
        Ok(())
    }

    /// Validate a redirect target relative to this URL, enforcing the hop limit.
    pub fn redirect_to(&self, location: &str, hops_so_far: usize) -> Result<Self, UrlError> {
        if hops_so_far >= MAX_REDIRECTS {
            return Err(UrlError::TooManyRedirects);
        }
        let next = self
            .0
            .join(location)
            .map_err(|e| UrlError::Malformed(e.to_string()))?;
        Self::from_url(next)
    }

    pub fn as_url(&self) -> &Url {
        &self.0
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Host without the port, lowercased. This is the per-host politeness key.
    pub fn host_str(&self) -> &str {
        self.0.host_str().unwrap_or_default()
    }

    pub fn into_url(self) -> Url {
        self.0
    }
}

impl fmt::Display for SafeUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}

/// Whether an address is on the public internet.
///
/// Written out explicitly rather than relying on `std` helpers: several of the relevant
/// predicates (`is_shared`, `is_benchmarking`) are still unstable, and being wrong here is a
/// security bug rather than a correctness nit.
/// Loopback escape hatch. See [`SafeUrl::allow_loopback_for_testing`].
static ALLOW_LOOPBACK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(
        a == 0                                  // 0.0.0.0/8   this network
        || a == 10                              // 10/8        private
        || (a == 100 && (64..128).contains(&b)) // 100.64/10   CGNAT
        || a == 127                             // 127/8       loopback
        || (a == 169 && b == 254)               // 169.254/16  link-local (cloud metadata)
        || (a == 172 && (16..32).contains(&b))  // 172.16/12   private
        || (a == 192 && b == 0 && c == 0)       // 192.0.0/24  IETF assignments
        || (a == 192 && b == 0 && c == 2)       // 192.0.2/24  TEST-NET-1
        || (a == 192 && b == 88 && c == 99)     // 192.88.99/24 6to4 relay
        || (a == 192 && b == 168)               // 192.168/16  private
        || (a == 198 && (18..20).contains(&b))  // 198.18/15   benchmarking
        || (a == 198 && b == 51 && c == 100)    // 198.51.100/24 TEST-NET-2
        || (a == 203 && b == 0 && c == 113)     // 203.0.113/24 TEST-NET-3
        || a >= 224
        // 224/4 multicast, 240/4 reserved, 255.255.255.255
    )
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }
    let seg = ip.segments();

    // IPv4-mapped (::ffff:a.b.c.d) and IPv4-compatible. The classic bypass is
    // `http://[::ffff:127.0.0.1]/` — the embedded v4 address must be checked.
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    if let Some(v4) = ip.to_ipv4() {
        return is_public_v4(v4);
    }
    // NAT64 well-known prefix 64:ff9b::/96 also embeds a v4 address.
    if seg[0] == 0x0064 && seg[1] == 0xff9b && seg[2] == 0 && seg[3] == 0 && seg[4] == 0 {
        let v4 = Ipv4Addr::new(
            (seg[6] >> 8) as u8,
            (seg[6] & 0xff) as u8,
            (seg[7] >> 8) as u8,
            (seg[7] & 0xff) as u8,
        );
        return is_public_v4(v4);
    }

    !(
        (seg[0] & 0xfe00) == 0xfc00       // fc00::/7   unique local
        || (seg[0] & 0xffc0) == 0xfe80    // fe80::/10  link-local
        || (seg[0] == 0x2001 && seg[1] == 0x0db8) // 2001:db8::/32 documentation
        || (seg[0] == 0x0100 && seg[1] == 0 && seg[2] == 0 && seg[3] == 0)
        // 100::/64 discard
    )
}

/// DNS resolution plus validation, in one step.
#[cfg(feature = "resolve")]
pub async fn resolve_and_check(url: &SafeUrl) -> Result<Vec<IpAddr>, UrlError> {
    let host = url.host_str().to_string();
    let port = url.as_url().port_or_known_default().unwrap_or(80);
    let addrs: Vec<IpAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| UrlError::Malformed(e.to_string()))?
        .map(|sa| sa.ip())
        .collect();
    if addrs.is_empty() {
        return Err(UrlError::UnsupportedHost);
    }
    url.check_resolved(&addrs)?;
    Ok(addrs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(u: &str) -> UrlError {
        SafeUrl::parse(u).expect_err("should have been rejected")
    }

    #[test]
    fn accepts_ordinary_public_urls() {
        for u in [
            "https://www.elkhabar.com/",
            "http://example.dz/article/123?a=b#frag",
            "https://example.dz:8443/x",
            "https://xn--mgbh0fb.dz/",
        ] {
            assert!(SafeUrl::parse(u).is_ok(), "should accept {u}");
        }
    }

    #[test]
    fn rejects_non_http_schemes() {
        for u in [
            "file:///etc/passwd",
            "ftp://example.dz/",
            "gopher://example.dz/",
        ] {
            assert!(matches!(err(u), UrlError::Scheme(_)), "should reject {u}");
        }
        // `javascript:` and `data:` have no host at all.
        assert!(SafeUrl::parse("javascript:alert(1)").is_err());
        assert!(SafeUrl::parse("data:text/html,hi").is_err());
    }

    #[test]
    fn rejects_credentials_in_url() {
        // `http://evil.com@127.0.0.1/` parses with host 127.0.0.1 — reject before that matters.
        assert_eq!(err("http://user:pass@example.dz/"), UrlError::Credentials);
        assert_eq!(err("http://evil.com@example.dz/"), UrlError::Credentials);
    }

    #[test]
    fn rejects_loopback_literals() {
        for u in ["http://127.0.0.1/", "http://127.1.2.3/", "http://[::1]/"] {
            assert!(
                matches!(err(u), UrlError::PrivateAddress(_)),
                "should reject {u}"
            );
        }
    }

    #[test]
    fn rejects_private_ranges() {
        for u in [
            "http://10.0.0.1/",
            "http://172.16.0.1/",
            "http://172.31.255.254/",
            "http://192.168.1.1/",
            "http://100.64.0.1/",
        ] {
            assert!(
                matches!(err(u), UrlError::PrivateAddress(_)),
                "should reject {u}"
            );
        }
    }

    #[test]
    fn rejects_cloud_metadata_endpoint() {
        // The single most valuable SSRF target.
        assert!(matches!(
            err("http://169.254.169.254/latest/meta-data/"),
            UrlError::PrivateAddress(_)
        ));
    }

    #[test]
    fn rejects_decimal_and_hex_ip_literals() {
        // WHATWG URL parsing normalises these to 127.0.0.1, which we then reject.
        for u in [
            "http://2130706433/",
            "http://0x7f000001/",
            "http://0177.0.0.1/",
        ] {
            assert!(
                matches!(err(u), UrlError::PrivateAddress(_)),
                "should reject {u}"
            );
        }
    }

    #[test]
    fn rejects_ipv4_mapped_ipv6_bypass() {
        // `::ffff:127.0.0.1` is loopback wearing an IPv6 hat.
        assert!(matches!(
            err("http://[::ffff:127.0.0.1]/"),
            UrlError::PrivateAddress(_)
        ));
        assert!(matches!(
            err("http://[::ffff:7f00:1]/"),
            UrlError::PrivateAddress(_)
        ));
        assert!(matches!(
            err("http://[::ffff:169.254.169.254]/"),
            UrlError::PrivateAddress(_)
        ));
    }

    #[test]
    fn rejects_ipv6_private_ranges() {
        for u in [
            "http://[fc00::1]/",
            "http://[fd12:3456::1]/",
            "http://[fe80::1]/",
        ] {
            assert!(
                matches!(err(u), UrlError::PrivateAddress(_)),
                "should reject {u}"
            );
        }
    }

    #[test]
    fn rejects_nat64_embedded_private_v4() {
        assert!(matches!(
            err("http://[64:ff9b::7f00:1]/"),
            UrlError::PrivateAddress(_)
        ));
    }

    #[test]
    fn the_loopback_hatch_is_off_unless_a_test_asks_for_it() {
        // This assertion is the whole safety argument for the escape hatch. It is process-wide,
        // so any test that enables it poisons every other test in the same binary — which is
        // why the only caller is an integration test with its own process, and why nothing in
        // the server binaries calls it at all.
        assert!(
            !SafeUrl::loopback_allowed(),
            "something in this test binary enabled the loopback hatch; every SSRF test after \
             it is now meaningless"
        );
        assert!(SafeUrl::parse("http://127.0.0.1/").is_err());
    }

    #[test]
    fn rejects_localhost_by_name() {
        for u in [
            "http://localhost/",
            "http://LOCALHOST/",
            "http://foo.localhost/",
            "http://db.local/",
        ] {
            assert!(
                matches!(err(u), UrlError::PrivateAddress(_)),
                "should reject {u}"
            );
        }
    }

    #[test]
    fn rejects_unusual_ports() {
        for (u, p) in [
            ("http://example.dz:22/", 22u16),
            ("http://example.dz:6379/", 6379),
            ("http://example.dz:7700/", 7700),
            ("http://example.dz:25/", 25),
        ] {
            assert_eq!(err(u), UrlError::Port(p), "should reject {u}");
        }
    }

    #[test]
    fn dns_rebinding_is_caught_at_resolution() {
        // A public-looking name that resolves inward: structural parse passes, resolved check
        // is what saves us. This is why both stages exist.
        let u = SafeUrl::parse("http://rebind.example.dz/").unwrap();
        assert!(u
            .check_resolved(&["93.184.216.34".parse().unwrap()])
            .is_ok());
        let bad: IpAddr = "127.0.0.1".parse().unwrap();
        assert_eq!(u.check_resolved(&[bad]), Err(UrlError::PrivateAddress(bad)));
    }

    #[test]
    fn rejects_when_any_resolved_address_is_private() {
        // Round-robin DNS with one poisoned record must not slip through.
        let u = SafeUrl::parse("http://mixed.example.dz/").unwrap();
        let addrs: Vec<IpAddr> = vec![
            "93.184.216.34".parse().unwrap(),
            "10.0.0.5".parse().unwrap(),
        ];
        assert!(u.check_resolved(&addrs).is_err());
    }

    #[test]
    fn redirect_targets_are_revalidated() {
        let u = SafeUrl::parse("https://example.dz/a").unwrap();
        assert!(u.redirect_to("/b", 0).is_ok());
        assert!(u.redirect_to("https://other.dz/c", 1).is_ok());
        // The bypass this closes:
        assert!(matches!(
            u.redirect_to("http://169.254.169.254/", 1),
            Err(UrlError::PrivateAddress(_))
        ));
        assert!(matches!(
            u.redirect_to("file:///etc/passwd", 1),
            Err(UrlError::Scheme(_))
        ));
    }

    #[test]
    fn redirect_hop_limit_enforced() {
        let u = SafeUrl::parse("https://example.dz/").unwrap();
        assert_eq!(
            u.redirect_to("/x", MAX_REDIRECTS),
            Err(UrlError::TooManyRedirects)
        );
    }

    #[test]
    fn public_addresses_are_accepted() {
        for ip in ["8.8.8.8", "93.184.216.34", "2001:4860:4860::8888"] {
            assert!(is_public(ip.parse().unwrap()), "{ip} should be public");
        }
    }

    #[test]
    fn boundary_addresses() {
        // Just outside the private ranges — these are real public addresses.
        assert!(is_public("9.255.255.255".parse().unwrap()));
        assert!(is_public("11.0.0.0".parse().unwrap()));
        assert!(is_public("172.15.255.255".parse().unwrap()));
        assert!(is_public("172.32.0.0".parse().unwrap()));
        assert!(is_public("192.167.255.255".parse().unwrap()));
        assert!(is_public("192.169.0.0".parse().unwrap()));
        // Just inside.
        assert!(!is_public("172.16.0.0".parse().unwrap()));
        assert!(!is_public("172.31.255.255".parse().unwrap()));
        assert!(!is_public("100.127.255.255".parse().unwrap()));
        // 100.64/10 ends at 100.127.255.255, so 100.128.0.0 is public again.
        assert!(is_public("100.128.0.0".parse().unwrap()));
        assert!(is_public("100.63.255.255".parse().unwrap()));
    }

    #[test]
    fn host_str_is_the_politeness_key() {
        let u = SafeUrl::parse("https://Example.DZ:8443/path").unwrap();
        assert_eq!(u.host_str(), "example.dz");
    }
}
