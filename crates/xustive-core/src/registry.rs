//! The data sources registry store (M2-T11.1, M2-T11.2).
//!
//! What Xustive crawls, under which policy, and on whose authority. The registry is the product:
//! the value is in crawling the right few thousand sources well, not the whole web badly, so this
//! is a curated editorial artefact as much as a config file. The record itself is
//! [`crate::model::Source`]; this module is the *store* around it — load, validate, mutate, export.
//!
//! # It is data, versioned in git
//!
//! The store is a JSON-Lines file — one [`Source`] per line, so a change is a one-line git diff and
//! a reviewer can see exactly what an operator added or disabled. The live registry is loaded from
//! it and exported back to it on every change, which is what keeps file and running state in step.
//! JSON Lines rather than one big JSON array precisely so the diff stays readable at a thousand
//! sources: adding a source in the middle is a one-line insertion, not a whole-file reshuffle.
//!
//! # Two rules enforced here, not by convention
//!
//! 1. **No record without a `legal_basis`** ([[Data Sources Registry]] §5). `Source::legal_basis`
//!    is not an `Option`, so serde rejects a line that omits it — the loader refuses to start
//!    rather than crawl on no authority or drop the record silently.
//! 2. **`approved` is false and `lifecycle` is `Proposed` until a human advances them.**
//!    [`Source::is_crawlable`] is the single predicate the crawler asks; an un-reviewed submission
//!    cannot be crawled by forgetting a check at some call site.

use crate::model::{Lifecycle, Source, SourceHealth};

/// Why loading or saving a registry file failed.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("cannot read registry {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("registry line {line}: {message}")]
    Parse { line: usize, message: String },
    #[error("cannot write registry {path}: {source}")]
    Write {
        path: String,
        source: std::io::Error,
    },
}

/// The registry: a set of sources, loaded from and exported to a JSON-Lines file.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    sources: Vec<Source>,
}

impl Registry {
    /// Load from a JSON-Lines file. Blank lines and `#` comments are skipped, so the file can carry
    /// section headers for the humans who edit it.
    ///
    /// A record that fails to parse — including one missing its `legal_basis` — is a hard error,
    /// not a skipped line: a malformed source record is one we would either crawl on no authority
    /// or drop silently, and both are worse than refusing to start.
    pub fn load(path: &str) -> Result<Self, RegistryError> {
        let text = std::fs::read_to_string(path).map_err(|source| RegistryError::Read {
            path: path.to_string(),
            source,
        })?;
        Self::parse(&text)
    }

    /// Parse JSON-Lines from a string.
    pub fn parse(text: &str) -> Result<Self, RegistryError> {
        let mut sources = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let source: Source = serde_json::from_str(line).map_err(|e| RegistryError::Parse {
                line: i + 1,
                message: e.to_string(),
            })?;
            sources.push(source);
        }
        Ok(Registry { sources })
    }

    /// Export to a JSON-Lines file, sorted by id so the git diff is stable — a source added in the
    /// middle is a one-line insertion, not a whole-file reshuffle. This is the "export on change":
    /// the caller mutates the registry, then writes it back, and the file is what git versions.
    pub fn save(&self, path: &str) -> Result<(), RegistryError> {
        std::fs::write(path, self.to_jsonl()?).map_err(|source| RegistryError::Write {
            path: path.to_string(),
            source,
        })
    }

    /// Serialise to JSON-Lines text, sorted by id. Separated from [`Registry::save`] so it can be
    /// used for an in-memory git export or a diff without touching the filesystem.
    pub fn to_jsonl(&self) -> Result<String, RegistryError> {
        let mut sorted: Vec<&Source> = self.sources.iter().collect();
        sorted.sort_by(|a, b| a.id.cmp(&b.id));
        let mut out = String::new();
        for s in sorted {
            let line = serde_json::to_string(s).map_err(|e| RegistryError::Write {
                path: "<memory>".into(),
                source: std::io::Error::other(e),
            })?;
            out.push_str(&line);
            out.push('\n');
        }
        Ok(out)
    }

    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// The sources the crawler should act on: approved, active-or-degraded, policy enabled.
    pub fn crawlable(&self) -> impl Iterator<Item = &Source> {
        self.sources.iter().filter(|s| s.is_crawlable())
    }

    pub fn get(&self, id: &str) -> Option<&Source> {
        self.sources.iter().find(|s| s.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Source> {
        self.sources.iter_mut().find(|s| s.id == id)
    }

    /// Add or replace a source by id. Returns the previous record if one was replaced.
    pub fn upsert(&mut self, source: Source) -> Option<Source> {
        if let Some(existing) = self.sources.iter_mut().find(|s| s.id == source.id) {
            Some(std::mem::replace(existing, source))
        } else {
            self.sources.push(source);
            None
        }
    }

    /// Auto-disable every source whose id is in `lapsed` (§5). Returns how many transitioned, so the
    /// caller exports only when something actually changed.
    pub fn disable_lapsed(&mut self, lapsed: &[&str]) -> usize {
        let mut changed = 0;
        for s in &mut self.sources {
            if lapsed.contains(&s.id.as_str()) && s.disable_for_lapsed_basis() {
                changed += 1;
            }
        }
        changed
    }

    /// Apply the lifecycle automation (§6) to every source for which the crawler reported health
    /// this cycle, plus the archival sweep to any long-disabled source. `health` maps source id →
    /// its recent metrics; a source absent from the map is still checked for archival. Returns the
    /// list of `(id, new_state)` transitions, so the caller exports once and can alert on each.
    pub fn apply_health(
        &mut self,
        health: &std::collections::HashMap<String, SourceHealth>,
        now: i64,
    ) -> Vec<(String, Lifecycle)> {
        let mut transitions = Vec::new();
        for s in &mut self.sources {
            // A source we have no metrics for this cycle is not evidence of health — skip its
            // degrade/recover check so a missing report can't silently recover a degraded source.
            // Archival is time-based, not health-based, so it still runs (default health is inert
            // for a disabled source).
            let observed = match health.get(&s.id) {
                Some(h) => *h,
                None if s.lifecycle == Lifecycle::Disabled => SourceHealth::default(),
                None => continue,
            };
            if let Some(state) = s.apply_health(observed, now) {
                transitions.push((s.id.clone(), state));
            }
        }
        transitions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Lifecycle, SourceType};
    use crate::{LegalBasis, TrustTier};

    fn sample(id: &str, approved: bool, lifecycle: Lifecycle) -> Source {
        Source {
            id: id.into(),
            kind: SourceType::Web,
            display_name: id.into(),
            entry_points: vec![format!("https://{id}.dz")],
            languages: vec![],
            crawl_policy: Default::default(),
            trust_tier: TrustTier::A,
            legal_basis: LegalBasis::PublicWebRobotsOk,
            approved,
            lifecycle,
            notes: None,
            last_run_at: 0,
            last_status: None,
            disabled_at: None,
        }
    }

    #[test]
    fn a_record_without_a_legal_basis_is_rejected() {
        // Every required field present except legal_basis — the loader must refuse it, because a
        // source with no basis is one we would crawl on no authority.
        let line = r#"{"id":"x","kind":"web","display_name":"X","trust_tier":"A"}"#;
        let err = Registry::parse(line).unwrap_err();
        assert!(matches!(err, RegistryError::Parse { line: 1, .. }));
    }

    #[test]
    fn only_approved_active_sources_are_crawlable() {
        assert!(sample("a", true, Lifecycle::Active).is_crawlable());
        assert!(
            sample("a", true, Lifecycle::Degraded).is_crawlable(),
            "degraded is still crawled"
        );
        assert!(
            !sample("a", false, Lifecycle::Active).is_crawlable(),
            "unapproved is never crawled"
        );
        assert!(!sample("a", true, Lifecycle::Proposed).is_crawlable());
        assert!(!sample("a", true, Lifecycle::Disabled).is_crawlable());
        assert!(!sample("a", true, Lifecycle::Archived).is_crawlable());
    }

    #[test]
    fn a_disabled_crawl_policy_stops_a_crawl_even_when_active() {
        let mut s = sample("a", true, Lifecycle::Active);
        s.crawl_policy.enabled = false;
        assert!(!s.is_crawlable(), "a paused policy is an off switch too");
    }

    #[test]
    fn a_lapsed_basis_disables_the_source() {
        let mut r = Registry::default();
        r.upsert(sample("aps", true, Lifecycle::Active));
        r.upsert(sample("elk", true, Lifecycle::Active));
        assert_eq!(r.crawlable().count(), 2);

        let changed = r.disable_lapsed(&["aps"]);
        assert_eq!(changed, 1);
        assert_eq!(r.get("aps").unwrap().lifecycle, Lifecycle::Disabled);
        assert!(r
            .get("aps")
            .unwrap()
            .notes
            .as_deref()
            .unwrap()
            .contains("auto-disabled"));
        assert_eq!(
            r.crawlable().count(),
            1,
            "the disabled source is no longer crawled"
        );
        // Idempotent: disabling again changes nothing.
        assert_eq!(r.disable_lapsed(&["aps"]), 0);
    }

    #[test]
    fn round_trips_through_json_lines_sorted_by_id() {
        let mut r = Registry::default();
        r.upsert(sample("b", true, Lifecycle::Active));
        r.upsert(sample("a", false, Lifecycle::Proposed));

        let jsonl = r.to_jsonl().unwrap();
        // Sorted: "a" before "b", so the export is a stable diff.
        let ids: Vec<&str> = jsonl
            .lines()
            .map(|l| l.split('"').nth(3).unwrap())
            .collect();
        assert_eq!(ids, vec!["a", "b"]);

        let reloaded = Registry::parse(&jsonl).unwrap();
        assert_eq!(reloaded.len(), 2);
        assert_eq!(reloaded.get("a").unwrap().lifecycle, Lifecycle::Proposed);
        assert_eq!(
            reloaded.get("b").unwrap().legal_basis,
            LegalBasis::PublicWebRobotsOk
        );
    }

    #[test]
    fn a_record_without_a_lifecycle_field_defaults_to_proposed() {
        // Backward compatibility: a line written before lifecycle existed loads as the safe default
        // — not crawlable until explicitly activated.
        let line = r#"{"id":"x","kind":"web","display_name":"X","trust_tier":"A","legal_basis":"public_web_robots_ok","approved":true}"#;
        let r = Registry::parse(line).unwrap();
        assert_eq!(r.get("x").unwrap().lifecycle, Lifecycle::Proposed);
        assert!(!r.get("x").unwrap().is_crawlable());
    }

    #[test]
    fn comments_and_blanks_are_skipped() {
        let text = format!(
            "# national news\n\n{}\n# regional\n{}\n",
            serde_json::to_string(&sample("a", true, Lifecycle::Active)).unwrap(),
            serde_json::to_string(&sample("b", true, Lifecycle::Active)).unwrap(),
        );
        assert_eq!(Registry::parse(&text).unwrap().len(), 2);
    }

    #[test]
    fn apply_health_degrades_reported_sources_but_not_absent_ones() {
        use crate::model::SourceHealth;
        use std::collections::HashMap;

        let mut r = Registry::default();
        r.upsert(sample("failing", true, Lifecycle::Active));
        r.upsert(sample("degraded_no_data", true, Lifecycle::Degraded));

        let mut health = HashMap::new();
        health.insert(
            "failing".to_string(),
            SourceHealth {
                error_rate_24h: 0.9,
                consecutive_zero_runs: 0,
            },
        );
        // "degraded_no_data" is absent from the report this cycle.
        let t = r.apply_health(&health, 1000);

        assert_eq!(t, vec![("failing".to_string(), Lifecycle::Degraded)]);
        assert_eq!(
            r.get("degraded_no_data").unwrap().lifecycle,
            Lifecycle::Degraded,
            "a missing report must not silently recover a degraded source"
        );
    }

    #[test]
    fn upsert_replaces_by_id() {
        let mut r = Registry::default();
        r.upsert(sample("a", false, Lifecycle::Proposed));
        let prev = r.upsert(sample("a", true, Lifecycle::Active));
        assert!(prev.is_some());
        assert_eq!(r.len(), 1, "same id replaces, not appends");
        assert!(r.get("a").unwrap().is_crawlable());
    }
}
