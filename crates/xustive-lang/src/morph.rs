//! Light Arabic morphology for query expansion (M7-T01.1).
//!
//! Meilisearch does not stem Arabic, so `الكتاب` (the book) and `كتاب` (book) are different tokens
//! and a query for one misses documents that only contain the other — the single biggest source of
//! Arabic word-mismatch. Rather than stem the whole index (a reindex and a correctness risk), we
//! generate morphological **variants** of each query token — the definite-article and common
//! prefix/suffix forms — and let the [`crate::expand`] expander OR them into the query. Recall
//! becomes symmetric at query time: a search for either form reaches documents holding either, with
//! no schema change.
//!
//! Deliberately conservative. Only Arabic-script tokens; only the well-known affixes below; and only
//! when the stripped stem is still a plausible word (≥ [`MIN_STEM`] letters), so it widens recall
//! without flooding the query with garbage. This is *light* morphology — surface affix stripping, not
//! root extraction — which is the right trade for a query-expansion aid: cheap, safe, and reversible.

/// Definite-article family prefixes (article, and article fused with a conjunction/preposition),
/// longest first so the longest match wins.
const PREFIXES: &[&str] = &["وال", "فال", "بال", "كال", "لل", "ال"];
/// Common inflectional suffixes (plurals, feminine, attached pronouns), longest first.
const SUFFIXES: &[&str] = &["ات", "ون", "ين", "ان", "ها", "هم", "ية", "ة"];
/// A stem shorter than this is more likely noise than a word, so stripping stops there.
pub const MIN_STEM: usize = 3;

/// True when the token is Arabic script (allowing the tatweel elongation character).
fn is_arabic(tok: &str) -> bool {
    let mut any = false;
    for c in tok.chars() {
        if ('\u{0600}'..='\u{06FF}').contains(&c) {
            any = true;
        } else if c != '\u{0640}' {
            return false;
        }
    }
    any
}

fn push_variant(out: &mut Vec<String>, original: &str, candidate: String) {
    if candidate != original && candidate.chars().count() >= MIN_STEM && !out.contains(&candidate) {
        out.push(candidate);
    }
}

/// Morphological variants of an Arabic token: the affix-stripped stem and the definite-article form.
/// Empty for non-Arabic tokens, and never includes the input itself.
pub fn arabic_variants(tok: &str) -> Vec<String> {
    if !is_arabic(tok) {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();

    // Strip one leading definite-article prefix → the bare stem. Longest prefix wins.
    let mut stem = tok;
    for p in PREFIXES {
        if let Some(rest) = tok.strip_prefix(p) {
            if rest.chars().count() >= MIN_STEM {
                stem = rest;
                push_variant(&mut out, tok, rest.to_string());
                break;
            }
        }
    }
    // A token that does not already begin with an article prefix gets the article form offered, so
    // `كتاب` also reaches `الكتاب`. (Checked on the raw token, not on whether a strip succeeded — a
    // bare `ال` starts with a prefix yet strips to nothing, and must not become `الال`.)
    if !PREFIXES.iter().any(|p| tok.starts_with(p)) {
        push_variant(&mut out, tok, format!("ال{tok}"));
    }
    // Strip one trailing suffix from the (possibly prefix-stripped) stem.
    for s in SUFFIXES {
        if let Some(rest) = stem.strip_suffix(s) {
            if rest.chars().count() >= MIN_STEM {
                push_variant(&mut out, tok, rest.to_string());
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_definite_article_is_reversible() {
        // الكتاب → offers كتاب; كتاب → offers الكتاب. So a query for either reaches both.
        assert!(arabic_variants("الكتاب").contains(&"كتاب".to_string()));
        assert!(arabic_variants("كتاب").contains(&"الكتاب".to_string()));
    }

    #[test]
    fn a_fused_prefix_is_stripped() {
        // وال (and-the) → the bare stem.
        assert!(arabic_variants("والمدينة").contains(&"مدينة".to_string()));
    }

    #[test]
    fn a_plural_suffix_is_stripped() {
        // معلمون (teachers) → معلم after stripping ون.
        assert!(arabic_variants("معلمون").contains(&"معلم".to_string()));
    }

    #[test]
    fn non_arabic_and_tiny_tokens_are_left_alone() {
        assert!(arabic_variants("constantine").is_empty());
        assert!(arabic_variants("book").is_empty());
        // Too short to strip to a plausible stem — no garbage produced.
        assert!(arabic_variants("ال").is_empty());
    }

    #[test]
    fn a_variant_never_repeats_the_input() {
        for v in arabic_variants("الجزائر") {
            assert_ne!(v, "الجزائر");
            assert!(v.chars().count() >= MIN_STEM);
        }
    }
}
