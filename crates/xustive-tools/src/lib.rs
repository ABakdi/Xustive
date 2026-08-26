//! Instant answers.
//!
//! Answer the query directly when the query *is* the question. `45 * 1.19` wants a number, not
//! ten pages about multiplication.
//!
//! # The rule that governs everything here
//!
//! **A tool must be right, or absent.** Never approximately right, never confidently stale.
//!
//! A mediocre search result costs the user a click. A calculator that is wrong destroys the
//! reason to use the product at all. So a tool that cannot answer renders **nothing** — not an
//! error, not a placeholder. The results are already on the page.
//!
//! # Matching
//!
//! Every tool declares a matcher that is **pure, total and fast**: no I/O, no panics, and quick
//! enough to run all of them on every query. They run in order and the highest confidence wins.
//!
//! Confidence must reflect **how much of the query the tool explains**. That single rule is what
//! stops the calculator hijacking every query containing a digit — it matches a fragment of
//! `5 km en miles` and the unit converter consumes the whole thing, so the converter wins.
//!
//! Below [`MIN_CONFIDENCE`] nothing renders. An unwanted card pushing results down is worse than
//! no card.

pub mod calculator;
pub mod currency;
pub mod datetime;
pub mod deep;
pub mod exam;
pub mod fuel;
pub mod prayer;
pub mod translator;
pub mod transliterate;
pub mod units;
pub mod utilities;
pub mod weather;
pub mod wilaya;
mod wilaya_data;

use serde::Serialize;

/// Below this, no card. Deliberately high: the cost of a false positive is that the engine starts
/// interrupting ordinary searches, which is the regression users notice fastest.
pub const MIN_CONFIDENCE: f32 = 0.5;

/// A rendered instant answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Answer {
    /// Which tool produced this. The client picks a renderer from it.
    pub tool: &'static str,
    pub confidence: f32,
    /// How the query was read, shown in small text above the answer.
    ///
    /// Not decoration. `20 dollar` interpreted as USD when the user meant Canadian is a wrong
    /// answer; showing the interpretation lets them see it in half a second.
    pub interpretation: String,
    /// The answer itself, already formatted for display.
    pub value: String,
    /// Extra structured detail for interactive cards.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
    /// When the underlying data was measured. `None` means the answer is timeless — arithmetic
    /// has no `as_of`, an exchange rate always does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of: Option<i64>,
}

/// A tool that can answer a query directly.
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;

    /// Explicit invocation prefix, e.g. `calc` for `!calc`.
    fn keyword(&self) -> &'static str;

    /// Try to answer. `None` means this tool does not apply — which is the common case and must
    /// be cheap.
    fn answer(&self, query: &str) -> Option<Answer>;

    /// Answer with the reader's language in hand.
    ///
    /// Most tools produce output that is the same in every language — a hash, a Roman numeral, a
    /// hex colour — so the default ignores `lang` entirely and defers to [`Tool::answer`]. Tools
    /// that name things override it: a fuel price answered as "Essence sans plomb" to someone who
    /// asked in Arabic is answering a different person's question.
    ///
    /// This is a trait method rather than a match on `name()` in the dispatcher because that
    /// match was already two cases from becoming the place every new tool has to be registered
    /// twice.
    fn answer_in(&self, query: &str, lang: &str) -> Option<Answer> {
        let _ = lang;
        self.answer(query)
    }
}

/// The tools, in precedence order.
///
/// Order only breaks ties in confidence. It exists because overlaps are real: `5 km en miles`
/// matches both the calculator and the converter, and the wrong resolution is embarrassing.
pub fn registry() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(calculator::Calculator),
        Box::new(units::UnitConverter),
        Box::new(datetime::DateTool),
        Box::new(prayer::PrayerTool),
        Box::new(fuel::FuelTool),
        Box::new(exam::ExamTool),
        Box::new(wilaya::WilayaTool),
        Box::new(utilities::Utilities),
        // The translation card, surfaced on an explicit translate/traduire/ترجم verb. The local
        // model's output *into Arabic* is still weak (see the translator module docs) — that is a
        // model limitation the card states plainly, not a reason to hide a feature the operator
        // asked for. The card is interactive and dismissible, and translation into fr/en/es is good.
        Box::new(translator::Translator),
        Box::new(transliterate::Transliterator),
    ]
}

/// Run every matcher and return the best answer, if any.
///
/// A tool that panics is **caught and skipped**. A matcher is a small pure function and should
/// never panic, but a search engine that returns 500 because a unit converter tripped over a
/// malformed number has its priorities inverted.
pub fn best(raw: &str) -> Option<Answer> {
    best_in(raw, "en")
}

/// As [`best`], but rendering unit names and labels in `lang`.
pub fn best_in(raw: &str, lang: &str) -> Option<Answer> {
    let query = raw.trim();
    if query.is_empty() {
        return None;
    }

    let tools = registry();

    // Explicit invocation skips arbitration entirely — for when inference gets it wrong and the
    // user already knows what they want.
    if let Some(rest) = query.strip_prefix('!') {
        let (keyword, operand) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let tool = tools.iter().find(|t| t.keyword() == keyword)?;
        return catch(|| localised(tool.as_ref(), operand.trim(), lang));
    }

    tools
        .iter()
        .filter_map(|tool| catch(|| localised(tool.as_ref(), query, lang)))
        .filter(|answer| answer.confidence >= MIN_CONFIDENCE)
        .max_by(|a, b| {
            a.confidence
                .partial_cmp(&b.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// Dispatch to a tool's localised entry point where it has one.
///
/// Only the converter renders language-dependent text today. A trait method taking a language
/// would push that concern into every tool that does not need it.
fn localised(tool: &dyn Tool, query: &str, lang: &str) -> Option<Answer> {
    tool.answer_in(query, lang)
}

fn catch(f: impl FnOnce() -> Option<Answer>) -> Option<Answer> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(None)
}

/// Normalise a query for matching.
///
/// Arabic-Indic digits fold to ASCII so `٤٥ + ٥` is arithmetic, and Arabic decimal and thousands
/// separators fold too. Algerians type both digit sets, frequently in the same expression.
pub fn fold_digits(input: &str) -> String {
    input
        .chars()
        .map(|c| match c {
            '\u{0660}'..='\u{0669}' => char::from(b'0' + (c as u32 - 0x0660) as u8),
            '\u{06F0}'..='\u{06F9}' => char::from(b'0' + (c as u32 - 0x06F0) as u8),
            '\u{066B}' => '.', // Arabic decimal separator
            '\u{066C}' => ',', // Arabic thousands separator
            '×' | '⋅' | '∗' => '*',
            '÷' => '/',
            '−' | '–' | '—' => '-',
            '٪' => '%',
            other => other,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ordinary_query_matches_no_tool() {
        // The single most important property. A search engine that interrupts normal searches
        // with a calculator card is worse than one with no tools at all.
        for query in [
            "الجزائر",
            "prix du gaz butane à Alger",
            "Sonelgaz consommation record",
            "horaires trains Oran Alger",
            "ch7al ta3 sarf euro",
            "météo",
            "covid 19",
            "windows 11 download",
            "article 51 constitution",
        ] {
            assert!(
                best(query).is_none(),
                "{query:?} should not activate a tool, got {:?}",
                best(query).map(|a| a.tool)
            );
        }
    }

    #[test]
    fn arithmetic_is_answered() {
        let answer = best("45*1.19").expect("an expression should answer");
        assert_eq!(answer.tool, "calculator");
        assert_eq!(answer.value, "53.55");
    }

    #[test]
    fn a_unit_conversion_beats_the_calculator() {
        // Both match: the calculator sees a number, the converter consumes the whole string.
        // Confidence reflects how much of the query is explained, which is what decides this.
        let answer = best("5 km to miles").expect("should answer");
        assert_eq!(answer.tool, "unit-converter", "got {answer:?}");
    }

    #[test]
    fn explicit_invocation_skips_arbitration() {
        let answer = best("!calc 2+2").expect("explicit invocation should answer");
        assert_eq!(answer.tool, "calculator");
        assert_eq!(answer.value, "4");
    }

    #[test]
    fn an_unknown_explicit_tool_answers_nothing() {
        assert!(best("!nonsense 2+2").is_none());
    }

    #[test]
    fn arabic_indic_digits_are_arithmetic_too() {
        let answer = best("٤٥ + ٥").expect("Arabic-Indic digits should work");
        assert_eq!(answer.value, "50");
    }

    #[test]
    fn the_confidence_floor_is_high_enough_to_be_meaningful() {
        const { assert!(MIN_CONFIDENCE >= 0.5) };
    }

    #[test]
    fn an_empty_query_answers_nothing() {
        assert!(best("").is_none());
        assert!(best("   ").is_none());
    }
}
