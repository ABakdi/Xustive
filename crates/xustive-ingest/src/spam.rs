//! Spam scoring (M2-T06.4).
//!
//! A score in `0.0..=1.0`, high meaning spam. It does not delete anything — search suppresses
//! documents at or above `0.8` (they stay in the index and out of default results), so the score
//! only has to be trustworthy at the top end. A false positive suppresses a real document, so the
//! signals here are deliberately conservative: it takes clear spam to clear the bar.
//!
//! Two signals, combined by taking the stronger:
//!
//! 1. **Spam phrases.** A compiled list of the phrases this engine actually meets on the Algerian
//!    web — betting, loan and money scams, fake pharma, crypto, adult, SEO filler — in Arabic,
//!    French and English. Matched folded and whole-token, so a phrase cannot hit inside a longer
//!    word, and scored by how many distinct spam phrases appear, not how often — one phrase
//!    repeated is one signal, not ten.
//!
//! 2. **Keyword stuffing.** The classic on-page spam: one term repeated far past natural frequency
//!    to rank for it. Measured as the share of the body taken by its single most common content
//!    word. Ordinary prose sits well below the threshold; a stuffed page spikes.
//!
//! A model would do better on novel spam, but a list plus a structural check is explainable — you
//! can read why a document was suppressed — and it never suppresses on a signal nobody can see.

use std::collections::HashMap;

use xustive_text::{fold, tokens};

const PHRASES_RAW: &str = include_str!("../../../data/spam/phrases.txt");

/// Distinct spam phrases at which the phrase signal saturates to 1.0.
///
/// Three: a page naming three different scam phrases is unambiguous, and requiring more would let
/// obvious spam through. One phrase alone is a weak signal — legitimate text quotes spam to warn
/// against it — so a single hit scores well below the suppression bar.
const PHRASES_FOR_MAX: f32 = 3.0;

/// Keyword-stuffing share at which the stuffing signal saturates.
///
/// A body where one content word is 18% of all content tokens is stuffed — natural prose, even
/// about one subject, rarely passes ~8%. Short function-like tokens are excluded from the count so
/// "the" or "و" cannot trip it.
const STUFF_SATURATION: f32 = 0.18;

/// A spam phrase as a folded token sequence.
fn phrases() -> Vec<Vec<String>> {
    PHRASES_RAW
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|line| tokens(&fold(line)).map(str::to_string).collect())
        .filter(|seq: &Vec<String>| !seq.is_empty())
        .collect()
}

/// Score a document's spam-likelihood from its title and body.
pub fn spam_score(title: &str, body: &str) -> f32 {
    let folded = fold(&format!("{title} {body}"));
    let toks: Vec<&str> = tokens(&folded).collect();
    if toks.is_empty() {
        return 0.0;
    }

    let phrase = phrase_signal(&toks);
    let stuffing = stuffing_signal(&toks);

    // The stronger of the two. Spam betrays itself by either — a scam-phrase page and a
    // keyword-stuffed page are both spam, and a document need not be both to be suppressed.
    phrase.max(stuffing).clamp(0.0, 1.0)
}

/// Fraction of `PHRASES_FOR_MAX` distinct spam phrases present.
fn phrase_signal(toks: &[&str]) -> f32 {
    let present: std::collections::HashSet<&str> = toks.iter().copied().collect();
    let mut distinct = 0usize;
    for seq in phrases() {
        let hit = match seq.as_slice() {
            [] => false,
            [one] => present.contains(one.as_str()),
            multi => toks
                .windows(multi.len())
                .any(|w| w.iter().zip(multi).all(|(t, m)| *t == m)),
        };
        if hit {
            distinct += 1;
        }
    }
    (distinct as f32 / PHRASES_FOR_MAX).min(1.0)
}

/// How stuffed the body is: the most common content token's share, scaled to the saturation point.
fn stuffing_signal(toks: &[&str]) -> f32 {
    // Content tokens only — short ones are articles, conjunctions and particles, which are
    // naturally frequent and must not be read as stuffing.
    let content: Vec<&&str> = toks.iter().filter(|t| t.chars().count() >= 4).collect();
    if content.len() < 25 {
        // Too little text to distinguish stuffing from ordinary repetition.
        return 0.0;
    }
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for t in &content {
        *counts.entry(**t).or_insert(0) += 1;
    }
    let top = counts.values().copied().max().unwrap_or(0);
    let share = top as f32 / content.len() as f32;
    (share / STUFF_SATURATION).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_news_scores_low() {
        let body = "أعلنت وزارة الطاقة والمناجم عن انطلاق أشغال المشروع الجديد في ولاية بشار. \
                    ويهدف المشروع إلى رفع القدرة الإنتاجية وتوفير مناصب شغل جديدة في المنطقة. \
                    وأوضح الوزير أن الأشغال ستنطلق خلال الأسابيع المقبلة وفق الرزنامة المحددة.";
        assert!(
            spam_score("خبر", body) < 0.5,
            "real news must not be suppressed"
        );
    }

    #[test]
    fn a_page_full_of_scam_phrases_scores_high() {
        let body = "gagnez de l argent facilement avec nos paris sportifs et notre casino en ligne. \
                    cliquez ici maintenant pour un bonus de bienvenue et un credit sans justificatif.";
        assert!(
            spam_score("offre", body) >= 0.8,
            "three distinct scam phrases should clear the suppression bar, got {}",
            spam_score("offre", body)
        );
    }

    #[test]
    fn one_spam_phrase_alone_is_below_the_bar() {
        // A news article may mention a betting brand while reporting on it. One phrase is weak.
        let body = "the regulator has fined the betting company after complaints about paris \
                    sportifs advertising aimed at minors according to a statement issued today by \
                    the ministry which said further measures are under consideration this year";
        assert!(
            spam_score("regulation", body) < 0.8,
            "a single spam phrase in real reporting must not be suppressed"
        );
    }

    #[test]
    fn keyword_stuffing_is_caught_without_any_phrase() {
        // One word repeated far past natural frequency — no listed phrase, still spam.
        let stuffed = "assurance ".repeat(40) + "voiture pas cher devis rapide en ligne maintenant";
        assert!(
            spam_score("assurance", &stuffed) >= 0.8,
            "keyword stuffing should score high, got {}",
            spam_score("assurance", &stuffed)
        );
    }

    #[test]
    fn empty_text_is_not_spam() {
        assert_eq!(spam_score("", ""), 0.0);
    }

    #[test]
    fn a_repeated_phrase_counts_once() {
        // Ten repeats of one phrase is one signal, not ten — otherwise a stutter looks like spam.
        let once = spam_score(
            "x",
            "casino en ligne. le reste du texte est parfaitement normal ici bon",
        );
        let many = spam_score(
            "x",
            "casino en ligne casino en ligne casino en ligne le reste du texte normal ici bon",
        );
        // Both are one distinct phrase; the many-repeats version must not score higher on phrases.
        // (It may differ on stuffing, so only assert it does not exceed the bar on phrase alone.)
        assert!(once < 0.8 && many < 0.9);
    }
}
