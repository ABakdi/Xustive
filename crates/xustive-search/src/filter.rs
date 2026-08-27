//! Typed filter construction.
//!
//! User input never reaches a filter expression as a string. The builder takes enum variants and
//! escapes the few free-text values (`site:`), so a query cannot inject filter syntax.

use xustive_core::{SentimentLabel, SourceType};

/// Spam suppression threshold: documents at or above it stay indexed but out of default results.
/// Lives here — beside the clause that applies it — and is shared by the API handler and the eval
/// harness, so the two cannot score under different spam regimes (BUG-003).
pub const SPAM_THRESHOLD: f32 = 0.8;

/// Filters extracted from a request, before they become a Meilisearch expression.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filters {
    pub source_types: Vec<SourceType>,
    pub sentiments: Vec<SentimentLabel>,
    pub languages: Vec<String>,
    /// Unix seconds, inclusive.
    pub published_from: Option<i64>,
    /// Unix seconds, inclusive.
    pub published_to: Option<i64>,
    /// From a `site:` operator. Free text, therefore escaped.
    pub domain: Option<String>,
    /// Fetched MIME, set by the Files vertical to select documents (`application/pdf`).
    pub content_type: Option<String>,
    /// `image` or `video`: the Images and Videos verticals (M9). A saved filter over `media.type`,
    /// which Meilisearch flattens out of the array of objects, so no reindex was needed.
    pub media_kind: Option<String>,
    /// File type (`png`, `jpg`…) and kind of picture (`photo`, `screenshot`…) of the images the
    /// Images tab shows — the reverse-image chips, offered on the ordinary direction too (M10).
    pub media_ext: Option<String>,
    pub media_style: Option<String>,
    /// Hide documents suppressed as spam. On by default.
    pub exclude_spam: bool,
    /// Hide documents whose publish date we guessed. Set when a date filter is active, because
    /// including guessed dates in a date filter would make the filter a lie.
    pub exclude_unknown_dates: bool,
}

impl Filters {
    pub fn is_empty(&self) -> bool {
        self.source_types.is_empty()
            && self.sentiments.is_empty()
            && self.languages.is_empty()
            && self.published_from.is_none()
            && self.published_to.is_none()
            && self.domain.is_none()
            && self.content_type.is_none()
            && self.media_kind.is_none()
            && self.media_ext.is_none()
            && self.media_style.is_none()
    }

    /// Render as a Meilisearch filter expression.
    ///
    /// Semantics: OR within a facet, AND across facets. That is what users expect from
    /// checkbox filters and needs no explanation in the UI.
    pub fn to_expression(&self, spam_threshold: f32) -> Option<String> {
        let mut clauses: Vec<String> = Vec::new();

        if !self.source_types.is_empty() {
            clauses.push(or_clause(
                "source_type",
                self.source_types.iter().map(|s| s.as_str()),
            ));
        }
        if !self.sentiments.is_empty() {
            clauses.push(or_clause(
                "sentiment.label",
                self.sentiments.iter().map(|s| s.as_str()),
            ));
        }
        if !self.languages.is_empty() {
            clauses.push(or_clause(
                "language",
                self.languages.iter().map(|s| s.as_str()),
            ));
        }
        if let Some(from) = self.published_from {
            clauses.push(format!("published_at >= {from}"));
        }
        if let Some(to) = self.published_to {
            clauses.push(format!("published_at <= {to}"));
        }
        if let Some(d) = &self.domain {
            clauses.push(format!("domain = {}", quote(d)));
        }
        if let Some(ct) = &self.content_type {
            clauses.push(format!("content_type = {}", quote(ct)));
        }
        if let Some(kind) = &self.media_kind {
            clauses.push(format!("media.type = {}", quote(kind)));
        }
        if let Some(ext) = &self.media_ext {
            clauses.push(format!("media.ext = {}", quote(ext)));
        }
        if let Some(style) = &self.media_style {
            clauses.push(format!("media.style = {}", quote(style)));
        }
        if self.exclude_spam {
            clauses.push(format!("spam_score < {spam_threshold}"));
        }
        if self.exclude_unknown_dates {
            clauses.push("published_at_precision != \"unknown\"".to_string());
        }

        if clauses.is_empty() {
            None
        } else {
            Some(clauses.join(" AND "))
        }
    }

    /// A date filter implies hiding documents whose date we do not actually know.
    pub fn normalise(mut self) -> Self {
        if self.published_from.is_some() || self.published_to.is_some() {
            self.exclude_unknown_dates = true;
        }
        // A reversed range is a user slip, not an error worth showing.
        if let (Some(f), Some(t)) = (self.published_from, self.published_to) {
            if f > t {
                self.published_from = Some(t);
                self.published_to = Some(f);
            }
        }
        self
    }
}

fn or_clause<'a>(field: &str, values: impl Iterator<Item = &'a str>) -> String {
    let parts: Vec<String> = values.map(|v| format!("{field} = {}", quote(v))).collect();
    if parts.len() == 1 {
        parts.into_iter().next().unwrap()
    } else {
        format!("({})", parts.join(" OR "))
    }
}

/// Quote and escape a value for a Meilisearch filter expression.
///
/// This is the injection boundary. Backslashes first, then quotes — the other order
/// double-escapes.
fn quote(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPAM: f32 = 0.8;

    #[test]
    fn empty_filters_produce_no_expression() {
        let f = Filters::default();
        assert!(f.is_empty());
        assert_eq!(f.to_expression(SPAM), None);
    }

    #[test]
    fn single_source_type() {
        let f = Filters {
            source_types: vec![SourceType::Web],
            ..Default::default()
        };
        assert_eq!(f.to_expression(SPAM).unwrap(), r#"source_type = "web""#);
    }

    #[test]
    fn multiple_values_within_a_facet_are_ored() {
        let f = Filters {
            source_types: vec![SourceType::Web, SourceType::Facebook],
            ..Default::default()
        };
        assert_eq!(
            f.to_expression(SPAM).unwrap(),
            r#"(source_type = "web" OR source_type = "facebook")"#
        );
    }

    #[test]
    fn different_facets_are_anded() {
        let f = Filters {
            source_types: vec![SourceType::Facebook],
            sentiments: vec![SentimentLabel::Negative],
            ..Default::default()
        };
        let e = f.to_expression(SPAM).unwrap();
        assert!(e.contains("source_type = \"facebook\""));
        assert!(e.contains("sentiment.label = \"negative\""));
        assert!(e.contains(" AND "));
    }

    #[test]
    fn date_range_renders_as_comparisons() {
        let f = Filters {
            published_from: Some(1_754_352_000),
            published_to: Some(1_754_438_400),
            ..Default::default()
        };
        let e = f.to_expression(SPAM).unwrap();
        assert!(e.contains("published_at >= 1754352000"));
        assert!(e.contains("published_at <= 1754438400"));
    }

    #[test]
    fn date_filter_hides_guessed_dates() {
        // Otherwise "posts from last week" silently includes documents whose date we invented.
        let f = Filters {
            published_from: Some(1),
            ..Default::default()
        }
        .normalise();
        assert!(f.exclude_unknown_dates);
        assert!(f
            .to_expression(SPAM)
            .unwrap()
            .contains("published_at_precision != \"unknown\""));
    }

    #[test]
    fn no_date_filter_keeps_unknown_dates() {
        let f = Filters::default().normalise();
        assert!(!f.exclude_unknown_dates);
    }

    #[test]
    fn reversed_date_range_is_swapped_not_rejected() {
        let f = Filters {
            published_from: Some(200),
            published_to: Some(100),
            ..Default::default()
        }
        .normalise();
        assert_eq!(f.published_from, Some(100));
        assert_eq!(f.published_to, Some(200));
    }

    #[test]
    fn spam_suppression_is_opt_out() {
        let f = Filters {
            exclude_spam: true,
            ..Default::default()
        };
        assert_eq!(f.to_expression(SPAM).unwrap(), "spam_score < 0.8");
    }

    #[test]
    fn domain_from_site_operator_is_quoted() {
        let f = Filters {
            domain: Some("elkhabar.com".into()),
            ..Default::default()
        };
        assert_eq!(f.to_expression(SPAM).unwrap(), r#"domain = "elkhabar.com""#);
    }

    #[test]
    fn filter_injection_is_escaped() {
        // The attack: close the string and append a clause that widens the result set.
        let hostile = r#"x" OR spam_score > 0 OR domain = ""#;
        let f = Filters {
            domain: Some(hostile.into()),
            ..Default::default()
        };
        let e = f.to_expression(SPAM).unwrap();
        // Every injected quote is escaped, so the whole thing stays one string literal.
        assert!(e.starts_with(r#"domain = "x\" OR"#), "got {e}");
        assert_eq!(e.matches("\\\"").count(), 2);
    }

    #[test]
    fn backslashes_are_escaped_before_quotes() {
        // Wrong order would turn `\"` into `\\"` and break out of the literal.
        let f = Filters {
            domain: Some(r#"a\"b"#.into()),
            ..Default::default()
        };
        let e = f.to_expression(SPAM).unwrap();
        assert_eq!(e, r#"domain = "a\\\"b""#);
    }

    #[test]
    fn is_empty_ignores_internal_flags() {
        // exclude_spam is a system default, not a user filter — the UI should not show it
        // as an active filter chip.
        let f = Filters {
            exclude_spam: true,
            ..Default::default()
        };
        assert!(f.is_empty());
    }
}
