//! Arabizi ↔ Arabic transliteration.
//!
//! Algerians write Arabic in Latin letters constantly — `3andi`, `khouya`, `wach rak` — because
//! for two decades that was the only keyboard available, and the habit outlived the constraint.
//! The engine already understands both directions internally: `xustive-lang` transliterates in
//! order to *search*, so a query typed in Arabizi finds documents written in Arabic script.
//!
//! This surfaces that conversion as an answer rather than leaving it as plumbing. Someone who
//! needs to send a message in Arabic script but only has a Latin keyboard currently leaves for a
//! site that does exactly this and logs what they typed.
//!
//! # Why this is offered rather than applied
//!
//! The card converts on request. It does **not** rewrite the query. Arabizi is ambiguous — `3` is
//! ع but `khouya` could reasonably be خويا or خوية — and a search engine that silently replaced
//! what someone typed with one guess would be unfixable from the user's side. The conversion is
//! shown, the original is searched, and both are visible.

use xustive_lang::translit::{self, TranslitConfig};

use crate::{Answer, Tool};

pub struct Transliterator;

/// Explicit request words, in the four languages the engine serves.
const TRIGGERS: &[&str] = &[
    "transliterate",
    "translitterer",
    "translittérer",
    "arabizi",
    "franco", // "franco-arabe", what Algerians actually call it
    "arabic script",
    "en arabe",
    "بالعربية",
    "بالحروف العربية",
    "عرزية", // "arabizi" written in Arabic
];

impl Tool for Transliterator {
    fn name(&self) -> &'static str {
        "transliterate"
    }

    fn keyword(&self) -> &'static str {
        "translit"
    }

    fn answer(&self, query: &str) -> Option<Answer> {
        let q = query.trim();
        if q.is_empty() {
            return None;
        }

        // Only on an explicit request. Arabizi is *ordinary text* here — half the queries this
        // engine will ever see are written in it, and offering to transliterate every one of them
        // would put a card above the results the user actually wanted.
        let lower = q.to_lowercase();
        let trigger = TRIGGERS.iter().find(|t| lower.contains(**t))?;

        // Strip the trigger word and any connecting particle around it.
        let subject = strip(q, &lower, trigger);
        if subject.is_empty() {
            return None;
        }

        let Some(converted) = to_arabic_phrase(&subject) else {
            // A conversion that produced nothing is not an answer. It means the input was already
            // Arabic script, or contained nothing transliterable.
            return None;
        };

        Some(Answer {
            tool: "transliterate",
            // Below the calculator and the utilities. The mapping is genuinely ambiguous, and the
            // confidence should say so rather than present one reading as settled.
            confidence: 0.7,
            interpretation: subject.clone(),
            value: converted.best,
            detail: Some(serde_json::json!({
                "from": "arabizi",
                "to": "arabic",
                // Shown beneath the main reading. Not hidden behind a control: the alternative is
                // often the one the user meant, and a card that presents one guess as settled is
                // worse than one that admits there are several.
                "alternatives": converted.alternatives,
                "ambiguous": true,
            })),
            as_of: None,
        })
    }
}

/// A phrase reading, plus the runners-up for the whole phrase.
struct Phrase {
    best: String,
    alternatives: Vec<String>,
}

/// How many whole-phrase readings to offer.
///
/// Two, not more. `xustive-lang` ranks per token and the combinations multiply — three tokens with
/// four variants each is 64 phrasings, and a card listing them is noise rather than help.
const ALTERNATIVES: usize = 2;

/// Transliterate a whole phrase.
///
/// `xustive-lang` works one token at a time because that is what the *index* needs — it expands a
/// query into candidate spellings to match against. Reading a phrase is a different job: the user
/// wants sentences, so tokens are joined and only whole-phrase readings are offered.
fn to_arabic_phrase(subject: &str) -> Option<Phrase> {
    // Two characters, not the indexing default of three. The index refuses short tokens because
    // they match too much; here nothing is being matched, and `ana`, `rak`, `3la` are exactly the
    // words a phrase is made of.
    let cfg = TranslitConfig {
        min_token_len: 2,
        ..TranslitConfig::default()
    };

    let mut best: Vec<String> = Vec::new();
    let mut second: Vec<String> = Vec::new();
    let mut converted_any = false;

    for token in subject.split_whitespace() {
        let variants = translit::to_arabic(token, &cfg);
        match variants.first() {
            Some(top) => {
                converted_any = true;
                best.push(top.text.clone());
                // A token with no second reading contributes its only one, so the alternative
                // phrase stays a complete sentence rather than developing gaps.
                second.push(variants.get(1).unwrap_or(top).text.clone());
            }
            None => {
                // Untransliterable — a number, punctuation, or already Arabic. Passed through so
                // `3andi 500 dinar` keeps its 500.
                best.push(token.to_string());
                second.push(token.to_string());
            }
        }
    }

    if !converted_any || best.is_empty() {
        return None;
    }

    let joined = best.join(" ");
    let alt = second.join(" ");
    let alternatives = if alt != joined { vec![alt] } else { Vec::new() };

    Some(Phrase {
        best: joined,
        alternatives: alternatives.into_iter().take(ALTERNATIVES).collect(),
    })
}

/// Remove the trigger word and the particles that usually sit next to it.
fn strip(original: &str, lower: &str, trigger: &str) -> String {
    let Some(at) = lower.find(trigger) else {
        return original.trim().to_string();
    };
    // Operate on byte offsets from the lowercased copy. `to_lowercase` can change byte length for
    // some scripts, so take the remainder from `lower` and match it back conservatively: for the
    // scripts here (Latin and Arabic) lengths agree, and where they would not, the fallback of
    // using the whole query is harmless.
    let before = &original[..at.min(original.len())];
    let after_start = (at + trigger.len()).min(original.len());
    let after = &original[after_start..];

    let mut subject = format!("{before} {after}");
    for particle in [" to ", " en ", " into ", " vers ", " إلى ", ":"] {
        subject = subject.replace(particle, " ");
    }
    subject.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(q: &str) -> Option<String> {
        Transliterator.answer(q).map(|a| a.value)
    }

    #[test]
    fn an_explicit_request_converts() {
        let out = value("transliterate wach rak").expect("should convert");
        // The exact mapping is xustive-lang's business; what matters here is that the result is
        // Arabic script rather than the Latin input.
        assert!(
            out.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c)),
            "expected Arabic script, got {out:?}"
        );
        assert!(
            !out.contains("wach"),
            "input survived untranslated: {out:?}"
        );
    }

    #[test]
    fn the_digits_algerians_actually_use_are_handled() {
        // 3 → ع, 7 → ح, 9 → ق. These are the whole reason Arabizi works.
        let out = value("transliterate 3andi").expect("should convert");
        assert!(
            out.starts_with('\u{0639}'),
            "3 should map to ع, got {out:?}"
        );
    }

    #[test]
    fn ordinary_arabizi_does_not_trigger_a_card() {
        // Half the queries this engine sees are written this way. Offering to transliterate every
        // one would put a card above the results the user came for.
        for q in ["wach rak", "3andi mochkil", "khouya", "kifach ndir"] {
            assert!(
                Transliterator.answer(q).is_none(),
                "{q:?} should be searched, not transliterated"
            );
        }
    }

    #[test]
    fn text_already_in_arabic_is_not_converted() {
        // A conversion that changes nothing is not an answer.
        assert!(value("transliterate الجزائر").is_none());
    }

    #[test]
    fn the_answer_declares_its_own_ambiguity() {
        // Someone about to send this to another person needs to know it is a best reading.
        let answer = Transliterator
            .answer("transliterate wach rak")
            .expect("answer");
        assert_eq!(answer.detail.as_ref().unwrap()["ambiguous"], true);
        // And it is below the calculator, which is never ambiguous.
        assert!(answer.confidence < 0.95);
    }

    #[test]
    fn empty_and_malformed_input_does_not_panic() {
        for q in ["transliterate", "transliterate   ", "arabizi", "", "   "] {
            let _ = Transliterator.answer(q);
        }
    }
}
