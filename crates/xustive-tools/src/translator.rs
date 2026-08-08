//! Detecting a translation request.
//!
//! Pure, like every matcher: it decides *whether* a query asks for a translation and what the
//! operands are. It does no generation — that runs on the model, behind a streaming endpoint the
//! card calls, because a matcher that blocked on a 3B model would put ten seconds on every search
//! that is not a translation.
//!
//! # Not currently registered
//!
//! This detector is correct and tested, and the endpoint behind it streams and cancels properly.
//! It is **left out of the registry** because the local model's output into Arabic is not good
//! enough to show anyone. Measured on Qwen2.5-3B-Instruct Q4_K_M, CPU:
//!
//! | Direction | Output |
//! |:---|:---|
//! | ar → fr | `Où est la pharmacie la plus proche ?` — correct |
//! | ar → en | `Where is the nearest pharmacy?` — correct |
//! | fr → en | `Hello my friend, how are you?` — correct |
//! | en → **ar** | `أين closest الصيدلية؟` — an English synonym substituted mid-sentence |
//! | fr → **ar** | `أين تقع الم下车؟` — a Chinese one |
//!
//! Arabic as a *source* is fine; Arabic as a *target* is not. The substituted word is always a
//! semantic equivalent of the correct Arabic one, which is the signature of a heavily quantised
//! multilingual model resolving to a cross-lingual neighbour rather than of a bug in the pipeline.
//!
//! Ruled out, each by measurement rather than reasoning: sampling (three configurations, byte-
//! identical output), the repetition penalty, an instruction restated in Arabic, a worked example,
//! and the prompt delimiter. The summariser produces fluent Arabic through the same engine and the
//! same model, so the engine is not at fault — its context is full of Arabic passages, which is
//! precisely what a translation prompt cannot supply.
//!
//! Registering it would put `أين closest الصيدلية؟` above the results in this engine's primary
//! language. The fix is a better model for this task, not more prompt work.
//!
//! # Why this demands an explicit verb
//!
//! Every query is text in some language, so "looks translatable" describes all of them. The only
//! usable signal is someone saying so: `translate`, `traduire`, `ترجم`, `معنى`. Without that the
//! card would appear above every result page in the engine.

use crate::{Answer, Tool};

pub struct Translator;

/// Verbs that ask for a translation, longest first so `traduire en` is not cut at `traduire`.
const VERBS: &[&str] = &[
    "translate",
    "translation of",
    "traduire",
    "traduction de",
    "traduction",
    "ترجم",
    "ترجمة",
    "معنى",
    "شنو معنى",
    "wach ma3na",
    "ma3na",
];

/// Language names as a user would type them, mapped to the codes the API serves.
///
/// Each language is listed under its own name and its names in the other three interface
/// languages, because someone writing in French asks for `en arabe` and someone writing in Arabic
/// asks for `إلى الإنجليزية`.
const NAMES: &[(&str, &str)] = &[
    ("arabic", "ar"),
    ("arabe", "ar"),
    ("العربية", "ar"),
    ("عربية", "ar"),
    ("darija", "ary"),
    ("درجة", "ary"),
    ("الدارجة", "ary"),
    ("french", "fr"),
    ("français", "fr"),
    ("francais", "fr"),
    ("الفرنسية", "fr"),
    ("english", "en"),
    ("anglais", "en"),
    ("الإنجليزية", "en"),
    ("انجليزية", "en"),
    ("spanish", "es"),
    ("espagnol", "es"),
    ("الإسبانية", "es"),
    ("german", "de"),
    ("allemand", "de"),
    ("الألمانية", "de"),
    ("italian", "it"),
    ("italien", "it"),
    ("الإيطالية", "it"),
    ("turkish", "tr"),
    ("turc", "tr"),
    ("التركية", "tr"),
];

/// Particles that introduce the target language.
const INTO: &[&str] = &[" to ", " into ", " en ", " vers ", " إلى ", " الى ", " ل"];

/// What the query asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub text: String,
    /// `None` means auto-detect, which is the common case — people say what they want it *in*,
    /// rarely what it is already.
    pub from: Option<&'static str>,
    pub to: &'static str,
}

/// Parse a translation request, or nothing.
pub fn detect(query: &str, ui_lang: &str) -> Option<Request> {
    let lower = query.trim().to_lowercase();
    if lower.is_empty() {
        return None;
    }

    // The verb must be present. Every query is text in some language, so without one the card
    // would appear above every result page in the engine.
    let verb = VERBS
        .iter()
        .filter(|v| lower.contains(**v))
        .max_by_key(|v| v.len())?;

    let at = lower.find(verb)?;
    let after = &query[(at + verb.len()).min(query.len())..];

    // Find a target language named after the verb. The *last* one wins: in "translate from French
    // to Arabic" both appear, and the one being translated into is the later.
    let after_lower = after.to_lowercase();
    let target = NAMES
        .iter()
        .filter_map(|(name, code)| after_lower.rfind(name).map(|at| (at, *name, *code)))
        .max_by_key(|(at, name, _)| (*at, name.len()));

    let (to, cut) = match target {
        Some((at, name, code)) => (code, Some((at, name.len()))),
        // No language named. Default to the interface language — someone typing `ترجم hello` in
        // the Arabic UI wants Arabic, and asking them to say so would be pedantry.
        None => (default_target(ui_lang), None),
    };

    // A source language named *before* the target, introduced by "from"/"de"/"من".
    let from = NAMES
        .iter()
        .filter(|(name, code)| {
            *code != to
                && ["from ", "de ", "من ", "depuis "]
                    .iter()
                    .any(|p| after_lower.contains(&format!("{p}{name}")))
        })
        .map(|(_, code)| *code)
        .next();

    let text = strip(after, cut, from.is_some());
    if text.is_empty() {
        return None;
    }

    Some(Request { text, from, to })
}

fn default_target(ui_lang: &str) -> &'static str {
    match ui_lang {
        "ary" => "ary",
        "fr" => "fr",
        "en" => "en",
        _ => "ar",
    }
}

/// Remove the language clause, leaving the text to translate.
fn strip(after: &str, cut: Option<(usize, usize)>, had_source: bool) -> String {
    let mut text = after.to_string();

    if let Some((at, len)) = cut {
        // Drop the language name and the particle introducing it. The particle is searched for
        // backwards from the name so `to Arabic` loses both words, not just the second.
        let head = &text[..at.min(text.len())];
        let tail = &text[(at + len).min(text.len())..];
        let head = INTO
            .iter()
            .filter_map(|p| head.to_lowercase().rfind(p).map(|i| (i, p.len())))
            .max_by_key(|(i, _)| *i)
            .map_or(head, |(i, _)| &head[..i]);
        text = format!("{head} {tail}");
    }

    if had_source {
        let lowered = text.to_lowercase();
        for particle in ["from ", "de ", "من ", "depuis "] {
            if let Some(i) = lowered.find(particle) {
                // Everything from the source clause to the end of that word.
                let rest = &text[i..];
                let end = rest
                    .char_indices()
                    .skip(particle.len())
                    .find(|(_, c)| c.is_whitespace())
                    .map_or(rest.len(), |(j, _)| j);
                text = format!("{}{}", &text[..i], &rest[end..]);
                break;
            }
        }
    }

    // Leading connectors left behind by the verb: `translate: hello`, `ترجم لي hello`.
    let text = text.trim().trim_start_matches([':', '،', ',']).trim();
    text.trim_start_matches("لي ").trim().to_string()
}

impl Tool for Translator {
    fn name(&self) -> &'static str {
        "translate"
    }

    fn keyword(&self) -> &'static str {
        "tr"
    }

    fn answer(&self, query: &str) -> Option<Answer> {
        self.answer_in(query, "ar")
    }

    fn answer_in(&self, query: &str, lang: &str) -> Option<Answer> {
        let request = detect(query, lang)?;

        Some(Answer {
            tool: "translate",
            // Below the calculator and the utilities. The verb is a strong signal but the operand
            // split is a guess, and the card is interactive — the reader can correct both
            // languages, which is why being slightly wrong here is recoverable.
            confidence: 0.85,
            interpretation: request.text.clone(),
            // Empty on purpose. The value arrives by stream from `/translate`; putting a
            // placeholder here would make a card that never fills in look finished.
            value: String::new(),
            detail: Some(serde_json::json!({
                "text": request.text,
                "from": request.from,
                "to": request.to,
                // Says outright that nothing has been produced yet, so a client that ignores the
                // stream renders a pending state rather than a blank answer.
                "pending": true,
            })),
            as_of: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(q: &str, ui: &str) -> Option<Request> {
        detect(q, ui)
    }

    #[test]
    fn an_explicit_request_names_text_and_target() {
        let r = req("translate hello to arabic", "en").expect("detected");
        assert_eq!(r.text, "hello");
        assert_eq!(r.to, "ar");
        assert_eq!(r.from, None);
    }

    #[test]
    fn the_target_is_the_later_language_when_both_are_named() {
        // "from French to Arabic" names two. The one being translated *into* is the later.
        let r = req("translate bonjour from french to arabic", "en").expect("detected");
        assert_eq!(r.to, "ar");
        assert_eq!(r.from, Some("fr"));
        assert_eq!(r.text, "bonjour");
    }

    #[test]
    fn an_unnamed_target_falls_back_to_the_interface_language() {
        // Someone typing `ترجم hello` in the Arabic interface wants Arabic. Asking them to say so
        // would be pedantry.
        assert_eq!(req("ترجم hello", "ar").unwrap().to, "ar");
        assert_eq!(req("traduire hello", "fr").unwrap().to, "fr");
        assert_eq!(req("translate bonjour", "en").unwrap().to, "en");
    }

    #[test]
    fn the_request_is_recognised_in_four_languages() {
        for (q, ui, to) in [
            ("translate hello to french", "en", "fr"),
            ("traduire hello en arabe", "fr", "ar"),
            ("ترجم hello إلى العربية", "ar", "ar"),
            ("ترجم hello الى الفرنسية", "ar", "fr"),
        ] {
            let r = req(q, ui).unwrap_or_else(|| panic!("{q:?} not detected"));
            assert_eq!(r.to, to, "{q:?}");
            assert!(
                r.text.contains("hello"),
                "{q:?} lost its text: {:?}",
                r.text
            );
        }
    }

    #[test]
    fn a_query_without_a_verb_is_not_a_translation_request() {
        // Every query is text in some language, so without an explicit verb this card would
        // appear above every result page in the engine.
        for q in [
            "الجزائر",
            "bonjour",
            "hello world",
            "prix du gaz",
            "how to say hello in arabic", // no verb we accept; a search, not a request
        ] {
            assert!(req(q, "en").is_none(), "{q:?} should not be a translation");
        }
    }

    #[test]
    fn a_verb_with_no_text_is_not_a_request() {
        for q in [
            "translate",
            "translate to arabic",
            "ترجم",
            "traduire en arabe",
        ] {
            assert!(req(q, "en").is_none(), "{q:?} has nothing to translate");
        }
    }

    #[test]
    fn the_answer_is_marked_pending_and_carries_no_value() {
        // The value arrives by stream. A placeholder here would make a card that never fills in
        // look finished.
        let answer = Translator
            .answer_in("translate hello to arabic", "en")
            .expect("answer");
        assert!(answer.value.is_empty());
        let detail = answer.detail.as_ref().unwrap();
        assert_eq!(detail["pending"], true);
        assert_eq!(detail["to"], "ar");
        assert_eq!(detail["text"], "hello");
    }

    #[test]
    fn a_multi_word_phrase_survives_intact() {
        let r = req("translate good morning my friend to french", "en").expect("detected");
        assert_eq!(r.text, "good morning my friend");
        assert_eq!(r.to, "fr");
    }

    #[test]
    fn punctuation_after_the_verb_is_dropped() {
        assert_eq!(
            req("translate: hello to arabic", "en").unwrap().text,
            "hello"
        );
    }

    #[test]
    fn malformed_input_does_not_panic() {
        for q in [
            "translate to",
            "ترجم إلى",
            "translate   ",
            "",
            "   ",
            "translate to to to",
        ] {
            let _ = detect(q, "ar");
        }
    }
}
