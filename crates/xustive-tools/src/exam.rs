//! Exam-results links (M1B-T07.6).
//!
//! # Why this returns links and never results
//!
//! A student's exam result is the most sensitive query an Algerian search engine will ever see, and
//! the ONERC/education-ministry portals are the sole authoritative source. Mirroring a result — even
//! caching it — would mean this engine could show a wrong grade, a stale one, or one it had no right
//! to hold. So this tool does one thing: it recognises "where are the BAC results" and points at the
//! official portal. It never fetches, stores, or displays a result.
//!
//! That restraint is the feature. The value is saving a panicked student the search for the real URL
//! among the SEO-spam mirror sites that spring up every July — not becoming another one.
//!
//! # What it recognises
//!
//! The three national exams, in the three languages people ask in: **BAC** (باكالوريا / bac), **BEM**
//! (شهادة التعليم المتوسط) and the **cinquième / السنة الخامسة** primary certificate. Each maps to its
//! official portal.

use crate::{Answer, Tool};

/// One exam and where its results actually live.
struct Exam {
    /// The `kind` the client renders from.
    kind: &'static str,
    /// Official results portal. The only URL this tool will ever emit.
    portal: &'static str,
    /// Query fragments that name this exam, across ar / fr / en, already lowercased. Matched as
    /// substrings after the query is lowercased, so "natidjat bac 2026" and "resultat du bac" both
    /// hit.
    triggers: &'static [&'static str],
}

const EXAMS: &[Exam] = &[
    Exam {
        kind: "bac",
        portal: "https://bac.onec.dz",
        triggers: &[
            "باكالوريا",
            "بكالوريا",
            "bac",
            "baccalaureat",
            "baccalauréat",
        ],
    },
    Exam {
        kind: "bem",
        portal: "https://bem.onec.dz",
        triggers: &["شهادة التعليم المتوسط", "التعليم المتوسط", "bem", "brevet"],
    },
    Exam {
        kind: "cinq",
        portal: "https://cinq.onec.dz",
        triggers: &[
            "السنة الخامسة",
            "التعليم الابتدائي",
            "cinquieme",
            "cinquième",
            "5eme",
        ],
    },
];

/// Words that signal the query is *about results*, not merely mentioning an exam.
///
/// "bac results" activates; "bac 2015 english syllabus" does not, because the bare exam name is a
/// common topic and answering every mention of it with a portal link would be noise. One of these
/// must be present, in any of the three languages.
const RESULT_WORDS: &[&str] = &[
    "نتائج",
    "نتيجة",
    "نتيجه",
    "resultat",
    "résultat",
    "results",
    "result",
    "natidja",
    "natija",
];

pub struct ExamTool;

impl Tool for ExamTool {
    fn name(&self) -> &'static str {
        "exam"
    }

    fn keyword(&self) -> &'static str {
        "exam"
    }

    fn answer(&self, query: &str) -> Option<Answer> {
        let q = query.trim().to_lowercase();

        // "results" intent is required, or the bare exam name — a common topic — would trigger on
        // every mention. The Arabic result words carry no case, but lowercasing is harmless to them.
        if !RESULT_WORDS.iter().any(|w| q.contains(w)) {
            return None;
        }

        let exam = EXAMS
            .iter()
            .find(|e| e.triggers.iter().any(|t| q.contains(t)))?;

        Some(Answer {
            tool: "exam",
            // Below the pure calculators: a keyword match is weaker evidence than a string that
            // parses. High enough to surface for an unambiguous "نتائج البكالوريا".
            confidence: 0.82,
            interpretation: String::new(),
            value: exam.portal.to_string(),
            // The client renders a link card from this, never a result. `official: true` is the
            // contract that this URL is the authoritative portal and nothing is mirrored.
            detail: Some(serde_json::json!({
                "kind": exam.kind,
                "portal": exam.portal,
                "official": true,
            })),
            as_of: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ans(q: &str) -> Option<Answer> {
        ExamTool.answer(q)
    }

    #[test]
    fn recognises_bac_results_in_three_languages() {
        for q in ["نتائج البكالوريا", "resultats du bac 2026", "bac results"] {
            let a = ans(q).unwrap_or_else(|| panic!("{q:?} should activate"));
            assert_eq!(a.value, "https://bac.onec.dz");
        }
    }

    #[test]
    fn recognises_bem_and_cinquieme() {
        assert_eq!(
            ans("نتائج شهادة التعليم المتوسط").unwrap().value,
            "https://bem.onec.dz"
        );
        assert_eq!(
            ans("resultat cinquieme").unwrap().value,
            "https://cinq.onec.dz"
        );
    }

    /// The bare exam name is a topic, not a results query. Answering it would be noise.
    #[test]
    fn a_mention_without_results_intent_does_not_activate() {
        for q in [
            "bac 2015 english syllabus",
            "باكالوريا آداب",
            "what is the bem",
        ] {
            assert!(ans(q).is_none(), "{q:?} should not activate the exam tool");
        }
    }

    /// The only URL this tool emits is an official portal. This is the whole safety property: it
    /// must never point anywhere it could show a wrong or mirrored result.
    #[test]
    fn only_ever_emits_an_official_onec_portal() {
        for q in [
            "نتائج البكالوريا",
            "resultat bem",
            "bac results",
            "resultat cinquieme",
        ] {
            let a = ans(q).unwrap();
            assert!(
                a.value.starts_with("https://") && a.value.ends_with(".onec.dz"),
                "{q:?} produced a non-official URL: {}",
                a.value
            );
            let detail = a.detail.unwrap();
            assert_eq!(detail["official"], serde_json::json!(true));
        }
    }

    #[test]
    fn is_deterministic() {
        for q in ["نتائج البكالوريا", "bac results"] {
            assert_eq!(ans(q), ans(q));
        }
    }
}
