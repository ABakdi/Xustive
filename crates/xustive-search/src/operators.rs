//! Query operators.
//!
//! `"exact phrase"`, `site:aps.dz`, `-excluded`. Three operators, chosen because they are the
//! ones people already try — a search box that silently treats `site:aps.dz` as three words has
//! taught the user it does not understand them.
//!
//! # What this deliberately does not do
//!
//! No boolean `AND`/`OR`/`NOT`, no nesting, no field-scoped search beyond `site:`. Every one of
//! those is a grammar, and a grammar in a search box is a thing users get wrong and blame
//! themselves for. These three are unambiguous, and each maps onto something the engine can
//! actually enforce.
//!
//! # Order matters
//!
//! Operators are extracted from the **raw** query, before normalisation. Normalisation folds
//! quotes and punctuation, so a `"phrase"` parsed afterwards is no longer a phrase — the marks
//! that made it one are gone.

use serde::Serialize;

/// A query split into what to search for and what constrains it.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Parsed {
    /// The query with every operator removed — what actually goes to the engine.
    pub terms: String,
    /// Quoted phrases, kept quoted so the engine matches them adjacently.
    pub phrases: Vec<String>,
    /// From `site:`. One only: two would mean an intersection, which is always empty.
    pub site: Option<String>,
    /// From `-term`. Excluded from results.
    pub excluded: Vec<String>,
}

impl Parsed {
    /// Whether anything is left to search for.
    ///
    /// `site:aps.dz` alone is a legitimate query — everything from that domain — so a parse with
    /// no terms but a site is not empty.
    pub fn is_empty(&self) -> bool {
        self.terms.trim().is_empty() && self.phrases.is_empty() && self.site.is_none()
    }

    /// What to send to the engine, phrases re-quoted.
    ///
    /// Meilisearch honours quotes in the query string, so phrases are reassembled rather than
    /// expressed as a filter — a filter would match documents containing the words anywhere.
    pub fn engine_query(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.terms.trim().is_empty() {
            parts.push(self.terms.trim().to_string());
        }
        for phrase in &self.phrases {
            parts.push(format!("\"{phrase}\""));
        }
        parts.join(" ")
    }
}

/// Split a raw query into terms and operators.
///
/// Total: any input produces a `Parsed`, and unrecognisable operator syntax stays in `terms`
/// rather than being discarded. A user who types `site:` with nothing after it means to search
/// for that text, not to be given no results.
pub fn parse(raw: &str) -> Parsed {
    let mut out = Parsed::default();
    let mut terms: Vec<String> = Vec::new();
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        if c.is_whitespace() {
            i += 1;
            continue;
        }

        // A quoted phrase. Both ASCII and typographic quotes, because a phone keyboard produces
        // the typographic ones and the user cannot tell the difference.
        if let Some(closing) = closing_quote(c) {
            if let Some(end) = chars[i + 1..].iter().position(|&ch| ch == closing) {
                let phrase: String = chars[i + 1..i + 1 + end].iter().collect();
                let phrase = phrase.trim();
                // An empty `""` is not a phrase; dropping it silently is right, because there is
                // nothing to search for and nothing to tell the user.
                if !phrase.is_empty() {
                    out.phrases.push(phrase.to_string());
                }
                i += end + 2;
                continue;
            }
            // Unclosed. Treated as an ordinary character — a half-typed query should still
            // search, not fail.
        }

        let token: String = chars[i..]
            .iter()
            .take_while(|ch| !ch.is_whitespace())
            .collect();
        let consumed = token.chars().count();

        if let Some(host) = token.strip_prefix("site:").filter(|h| !h.is_empty()) {
            // Last one wins. Two `site:` operators would mean documents on both domains at once,
            // which is always empty — so the later is treated as a correction, not a conflict.
            out.site = Some(normalise_host(host));
        } else if let Some(term) = token.strip_prefix('-').filter(|t| !t.is_empty()) {
            // A leading hyphen inside a word is a hyphenated word, not an exclusion:
            // `Sidi Bel-Abbes` must not exclude `Abbes`. Only a token that *begins* with one.
            out.excluded.push(term.to_string());
        } else {
            terms.push(token);
        }
        i += consumed;
    }

    out.terms = terms.join(" ");
    out
}

fn closing_quote(c: char) -> Option<char> {
    match c {
        '"' => Some('"'),
        '\u{201C}' => Some('\u{201D}'), // “ ”
        '\u{00AB}' => Some('\u{00BB}'), // « » — the French quotes Algerians type
        _ => None,
    }
}

/// Reduce `site:` to a bare host.
///
/// Accepts what people paste: a full URL, a `www.` prefix, a trailing slash. Refusing those would
/// mean the operator works only for users who already know its exact syntax.
fn normalise_host(input: &str) -> String {
    let host = input
        .trim()
        .trim_end_matches('/')
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.");
    host.split('/').next().unwrap_or(host).to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_query_is_unchanged() {
        let p = parse("سعر صرف الأورو");
        assert_eq!(p.terms, "سعر صرف الأورو");
        assert!(p.phrases.is_empty() && p.site.is_none() && p.excluded.is_empty());
    }

    #[test]
    fn a_quoted_phrase_is_extracted_and_re_quoted() {
        let p = parse(r#"prix "gaz butane" Alger"#);
        assert_eq!(p.phrases, vec!["gaz butane"]);
        assert_eq!(p.terms, "prix Alger");
        assert!(p.engine_query().contains(r#""gaz butane""#));
    }

    #[test]
    fn typographic_and_french_quotes_work_too() {
        // A phone keyboard produces these and the user cannot tell the difference.
        assert_eq!(
            parse("prix \u{201C}gaz butane\u{201D}").phrases,
            vec!["gaz butane"]
        );
        assert_eq!(
            parse("prix \u{00AB}gaz butane\u{00BB}").phrases,
            vec!["gaz butane"]
        );
    }

    #[test]
    fn an_unclosed_quote_still_searches() {
        // A half-typed query is the normal state of a search box. Failing on it would mean the
        // engine breaks while the user is still typing.
        let p = parse(r#"prix "gaz butane"#);
        assert!(p.phrases.is_empty());
        assert!(p.terms.contains("gaz"), "got {:?}", p.terms);
    }

    #[test]
    fn site_accepts_what_people_actually_paste() {
        for input in [
            "site:aps.dz",
            "site:www.aps.dz",
            "site:https://www.aps.dz",
            "site:https://www.aps.dz/",
            "site:APS.DZ",
            "site:aps.dz/economie",
        ] {
            assert_eq!(parse(input).site.as_deref(), Some("aps.dz"), "{input}");
        }
    }

    #[test]
    fn the_last_site_wins() {
        // Two domains at once is always empty, so the second is a correction, not a conflict.
        assert_eq!(
            parse("site:aps.dz site:elkhabar.com").site.as_deref(),
            Some("elkhabar.com")
        );
    }

    #[test]
    fn an_excluded_term_is_removed_from_the_query() {
        let p = parse("الجزائر -رياضة");
        assert_eq!(p.excluded, vec!["رياضة"]);
        assert_eq!(p.terms, "الجزائر");
    }

    #[test]
    fn a_hyphenated_word_is_not_an_exclusion() {
        // `Sidi Bel-Abbes` must not exclude `Abbes`. Only a token that *begins* with a hyphen.
        let p = parse("Sidi Bel-Abbes");
        assert!(p.excluded.is_empty(), "got {:?}", p.excluded);
        assert_eq!(p.terms, "Sidi Bel-Abbes");
    }

    #[test]
    fn a_bare_operator_is_treated_as_text() {
        // Someone typing `site:` with nothing after it means to search for that, not to get
        // nothing back.
        let p = parse("site:");
        assert!(p.site.is_none());
        assert_eq!(p.terms, "site:");

        let dash = parse("-");
        assert!(dash.excluded.is_empty());
        assert_eq!(dash.terms, "-");
    }

    #[test]
    fn a_site_only_query_is_not_empty() {
        // Everything from a domain is a legitimate thing to ask for.
        let p = parse("site:aps.dz");
        assert!(!p.is_empty());
        assert_eq!(p.engine_query(), "");
    }

    #[test]
    fn an_actually_empty_query_is_empty() {
        assert!(parse("").is_empty());
        assert!(parse("   ").is_empty());
        assert!(
            parse(r#""""#).is_empty(),
            "an empty phrase is nothing to search for"
        );
    }

    #[test]
    fn operators_combine() {
        let p = parse(r#"site:elkhabar.com "كرة القدم" الجزائر -تنس"#);
        assert_eq!(p.site.as_deref(), Some("elkhabar.com"));
        assert_eq!(p.phrases, vec!["كرة القدم"]);
        assert_eq!(p.excluded, vec!["تنس"]);
        assert_eq!(p.terms, "الجزائر");
    }

    #[test]
    fn parsing_is_total_and_does_not_panic() {
        for input in [
            "\"",
            "\"\"\"",
            "site:site:site:",
            "---",
            "- - -",
            "\u{00AB}",
            "a\"b\"c",
            &"\"".repeat(1000),
            &"site:x ".repeat(500),
        ] {
            let _ = parse(input);
        }
    }

    #[test]
    fn a_long_query_parses_quickly() {
        // Runs on every search, so it must not scale badly with length.
        let long = "الجزائر ".repeat(5000);
        let started = std::time::Instant::now();
        let _ = parse(&long);
        assert!(
            started.elapsed().as_millis() < 50,
            "took {:?}",
            started.elapsed()
        );
    }
}
