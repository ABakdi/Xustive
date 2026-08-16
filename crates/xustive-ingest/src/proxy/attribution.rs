//! Failure attribution ([[Proxy Manager]] §4.5).
//!
//! A failure means one of four things: the proxy is bad, the host is down, the identity is flagged,
//! or an ASN's reputation is collapsing. Getting this wrong produces the classic spiral where one
//! dead host quarantines an entire pool — every proxy that touched it looks bad. So the rules are
//! evaluated in an order that resolves the ambiguity toward the *shared* cause first: a pattern that
//! implicates a host or an ASN is preferred over one that would blame an individual proxy, because
//! punishing the proxies for a host's outage is the expensive mistake.

use std::collections::{HashMap, HashSet};

use super::Outcome;

/// One failed (or, for context, successful) request. Successes are included because the identity
/// rule needs to know a proxy is healthy *elsewhere* while one identity on it is challenged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureEvent {
    pub proxy_id: String,
    pub host: String,
    pub asn: String,
    /// The identity in use, for platform requests. `None` for open-web `direct`.
    pub identity: Option<String>,
    pub outcome: Outcome,
    pub at_ms: i64,
}

/// Who to blame for a cluster of failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blame {
    /// The host is down — open its breaker, leave the proxies alone.
    Host(String),
    /// The proxy is bad — quarantine it.
    Proxy(String),
    /// This one identity is flagged — quarantine the identity, not the proxy.
    Identity(String),
    /// An ASN's reputation is collapsing — drain it and redistribute.
    Asn(String),
}

/// ≥ this many distinct proxies failing on one host means the host is the problem (§4.5).
const HOST_PROXY_THRESHOLD: usize = 3;
/// One proxy failing across ≥ this many distinct hosts means the proxy is the problem.
const PROXY_HOST_THRESHOLD: usize = 3;
/// ≥ this many distinct identities challenged on one ASN means the ASN is the problem.
const ASN_IDENTITY_THRESHOLD: usize = 3;

/// Attribute a window of events to a cause, or `None` if nothing rises to a pattern.
///
/// Only events within `window_ms` of `now_ms` are considered — attribution is about a burst, not
/// history. The order is deliberate (§4.5): **host, then ASN, then proxy, then identity**. Host and
/// ASN are shared causes and must win over the proxy rule, or one outage quarantines the pool; the
/// single-identity rule is last because it is the most specific and should not swallow an ASN-wide
/// trend.
pub fn attribute(events: &[FailureEvent], now_ms: i64, window_ms: i64) -> Option<Blame> {
    let recent: Vec<&FailureEvent> = events
        .iter()
        .filter(|e| now_ms - e.at_ms <= window_ms && now_ms - e.at_ms >= 0)
        .collect();
    let failures: Vec<&FailureEvent> = recent
        .iter()
        .copied()
        .filter(|e| !e.outcome.is_success())
        .collect();

    // 1. Host: a host with failures from ≥ N distinct proxies is down. Checked first so its outage
    //    never gets blamed on the proxies that happened to hit it.
    let mut proxies_per_host: HashMap<&str, HashSet<&str>> = HashMap::new();
    for e in &failures {
        proxies_per_host
            .entry(e.host.as_str())
            .or_default()
            .insert(e.proxy_id.as_str());
    }
    if let Some((host, _)) = proxies_per_host
        .iter()
        .find(|(_, ps)| ps.len() >= HOST_PROXY_THRESHOLD)
    {
        return Some(Blame::Host(host.to_string()));
    }

    // 2. ASN: challenges spanning ≥ N distinct identities on one ASN is a reputation collapse.
    let mut identities_per_asn: HashMap<&str, HashSet<&str>> = HashMap::new();
    for e in &failures {
        if matches!(e.outcome, Outcome::Challenged | Outcome::Banned) {
            if let Some(id) = &e.identity {
                identities_per_asn
                    .entry(e.asn.as_str())
                    .or_default()
                    .insert(id.as_str());
            }
        }
    }
    if let Some((asn, _)) = identities_per_asn
        .iter()
        .find(|(_, ids)| ids.len() >= ASN_IDENTITY_THRESHOLD)
    {
        return Some(Blame::Asn(asn.to_string()));
    }

    // 3. Proxy: one proxy failing across ≥ N distinct hosts is a bad proxy.
    let mut hosts_per_proxy: HashMap<&str, HashSet<&str>> = HashMap::new();
    for e in &failures {
        hosts_per_proxy
            .entry(e.proxy_id.as_str())
            .or_default()
            .insert(e.host.as_str());
    }
    if let Some((proxy, _)) = hosts_per_proxy
        .iter()
        .find(|(_, hs)| hs.len() >= PROXY_HOST_THRESHOLD)
    {
        return Some(Blame::Proxy(proxy.to_string()));
    }

    // 4. Identity: one identity challenged while its proxy is demonstrably healthy elsewhere (a
    //    success on another host in the window). The most specific pattern, so it is last.
    for e in &failures {
        if !matches!(e.outcome, Outcome::Challenged) {
            continue;
        }
        let Some(id) = &e.identity else { continue };
        let proxy_ok_elsewhere = recent
            .iter()
            .any(|o| o.proxy_id == e.proxy_id && o.host != e.host && o.outcome.is_success());
        if proxy_ok_elsewhere {
            return Some(Blame::Identity(id.clone()));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(proxy: &str, host: &str, asn: &str, id: Option<&str>, outcome: Outcome) -> FailureEvent {
        FailureEvent {
            proxy_id: proxy.into(),
            host: host.into(),
            asn: asn.into(),
            identity: id.map(String::from),
            outcome,
            at_ms: 1000,
        }
    }

    #[test]
    fn three_proxies_failing_one_host_blames_the_host() {
        let events = [
            ev("p1", "elkhabar.dz", "as1", None, Outcome::Timeout),
            ev("p2", "elkhabar.dz", "as2", None, Outcome::Timeout),
            ev("p3", "elkhabar.dz", "as3", None, Outcome::Refused),
        ];
        assert_eq!(
            attribute(&events, 1000, 60_000),
            Some(Blame::Host("elkhabar.dz".into()))
        );
    }

    #[test]
    fn one_proxy_failing_three_hosts_blames_the_proxy() {
        let events = [
            ev("p1", "a.dz", "as1", None, Outcome::Refused),
            ev("p1", "b.dz", "as1", None, Outcome::Refused),
            ev("p1", "c.dz", "as1", None, Outcome::Timeout),
        ];
        assert_eq!(
            attribute(&events, 1000, 60_000),
            Some(Blame::Proxy("p1".into()))
        );
    }

    #[test]
    fn a_host_outage_is_not_blamed_on_its_proxies() {
        // Three proxies, one host: without host-first ordering, each proxy also "fails on hosts" and
        // the pool would quarantine. Host must win.
        let events = [
            ev("p1", "down.dz", "as1", None, Outcome::Timeout),
            ev("p2", "down.dz", "as2", None, Outcome::Timeout),
            ev("p3", "down.dz", "as3", None, Outcome::Timeout),
        ];
        assert!(matches!(
            attribute(&events, 1000, 60_000),
            Some(Blame::Host(_))
        ));
    }

    #[test]
    fn challenges_across_identities_on_one_asn_blame_the_asn() {
        let events = [
            ev(
                "p1",
                "instagram.com",
                "asX",
                Some("id1"),
                Outcome::Challenged,
            ),
            ev(
                "p2",
                "instagram.com",
                "asX",
                Some("id2"),
                Outcome::Challenged,
            ),
            ev(
                "p3",
                "instagram.com",
                "asX",
                Some("id3"),
                Outcome::Challenged,
            ),
        ];
        // Host rule needs distinct proxies failing — here the challenge also trips host (3 proxies,
        // 1 host). Host is checked first, so this asserts the realistic case where the hosts differ.
        let spread = [
            ev(
                "p1",
                "instagram.com/a",
                "asX",
                Some("id1"),
                Outcome::Challenged,
            ),
            ev(
                "p2",
                "instagram.com/b",
                "asX",
                Some("id2"),
                Outcome::Challenged,
            ),
            ev(
                "p3",
                "instagram.com/c",
                "asX",
                Some("id3"),
                Outcome::Challenged,
            ),
        ];
        let _ = events;
        assert_eq!(
            attribute(&spread, 1000, 60_000),
            Some(Blame::Asn("asX".into()))
        );
    }

    #[test]
    fn one_identity_challenged_while_its_proxy_works_elsewhere_blames_the_identity() {
        let events = [
            ev(
                "p1",
                "instagram.com/x",
                "as1",
                Some("flagged"),
                Outcome::Challenged,
            ),
            // Same proxy, different host, success → the proxy is fine; the identity is the problem.
            ev("p1", "instagram.com/y", "as1", Some("other"), Outcome::Ok),
        ];
        assert_eq!(
            attribute(&events, 1000, 60_000),
            Some(Blame::Identity("flagged".into()))
        );
    }

    #[test]
    fn events_outside_the_window_are_ignored() {
        let events = [
            ev("p1", "a.dz", "as1", None, Outcome::Refused),
            ev("p1", "b.dz", "as1", None, Outcome::Refused),
            FailureEvent {
                at_ms: 0, // 1000ms ago with a 500ms window — excluded
                ..ev("p1", "c.dz", "as1", None, Outcome::Refused)
            },
        ];
        // Only two in-window failures → no proxy pattern.
        assert_eq!(attribute(&events, 1000, 500), None);
    }
}
