//! Post-fetch exclusion, and the blocklists.
//!
//! `robots.txt` governs whether we may **fetch**. These govern whether we may **keep** what we
//! fetched, and whether we should have asked at all. They are separate mechanisms and a crawler
//! that implements only the first is not compliant — a site can allow crawling and still refuse
//! indexing, and that combination is the normal way to let a search engine follow links through a
//! section it does not want listed.
//!
//! # Why `X-Robots-Tag` matters as much as the meta tag
//!
//! A `<meta name="robots" content="noindex">` is only available to something that parses HTML. A
//! PDF, an image, a JSON endpoint — anything without a `<head>` — can only say `noindex` through
//! the HTTP header. Honouring the tag but not the header means honouring the request on exactly
//! the documents where it is easy and ignoring it where it is the site's only option.
//!
//! # The three tiers
//!
//! They are separate because they are answerable to different people and have different urgency:
//!
//! | Tier | Set by | Removed by |
//! |:---|:---|:---|
//! | Global | us, permanently — malware hosts, content we will not carry | a code change |
//! | Takedown | a legal request | the requester, or a lawyer |
//! | Host opt-out | the site operator, self-service | the operator |
//!
//! Collapsing them into one list loses the reason an entry exists, and the reason is what decides
//! whether it can ever be removed. A takedown entry deleted because someone tidied the list is a
//! legal problem, not a bug.

use std::collections::HashSet;

/// Why a document was excluded.
///
/// Distinct variants rather than a boolean, because they need different handling: a `noindex`
/// document may still be crawled for its links, while a blocked host must not be requested again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exclusion {
    /// The document asked not to be indexed. Links may still be followed unless `nofollow`.
    NoIndex,
    /// The document asked that its links not be followed.
    NoFollow,
    /// Both.
    None_,
    /// On a blocklist. The host must not be requested at all.
    Blocked(Tier),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Global,
    Takedown,
    HostOptOut,
}

impl Tier {
    /// Stable label for metrics and logs.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Takedown => "takedown",
            Self::HostOptOut => "host_opt_out",
        }
    }
}

impl Exclusion {
    pub fn blocks_indexing(&self) -> bool {
        matches!(self, Self::NoIndex | Self::None_ | Self::Blocked(_))
    }

    pub fn blocks_links(&self) -> bool {
        matches!(self, Self::NoFollow | Self::None_ | Self::Blocked(_))
    }
}

/// Read an `X-Robots-Tag` header value.
///
/// The header may appear more than once, and each value may carry a user-agent prefix:
///
/// ```text
/// X-Robots-Tag: noindex
/// X-Robots-Tag: xustivebot: noindex, nofollow
/// X-Robots-Tag: googlebot: noindex
/// ```
///
/// A directive addressed to another crawler is **not** ours to obey — treating `googlebot:
/// noindex` as a general instruction would drop documents the site was happy for us to keep.
pub fn from_header(values: &[String], ua_token: &str) -> Option<Exclusion> {
    let mut noindex = false;
    let mut nofollow = false;

    for raw in values {
        for directive in raw.split(',') {
            let directive = directive.trim().to_ascii_lowercase();
            if directive.is_empty() {
                continue;
            }

            // `<agent>: <rule>` — but only when the part before the colon is a user-agent token
            // rather than part of a rule like `unavailable_after: ...`.
            let rule = match directive.split_once(':') {
                Some((agent, rest)) if !agent.contains(' ') && !is_rule(agent) => {
                    let agent = agent.trim();
                    if agent != "*" && !agent.contains(ua_token) {
                        continue;
                    }
                    rest.trim().to_string()
                }
                _ => directive,
            };

            match rule.as_str() {
                "noindex" => noindex = true,
                "nofollow" => nofollow = true,
                "none" => {
                    noindex = true;
                    nofollow = true;
                }
                _ => {}
            }
        }
    }

    match (noindex, nofollow) {
        (true, true) => Some(Exclusion::None_),
        (true, false) => Some(Exclusion::NoIndex),
        (false, true) => Some(Exclusion::NoFollow),
        (false, false) => None,
    }
}

/// Directive names that must not be mistaken for a user-agent prefix.
fn is_rule(token: &str) -> bool {
    matches!(
        token.trim(),
        "noindex"
            | "nofollow"
            | "none"
            | "noarchive"
            | "nosnippet"
            | "noimageindex"
            | "unavailable_after"
            | "max-snippet"
            | "max-image-preview"
            | "max-video-preview"
    )
}

/// The three blocklists.
///
/// Hosts are matched on the registrable name and on any subdomain of it: blocking `example.dz`
/// blocks `www.example.dz`. Anything else would make a blocklist trivially evadable by the site
/// itself and useless as a takedown mechanism.
#[derive(Debug, Default, Clone)]
pub struct Blocklist {
    global: HashSet<String>,
    takedown: HashSet<String>,
    host_opt_out: HashSet<String>,
}

impl Blocklist {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, tier: Tier, host: &str) {
        let host = normalise(host);
        if host.is_empty() {
            return;
        }
        match tier {
            Tier::Global => self.global.insert(host),
            Tier::Takedown => self.takedown.insert(host),
            Tier::HostOptOut => self.host_opt_out.insert(host),
        };
    }

    /// Remove an entry from **one** tier.
    ///
    /// Tier-specific on purpose. A host may be on the takedown list and have opted out
    /// separately; honouring the opt-out withdrawal must not lift the legal one.
    pub fn remove(&mut self, tier: Tier, host: &str) -> bool {
        let host = normalise(host);
        match tier {
            Tier::Global => self.global.remove(&host),
            Tier::Takedown => self.takedown.remove(&host),
            Tier::HostOptOut => self.host_opt_out.remove(&host),
        }
    }

    /// Which tier blocks this host, if any.
    ///
    /// Checked most-permanent first, so a host on several lists reports the one that is hardest to
    /// lift — which is the one an operator asking "why am I not indexed" needs to hear about.
    pub fn blocked(&self, host: &str) -> Option<Tier> {
        let host = normalise(host);
        for (tier, set) in [
            (Tier::Global, &self.global),
            (Tier::Takedown, &self.takedown),
            (Tier::HostOptOut, &self.host_opt_out),
        ] {
            if matches_host(set, &host) {
                return Some(tier);
            }
        }
        None
    }

    pub fn len(&self) -> usize {
        self.global.len() + self.takedown.len() + self.host_opt_out.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Lower-cased, with a leading `www.` and any trailing dot removed.
fn normalise(host: &str) -> String {
    host.trim()
        .trim_end_matches('.')
        .to_ascii_lowercase()
        .trim_start_matches("www.")
        .to_string()
}

/// True when `host` is the blocked name or a subdomain of it.
fn matches_host(set: &HashSet<String>, host: &str) -> bool {
    if set.contains(host) {
        return true;
    }
    // Walk up the labels: `a.b.example.dz` checks `b.example.dz`, then `example.dz`.
    let mut rest = host;
    while let Some((_, parent)) = rest.split_once('.') {
        if set.contains(parent) {
            return true;
        }
        rest = parent;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const UA: &str = "xustivebot";

    fn header(v: &str) -> Option<Exclusion> {
        from_header(&[v.to_string()], UA)
    }

    #[test]
    fn a_bare_directive_applies_to_everyone() {
        assert_eq!(header("noindex"), Some(Exclusion::NoIndex));
        assert_eq!(header("nofollow"), Some(Exclusion::NoFollow));
        assert_eq!(header("none"), Some(Exclusion::None_));
        assert_eq!(header("noindex, nofollow"), Some(Exclusion::None_));
    }

    #[test]
    fn a_directive_addressed_to_us_applies() {
        assert_eq!(header("xustivebot: noindex"), Some(Exclusion::NoIndex));
        assert_eq!(
            header("XustiveBot: noindex, nofollow"),
            Some(Exclusion::None_)
        );
        assert_eq!(header("*: noindex"), Some(Exclusion::NoIndex));
    }

    #[test]
    fn a_directive_addressed_to_another_crawler_does_not() {
        // Obeying it would drop documents the site was happy for us to keep — a silent loss of
        // coverage with nothing to trace it back to.
        assert_eq!(header("googlebot: noindex"), None);
        assert_eq!(header("gptbot: none"), None);
    }

    #[test]
    fn directives_with_their_own_colons_are_not_read_as_agents() {
        // `unavailable_after: 2026-01-01` would otherwise parse as an agent named
        // `unavailable_after`, and the `noindex` beside it would be attributed to that agent and
        // dropped.
        assert_eq!(
            header("noindex, unavailable_after: 25 Jun 2026 15:00:00 GMT"),
            Some(Exclusion::NoIndex)
        );
        assert_eq!(
            header("max-snippet: 20, nofollow"),
            Some(Exclusion::NoFollow)
        );
    }

    #[test]
    fn a_repeated_header_is_combined() {
        // Servers commonly emit one directive per header line.
        let values = vec!["noindex".to_string(), "nofollow".to_string()];
        assert_eq!(from_header(&values, UA), Some(Exclusion::None_));
    }

    #[test]
    fn unrelated_directives_do_not_exclude() {
        assert_eq!(header("noarchive"), None);
        assert_eq!(header("max-image-preview: large"), None);
        assert_eq!(header(""), None);
        assert_eq!(from_header(&[], UA), None);
    }

    #[test]
    fn noindex_and_nofollow_gate_different_things() {
        // A noindex page is still worth crawling for its links; a nofollow page is not.
        assert!(Exclusion::NoIndex.blocks_indexing());
        assert!(!Exclusion::NoIndex.blocks_links());
        assert!(!Exclusion::NoFollow.blocks_indexing());
        assert!(Exclusion::NoFollow.blocks_links());
        assert!(Exclusion::None_.blocks_indexing() && Exclusion::None_.blocks_links());
    }

    #[test]
    fn a_blocked_host_blocks_its_subdomains() {
        // Otherwise a blocklist is evadable by the blocked site itself, which makes it useless as
        // a takedown mechanism.
        let mut b = Blocklist::new();
        b.add(Tier::Takedown, "example.dz");
        assert_eq!(b.blocked("example.dz"), Some(Tier::Takedown));
        assert_eq!(b.blocked("www.example.dz"), Some(Tier::Takedown));
        assert_eq!(b.blocked("news.sub.example.dz"), Some(Tier::Takedown));
        assert_eq!(b.blocked("notexample.dz"), None);
        assert_eq!(b.blocked("example.dz.attacker.com"), None);
    }

    #[test]
    fn matching_ignores_case_www_and_a_trailing_dot() {
        let mut b = Blocklist::new();
        b.add(Tier::Global, "WWW.Example.DZ.");
        assert_eq!(b.blocked("example.dz"), Some(Tier::Global));
        assert_eq!(b.blocked("EXAMPLE.dz"), Some(Tier::Global));
    }

    #[test]
    fn the_most_permanent_tier_is_reported() {
        // An operator asking why they are not indexed needs to hear about the entry that is
        // hardest to lift, not whichever was checked first.
        let mut b = Blocklist::new();
        b.add(Tier::HostOptOut, "example.dz");
        b.add(Tier::Takedown, "example.dz");
        assert_eq!(b.blocked("example.dz"), Some(Tier::Takedown));
    }

    #[test]
    fn removing_an_opt_out_does_not_lift_a_takedown() {
        // The reason the tiers are separate. A host that withdraws its opt-out is not thereby
        // released from a legal order.
        let mut b = Blocklist::new();
        b.add(Tier::Takedown, "example.dz");
        b.add(Tier::HostOptOut, "example.dz");
        assert!(b.remove(Tier::HostOptOut, "example.dz"));
        assert_eq!(b.blocked("example.dz"), Some(Tier::Takedown));
    }

    #[test]
    fn tiers_have_stable_labels() {
        for t in [Tier::Global, Tier::Takedown, Tier::HostOptOut] {
            let s = t.as_str();
            assert!(!s.is_empty() && !s.contains(' '));
        }
    }

    #[test]
    fn malformed_input_is_ignored_rather_than_panicking() {
        let mut b = Blocklist::new();
        for h in ["", " ", ".", "..", "a..b"] {
            b.add(Tier::Global, h);
            let _ = b.blocked(h);
        }
        for v in [":", "::", ": noindex", "noindex:", ","] {
            let _ = header(v);
        }
    }
}
