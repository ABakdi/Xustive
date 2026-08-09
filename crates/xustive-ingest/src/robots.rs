//! `robots.txt` parsing and per-host politeness.
//!
//! This is the component that keeps Xustive a good citizen of the Algerian web, most of which
//! runs on modest hosting where an aggressive crawler is indistinguishable from an outage.
//!
//! Being polite is not only ethics. Sites that notice us block us, and a blocked source is a
//! permanent hole in the index.
//!
//! # Failing closed
//!
//! An unreachable `robots.txt` is **not** permission. A 5xx or a timeout is treated as a full
//! disallow, and so is a 401 or 403 — the site is refusing us. Only a 404 means "no restrictions".
//! Implementations that fail open here are the ones that end up in abuse reports.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Our identity. Published at `/bot` so anyone can see who we are and how to block us.
pub const USER_AGENT: &str = "XustiveBot/1.0 (+https://xustive.dz/bot; Algerian search engine)";

/// The token sites use to address us specifically in `robots.txt`.
pub const UA_TOKEN: &str = "xustivebot";

/// Applied when `robots.txt` states no `Crawl-delay`.
pub const DEFAULT_CRAWL_DELAY: Duration = Duration::from_millis(1500);

/// Ceiling on a declared `Crawl-delay`. Beyond this a site is effectively uncrawlable, and we
/// reduce visit frequency instead of blocking a worker for minutes.
pub const MAX_CRAWL_DELAY: Duration = Duration::from_secs(60);

const MAX_ROBOTS_BYTES: usize = 512 * 1024;

/// One `User-agent` group.
#[derive(Debug, Clone, Default)]
struct Group {
    allow: Vec<String>,
    disallow: Vec<String>,
    crawl_delay: Option<Duration>,
}

/// Parsed rules for one host.
#[derive(Debug, Clone)]
pub struct Robots {
    group: Group,
    pub sitemaps: Vec<String>,
    /// Set when the file could not be fetched, which means disallow everything.
    blanket_disallow: bool,
}

impl Robots {
    /// Rules that permit everything. Used for a 404, which genuinely means no restrictions.
    pub fn permissive() -> Self {
        Self {
            group: Group::default(),
            sitemaps: Vec::new(),
            blanket_disallow: false,
        }
    }

    /// Rules that permit nothing. Used when `robots.txt` could not be read.
    pub fn deny_all() -> Self {
        Self {
            group: Group {
                disallow: vec!["/".into()],
                ..Default::default()
            },
            sitemaps: Vec::new(),
            blanket_disallow: true,
        }
    }

    pub fn is_deny_all(&self) -> bool {
        self.blanket_disallow
    }

    /// Parse `robots.txt`.
    ///
    /// A group addressed to `xustivebot` wins over `*` entirely — that is the standard's
    /// precedence rule, and a site that names us has been specific on purpose.
    pub fn parse(text: &str) -> Self {
        // Truncated on a character boundary, not a byte offset. Slicing a `&str` mid-character
        // panics, and this file comes from a server we do not control — an Arabic robots.txt long
        // enough to hit the cap crashed the parser, and a site could do that deliberately.
        let text = if text.len() > MAX_ROBOTS_BYTES {
            let mut end = MAX_ROBOTS_BYTES;
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            &text[..end]
        } else {
            text
        };

        let mut specific = Group::default();
        let mut wildcard = Group::default();
        let mut sitemaps = Vec::new();

        // Which groups the current run of `User-agent:` lines addresses.
        let mut in_specific = false;
        let mut in_wildcard = false;
        // A rule line closes the preceding run of user-agent lines.
        let mut last_was_agent = false;

        for raw in text.lines() {
            // Strip comments, and the BOM some servers emit.
            let line = raw.trim_start_matches('\u{FEFF}');
            let line = match line.find('#') {
                Some(i) => &line[..i],
                None => line,
            }
            .trim();
            if line.is_empty() {
                continue;
            }

            let Some((field, value)) = line.split_once(':') else {
                continue;
            };
            let field = field.trim().to_ascii_lowercase();
            let value = value.trim();

            match field.as_str() {
                "user-agent" => {
                    if !last_was_agent {
                        in_specific = false;
                        in_wildcard = false;
                    }
                    let ua = value.to_ascii_lowercase();
                    if ua == "*" {
                        in_wildcard = true;
                    } else if ua.contains(UA_TOKEN) {
                        in_specific = true;
                    }
                    last_was_agent = true;
                }
                "allow" | "disallow" | "crawl-delay" => {
                    last_was_agent = false;
                    let apply = |g: &mut Group| match field.as_str() {
                        "allow" if !value.is_empty() => g.allow.push(value.to_string()),
                        // `Disallow:` with an empty value means allow everything, per the RFC.
                        "disallow" if !value.is_empty() => g.disallow.push(value.to_string()),
                        "crawl-delay" => {
                            if let Ok(secs) = value.parse::<f64>() {
                                if secs.is_finite() && secs > 0.0 {
                                    g.crawl_delay = Some(Duration::from_secs_f64(secs.min(3600.0)));
                                }
                            }
                        }
                        _ => {}
                    };
                    if in_specific {
                        apply(&mut specific);
                    }
                    if in_wildcard {
                        apply(&mut wildcard);
                    }
                }
                "sitemap" => {
                    last_was_agent = false;
                    if !value.is_empty() {
                        sitemaps.push(value.to_string());
                    }
                }
                _ => last_was_agent = false,
            }
        }

        // A group naming us replaces the wildcard group entirely rather than merging with it.
        let group = if specific.allow.is_empty()
            && specific.disallow.is_empty()
            && specific.crawl_delay.is_none()
        {
            wildcard
        } else {
            specific
        };

        Self {
            group,
            sitemaps,
            blanket_disallow: false,
        }
    }

    /// Whether a path may be fetched.
    ///
    /// Longest-match wins, and `Allow` beats `Disallow` at equal length — that is the standard's
    /// tie-break and it is what lets a site carve an exception out of a broad block.
    pub fn allows(&self, path: &str) -> bool {
        let best_allow = self
            .group
            .allow
            .iter()
            .filter(|p| matches_pattern(p, path))
            .map(|p| p.len())
            .max();
        let best_disallow = self
            .group
            .disallow
            .iter()
            .filter(|p| matches_pattern(p, path))
            .map(|p| p.len())
            .max();

        match (best_allow, best_disallow) {
            (Some(a), Some(d)) => a >= d,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => true,
        }
    }

    /// The delay this host asks for, bounded.
    pub fn crawl_delay(&self) -> Duration {
        self.group
            .crawl_delay
            .unwrap_or(DEFAULT_CRAWL_DELAY)
            .min(MAX_CRAWL_DELAY)
    }
}

/// `robots.txt` path matching: `*` is any run of characters, `$` anchors the end.
fn matches_pattern(pattern: &str, path: &str) -> bool {
    if !pattern.contains('*') && !pattern.ends_with('$') {
        return path.starts_with(pattern);
    }

    let anchored = pattern.ends_with('$');
    let pattern = pattern.strip_suffix('$').unwrap_or(pattern);
    let parts: Vec<&str> = pattern.split('*').collect();

    let mut pos = 0usize;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !path[pos..].starts_with(part) {
                return false;
            }
            pos += part.len();
        } else {
            match path[pos..].find(part) {
                Some(idx) => pos += idx + part.len(),
                None => return false,
            }
        }
    }

    if anchored {
        // The final literal must land exactly at the end.
        match parts.last() {
            Some(last) if !last.is_empty() => path.len() == pos,
            _ => true,
        }
    } else {
        true
    }
}

/// Per-host politeness state: cached rules and when we may next fetch.
///
/// Concurrency per host is one, enforced by the scheduler holding this lock across the wait.
#[derive(Debug, Default)]
pub struct Politeness {
    hosts: HashMap<String, HostState>,
}

#[derive(Debug)]
struct HostState {
    robots: Robots,
    fetched_at: Instant,
    next_allowed: Instant,
    /// Grown by 429 and 503 responses, shrunk slowly by success.
    adaptive_delay: Duration,
}

impl Politeness {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn has_rules_for(&self, host: &str) -> bool {
        self.hosts.contains_key(host)
    }

    pub fn set_rules(&mut self, host: &str, robots: Robots) {
        let delay = robots.crawl_delay();
        self.hosts.insert(
            host.to_string(),
            HostState {
                robots,
                fetched_at: Instant::now(),
                next_allowed: Instant::now(),
                adaptive_delay: delay,
            },
        );
    }

    pub fn rules(&self, host: &str) -> Option<&Robots> {
        self.hosts.get(host).map(|s| &s.robots)
    }

    /// Whether the cached rules are older than `ttl` and should be refetched.
    pub fn rules_stale(&self, host: &str, ttl: Duration) -> bool {
        match self.hosts.get(host) {
            Some(s) => s.fetched_at.elapsed() > ttl,
            None => true,
        }
    }

    pub fn allows(&self, host: &str, path: &str) -> bool {
        match self.hosts.get(host) {
            Some(s) => s.robots.allows(path),
            // No rules cached means we have not checked yet. Refuse rather than assume.
            None => false,
        }
    }

    /// How long to wait before the next request to this host.
    pub fn wait_for(&self, host: &str) -> Duration {
        match self.hosts.get(host) {
            Some(s) => s.next_allowed.saturating_duration_since(Instant::now()),
            None => Duration::ZERO,
        }
    }

    /// Record that a request was made, and schedule the next permitted time.
    pub fn record_fetch(&mut self, host: &str) {
        if let Some(s) = self.hosts.get_mut(host) {
            s.next_allowed = Instant::now() + s.adaptive_delay;
        }
    }

    /// Adjust pacing from what the host actually did.
    ///
    /// Delays grow fast and shrink slowly, deliberately: overshooting costs us a little
    /// throughput, undershooting costs us the site.
    pub fn observe(&mut self, host: &str, status: u16, retry_after: Option<Duration>) {
        let Some(s) = self.hosts.get_mut(host) else {
            return;
        };
        match status {
            429 => {
                s.adaptive_delay = retry_after
                    .unwrap_or(s.adaptive_delay * 4)
                    .max(Duration::from_secs(60))
                    .min(Duration::from_secs(600));
            }
            500..=599 => {
                s.adaptive_delay = (s.adaptive_delay * 2).min(Duration::from_secs(300));
            }
            200..=399 => {
                let floor = s.robots.crawl_delay();
                let relaxed = s.adaptive_delay.mul_f32(0.9);
                s.adaptive_delay = if relaxed < floor { floor } else { relaxed };
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_robots_allows_everything() {
        let r = Robots::parse("");
        assert!(r.allows("/"));
        assert!(r.allows("/anything/at/all"));
    }

    #[test]
    fn wildcard_group_applies() {
        let r = Robots::parse("User-agent: *\nDisallow: /admin\n");
        assert!(!r.allows("/admin"));
        assert!(!r.allows("/admin/users"));
        assert!(r.allows("/articles"));
    }

    #[test]
    fn empty_disallow_means_allow_everything() {
        // `Disallow:` with no value is the RFC's way of saying "no restrictions".
        let r = Robots::parse("User-agent: *\nDisallow:\n");
        assert!(r.allows("/anything"));
    }

    #[test]
    fn a_group_naming_us_replaces_the_wildcard() {
        let txt = "User-agent: *\nDisallow: /\n\nUser-agent: XustiveBot\nDisallow: /private\n";
        let r = Robots::parse(txt);
        assert!(
            r.allows("/articles"),
            "our own group should not inherit the wildcard block"
        );
        assert!(!r.allows("/private"));
    }

    #[test]
    fn longest_match_wins_and_allow_breaks_ties() {
        let txt = "User-agent: *\nDisallow: /articles\nAllow: /articles/public\n";
        let r = Robots::parse(txt);
        assert!(!r.allows("/articles/secret"));
        assert!(
            r.allows("/articles/public/x"),
            "the longer Allow should win"
        );
    }

    #[test]
    fn wildcards_and_anchors() {
        let r = Robots::parse("User-agent: *\nDisallow: /*.pdf$\nDisallow: /tmp/*/cache\n");
        assert!(!r.allows("/files/report.pdf"));
        assert!(
            r.allows("/files/report.pdf.html"),
            "$ should anchor to the end"
        );
        assert!(!r.allows("/tmp/abc/cache"));
        assert!(r.allows("/tmp/abc/other"));
    }

    #[test]
    fn consecutive_user_agent_lines_share_one_group() {
        let txt = "User-agent: badbot\nUser-agent: XustiveBot\nDisallow: /nope\n";
        let r = Robots::parse(txt);
        assert!(!r.allows("/nope"));
    }

    #[test]
    fn comments_bom_and_crlf_are_tolerated() {
        let txt = "\u{FEFF}# a comment\r\nUser-agent: *\r\nDisallow: /x # trailing\r\n";
        let r = Robots::parse(txt);
        assert!(!r.allows("/x"));
    }

    #[test]
    fn sitemaps_are_collected() {
        let txt = "Sitemap: https://a.dz/sitemap.xml\nUser-agent: *\nSitemap: https://a.dz/n.xml\n";
        let r = Robots::parse(txt);
        assert_eq!(r.sitemaps.len(), 2);
    }

    #[test]
    fn crawl_delay_is_read_and_bounded() {
        assert_eq!(
            Robots::parse("User-agent: *\nCrawl-delay: 5\n").crawl_delay(),
            Duration::from_secs(5)
        );
        // A five-minute delay would block a worker; we cap and reduce frequency instead.
        assert_eq!(
            Robots::parse("User-agent: *\nCrawl-delay: 300\n").crawl_delay(),
            MAX_CRAWL_DELAY
        );
        assert_eq!(Robots::parse("").crawl_delay(), DEFAULT_CRAWL_DELAY);
    }

    #[test]
    fn deny_all_blocks_everything() {
        let r = Robots::deny_all();
        assert!(!r.allows("/"));
        assert!(!r.allows("/anything"));
        assert!(r.is_deny_all());
    }

    #[test]
    fn unchecked_host_is_refused() {
        // Not having checked robots.txt is not the same as being allowed.
        let p = Politeness::new();
        assert!(!p.allows("example.dz", "/"));
        assert!(p.rules_stale("example.dz", Duration::from_secs(1)));
    }

    #[test]
    fn pacing_is_scheduled_after_a_fetch() {
        let mut p = Politeness::new();
        p.set_rules("a.dz", Robots::parse("User-agent: *\nCrawl-delay: 2\n"));
        assert_eq!(p.wait_for("a.dz"), Duration::ZERO);
        p.record_fetch("a.dz");
        let w = p.wait_for("a.dz");
        assert!(
            w > Duration::from_millis(1900) && w <= Duration::from_secs(2),
            "got {w:?}"
        );
    }

    #[test]
    fn rate_limiting_grows_the_delay_sharply() {
        let mut p = Politeness::new();
        p.set_rules("a.dz", Robots::parse(""));
        p.observe("a.dz", 429, None);
        p.record_fetch("a.dz");
        assert!(
            p.wait_for("a.dz") >= Duration::from_secs(59),
            "a 429 must back off hard"
        );
    }

    #[test]
    fn retry_after_is_honoured_exactly() {
        let mut p = Politeness::new();
        p.set_rules("a.dz", Robots::parse(""));
        p.observe("a.dz", 429, Some(Duration::from_secs(120)));
        p.record_fetch("a.dz");
        assert!(p.wait_for("a.dz") > Duration::from_secs(119));
    }

    #[test]
    fn success_relaxes_slowly_and_never_below_the_declared_delay() {
        let mut p = Politeness::new();
        p.set_rules("a.dz", Robots::parse("User-agent: *\nCrawl-delay: 2\n"));
        for _ in 0..50 {
            p.observe("a.dz", 200, None);
        }
        p.record_fetch("a.dz");
        assert!(
            p.wait_for("a.dz") >= Duration::from_millis(1900),
            "must never go below the host's declared Crawl-delay"
        );
    }
}
