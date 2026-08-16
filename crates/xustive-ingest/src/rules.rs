//! Per-domain extraction rules.
//!
//! Generic extraction reads JSON-LD, Open Graph, `<time>` and semantic class names. That covers
//! most publishers and misses the ones that matter here: aps.dz renders its publication date as
//! `<span class="text-xs">الأربعاء 05 أوت 2026 13:37</span>` — a Tailwind utility class with no
//! machine-readable markup anywhere on the page.
//!
//! No amount of generic cleverness finds that. A selector does, and 25 of 40 aps.dz articles had
//! no date at all before this existed. Freshness ranking with nothing to rank on is the largest
//! single cost of a missing date.
//!
//! Rules are tried **before** generic extraction, never after. A publisher shipping correct
//! metadata does not need a rule, and one that is not is telling us its markup is unreliable.

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct DomainRule {
    pub host: String,
    /// CSS selector for the publication date's containing element.
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    /// Why this rule exists. Read by whoever has to fix it when the publisher changes template.
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct RuleFile {
    #[serde(default)]
    domain: Vec<DomainRule>,
}

/// Rules indexed by host.
#[derive(Debug, Clone, Default)]
pub struct Rules {
    by_host: HashMap<String, DomainRule>,
}

impl Rules {
    /// Parse a rules document.
    pub fn parse(toml: &str) -> Result<Self, String> {
        let file: RuleFile = toml::from_str(toml).map_err(|e| e.to_string())?;
        let mut by_host = HashMap::new();
        for rule in file.domain {
            let host = normalise_host(&rule.host);
            if host.is_empty() {
                return Err("a rule has an empty host".into());
            }
            // A duplicate host means one of the two rules silently never applies, and which one
            // depends on file order. Better to refuse the file than to guess.
            if by_host.insert(host.clone(), rule).is_some() {
                return Err(format!("duplicate rule for {host}"));
            }
        }
        Ok(Self { by_host })
    }

    /// Load from disk, falling back to none.
    ///
    /// A missing rules file is not an error: generic extraction still works, just less well. A
    /// crawler that refuses to start because an optional tuning file is absent is worse than one
    /// that extracts a few fewer dates.
    pub fn load(path: &str) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => match Self::parse(&text) {
                Ok(rules) => {
                    tracing::info!(path, count = rules.len(), "loaded per-domain parser rules");
                    rules
                }
                Err(e) => {
                    // Logged loudly rather than swallowed: a malformed rules file means every
                    // rule stops applying, and the symptom is dates quietly disappearing.
                    tracing::error!(path, error = %e, "parser rules are malformed; ignoring all");
                    Self::default()
                }
            },
            Err(_) => {
                tracing::info!(
                    path,
                    "no per-domain parser rules; using generic extraction only"
                );
                Self::default()
            }
        }
    }

    /// The rule for a host, matching a subdomain to its parent.
    ///
    /// `www.aps.dz` and `m.aps.dz` are the same publisher with the same template, and writing a
    /// rule per subdomain would be three copies to keep in step.
    pub fn for_host(&self, host: &str) -> Option<&DomainRule> {
        let host = normalise_host(host);
        if let Some(rule) = self.by_host.get(&host) {
            return Some(rule);
        }
        // Walk up the labels: m.aps.dz → aps.dz → dz.
        let mut rest = host.as_str();
        while let Some((_, parent)) = rest.split_once('.') {
            if let Some(rule) = self.by_host.get(parent) {
                return Some(rule);
            }
            rest = parent;
        }
        None
    }

    pub fn len(&self) -> usize {
        self.by_host.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_host.is_empty()
    }

    pub fn hosts(&self) -> impl Iterator<Item = &String> {
        self.by_host.keys()
    }
}

/// Text the first element matching `selector` would capture, collapsed to single spaces — the same
/// shape a rule captures at extraction time. `None` for an invalid selector or no match. Exposed so
/// the rule-authoring tool (`xustive-cli parse-check`) can verify a candidate selector against real
/// HTML before it is written into a rule, rather than discovering it never matched at crawl time.
pub fn capture_selector(html: &str, selector: &str) -> Option<String> {
    let sel = scraper::Selector::parse(selector).ok()?;
    let doc = scraper::Html::parse_document(html);
    let el = doc.select(&sel).next()?;
    let collapsed = el
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!collapsed.is_empty()).then_some(collapsed)
}

fn normalise_host(host: &str) -> String {
    host.trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.")
        .split('/')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[[domain]]
host = "aps.dz"
date = "span.text-xs"
title = "h1"

[[domain]]
host = "elkhabar.com"
title = "h1.title"
"#;

    #[test]
    fn rules_parse_and_index_by_host() {
        let rules = Rules::parse(SAMPLE).unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(
            rules.for_host("aps.dz").unwrap().date.as_deref(),
            Some("span.text-xs")
        );
    }

    #[test]
    fn a_subdomain_uses_its_parents_rule() {
        // www.aps.dz and m.aps.dz are the same publisher with the same template. A rule per
        // subdomain is three copies to keep in step.
        let rules = Rules::parse(SAMPLE).unwrap();
        for host in ["www.aps.dz", "m.aps.dz", "https://www.aps.dz"] {
            assert!(rules.for_host(host).is_some(), "{host} should match aps.dz");
        }
    }

    #[test]
    fn an_unknown_host_has_no_rule() {
        let rules = Rules::parse(SAMPLE).unwrap();
        assert!(rules.for_host("example.com").is_none());
        // And must not match on a shared suffix alone.
        assert!(rules.for_host("notaps.dz").is_none());
    }

    #[test]
    fn a_duplicate_host_is_refused() {
        // One of the two would silently never apply, and which one would depend on file order.
        let dup = "[[domain]]\nhost = \"aps.dz\"\n\n[[domain]]\nhost = \"www.aps.dz\"\n";
        assert!(Rules::parse(dup).is_err());
    }

    #[test]
    fn capture_selector_returns_collapsed_text_or_none() {
        let html = r#"<html><body>
            <h1>  البلاد   :  خبر  </h1>
            <article><p>نص المقال.</p></article>
        </body></html>"#;
        // Whitespace collapsed to single spaces, like a rule captures at extraction time.
        assert_eq!(capture_selector(html, "h1").as_deref(), Some("البلاد : خبر"));
        assert_eq!(
            capture_selector(html, "article").as_deref(),
            Some("نص المقال.")
        );
        // No match and an invalid selector both yield None, not a panic.
        assert!(capture_selector(html, ".nope").is_none());
        assert!(capture_selector(html, ">>bad<<").is_none());
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        // A crawler that refuses to start because an optional tuning file is absent is worse
        // than one that extracts a few fewer dates.
        assert!(Rules::load("/nonexistent/domains.toml").is_empty());
    }

    #[test]
    fn the_shipped_rules_are_well_formed() {
        // They ship with the product, so a typo in them is a shipped defect.
        let Ok(text) = std::fs::read_to_string("../../data/parsers/domains.toml") else {
            return; // Run from elsewhere; the parse tests above still cover the logic.
        };
        let rules = Rules::parse(&text).expect("shipped rules must parse");
        assert!(rules.len() >= 10, "only {} rules", rules.len());

        for host in rules.hosts() {
            assert!(!host.starts_with("www."), "{host} should be normalised");
            assert!(host.contains('.'), "{host} is not a host");
        }
        // The rule this whole module exists for.
        let aps = rules.for_host("aps.dz").expect("aps.dz must have a rule");
        assert!(
            aps.date.is_some(),
            "aps.dz needs a date selector or it has no dates at all"
        );
    }

    #[test]
    fn every_selector_is_syntactically_valid() {
        // An invalid selector fails silently at extraction time, so the field simply never
        // populates and nobody finds out why.
        let Ok(text) = std::fs::read_to_string("../../data/parsers/domains.toml") else {
            return;
        };
        let rules = Rules::parse(&text).unwrap();
        for host in rules.hosts() {
            let rule = rules.for_host(host).unwrap();
            for (field, selector) in [
                ("date", &rule.date),
                ("title", &rule.title),
                ("body", &rule.body),
            ] {
                if let Some(s) = selector {
                    assert!(
                        scraper::Selector::parse(s).is_ok(),
                        "{host}: {field} selector {s:?} does not parse"
                    );
                }
            }
        }
    }
}
