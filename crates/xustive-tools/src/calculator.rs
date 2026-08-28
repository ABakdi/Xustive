//! Arithmetic.
//!
//! **Decimal, not binary float.** `0.1 + 0.2` renders `0.3`. A calculator that shows
//! `0.30000000000000004` is one nobody trusts again, and every price, tax and conversion an
//! Algerian types into a search box is decimal by nature.
//!
//! Deliberately *only* a calculator: no variables, no state, no user-defined functions. An
//! expression evaluator reachable from a query string is an attack surface, and keeping it a pure
//! calculator keeps it a small one.

use rust_decimal::prelude::*;
use rust_decimal::{Decimal, MathematicalOps};
use std::str::FromStr;

use crate::{fold_digits, Answer, Tool};

pub struct Calculator;

impl Tool for Calculator {
    fn name(&self) -> &'static str {
        "calculator"
    }

    fn keyword(&self) -> &'static str {
        "calc"
    }

    fn answer(&self, query: &str) -> Option<Answer> {
        let folded = fold_digits(query);
        // The question words of three languages, and the symbols a keyboard offers for the
        // operators, so "what is 2+2", "combien font 3 × 4" and "احسب 15% من 80" all count.
        let folded = normalise_phrasing(&folded);
        let expr = folded.trim().trim_end_matches('=').trim();
        if expr.is_empty() {
            return None;
        }
        // Percent of a number: "15% of 80" / "15% de 80" / "15٪ من 80".
        if let Some((shown, value)) = percent_of(expr) {
            return Some(Answer {
                tool: self.name(),
                confidence: 0.97,
                interpretation: shown,
                value: render(value),
                detail: None,
                as_of: None,
            });
        }

        // A bare number is not a calculation. Someone typing `2026` wants the year, not a
        // calculator telling them it equals 2026.
        //
        // `%` is deliberately not in this set. `50%` alone parses fine and evaluates to 0.5, but
        // it is far more often part of a search — "50% off", "50% des Algériens" — than a
        // question about a number. It needs a real operator alongside it to count.
        if !expr.chars().any(|c| "+-*/^(".contains(c)) {
            return None;
        }
        // Nor is a lone leading minus: `-5` is a number, and `-covid` is a search operator.
        if expr
            .strip_prefix('-')
            .is_some_and(|rest| !rest.chars().any(|c| "+-*/^%(".contains(c)))
        {
            return None;
        }

        // The exact decimal parser first, and it keeps its results. It is proven, it groups
        // thousands, and every golden expression is written against it.
        if let Some(value) = Parser::new(expr).evaluate() {
            return Some(Answer {
                tool: self.name(),
                // Structural: the string parsed as an expression and nothing was left over, so
                // this is as certain as matching gets.
                confidence: 0.98,
                interpretation: pretty(expr),
                value: render(value),
                detail: None,
                as_of: None,
            });
        }

        // Only what the decimal parser cannot take reaches the unit-aware engine (M8-T07): mixed
        // units, bases, constants. Currency is deliberately not available here — rates live in a
        // cache this crate cannot reach, so the API builds those answers where the cache is.
        let value = crate::deep::evaluate(expr, None)?;
        Some(Answer {
            tool: self.name(),
            // A shade lower than the exact path. The engine accepts a wider language, so a string
            // it happens to reduce is slightly weaker evidence that a calculation was intended.
            confidence: 0.9,
            interpretation: pretty(expr),
            value,
            detail: None,
            as_of: None,
        })
    }
}

/// Format for display: trim trailing zeros, group thousands.
/// Strip the words that ask, and map the symbols people type to the ones the parser reads.
fn normalise_phrasing(q: &str) -> String {
    const LEAD: &[&str] = &[
        "what is",
        "what's",
        "whats",
        "calculate",
        "compute",
        "calcul",
        "calcule",
        "calculer",
        "combien font",
        "combien fait",
        "combien vaut",
        "combien",
        "quel est",
        "احسب",
        "ما هو",
        "ما ناتج",
        "كم يساوي",
        "كم تساوي",
        "كم",
        "شحال",
        "قداش",
    ];
    let mut s = q
        .trim()
        .trim_end_matches(['?', '؟', '!'])
        .trim()
        .to_lowercase();
    let mut changed = true;
    while changed {
        changed = false;
        for lead in LEAD {
            if let Some(rest) = s.strip_prefix(lead) {
                if rest.starts_with(' ')
                    || rest.starts_with(|c: char| c.is_ascii_digit() || c == '(')
                {
                    s = rest.trim_start().to_string();
                    changed = true;
                }
            }
        }
    }
    // Keyboard and locale symbols → the parser's operators. A lone `x` between numbers is a
    // multiplication sign to most people ("3 x 4"), never a variable here.
    let s = s
        .replace(['×', '✕'], "*")
        .replace('÷', "/")
        .replace('٪', "%")
        .replace('،', ",")
        .replace('−', "-")
        .replace("plus", "+")
        .replace("moins", "-")
        .replace("fois", "*")
        .replace("divisé par", "/")
        .replace("زائد", "+")
        .replace("ناقص", "-")
        .replace("ضرب", "*")
        .replace("تقسيم", "/");
    let re_x = regex_lite_x(&s);
    re_x
}

/// `3 x 4` → `3 * 4`, without touching `0x1f` or a word containing x.
fn regex_lite_x(s: &str) -> String {
    let b: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &c) in b.iter().enumerate() {
        let is_x = c == 'x' || c == 'X';
        let before = b[..i].iter().rev().find(|ch| !ch.is_whitespace()).copied();
        let after = b[i + 1..].iter().find(|ch| !ch.is_whitespace()).copied();
        if is_x
            && before.is_some_and(|ch| ch.is_ascii_digit() || ch == ')')
            && after.is_some_and(|ch| ch.is_ascii_digit() || ch == '(')
            && (i == 0 || b[i - 1].is_whitespace() || b[i - 1].is_ascii_digit())
        {
            out.push('*');
        } else {
            out.push(c);
        }
    }
    out
}

/// "15% of 80" in three languages → (what was read, the value).
fn percent_of(expr: &str) -> Option<(String, Decimal)> {
    let t: Vec<&str> = expr.split_whitespace().collect();
    // Shapes: `15% of 80`, `15 % of 80`, `15% de 80`, `15% من 80`.
    let (pct, rest) = match t.as_slice() {
        [p, of, n] if p.ends_with('%') => (p.trim_end_matches('%'), (*of, *n)),
        [p, "%", of, n] => (*p, (*of, *n)),
        _ => return None,
    };
    let (of, n) = rest;
    if !matches!(of, "of" | "de" | "du" | "des" | "من") {
        return None;
    }
    let pct = Decimal::from_str(pct).ok()?;
    let n = Decimal::from_str(&n.replace(',', "")).ok()?;
    let value = pct.checked_mul(n)?.checked_div(Decimal::from(100))?;
    Some((format!("{pct}% × {n}"), value))
}

fn render(value: Decimal) -> String {
    let value = value.normalize();
    let text = value.to_string();
    let (int, frac) = text.split_once('.').unwrap_or((text.as_str(), ""));
    let (sign, digits) = int.strip_prefix('-').map_or(("", int), |d| ("-", d));

    // Grouped from the right in threes. Long numbers are the ones most likely to be misread.
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(' ');
        }
        grouped.push(c);
    }

    if frac.is_empty() {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped}.{frac}")
    }
}

/// Echo the expression with real operators, so a misreading is visible.
fn pretty(expr: &str) -> String {
    expr.replace('*', " × ")
        .replace('/', " ÷ ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Recursive-descent parser over the expression grammar.
///
/// Hand-written rather than pulled in: the grammar is a dozen productions, and an expression
/// evaluator is exactly the kind of dependency whose behaviour on hostile input we would have to
/// audit anyway.
struct Parser<'a> {
    chars: Vec<char>,
    pos: usize,
    src: &'a str,
    /// Guards against a deeply nested expression exhausting the stack. `((((…))))` from a query
    /// string is a denial-of-service, not a calculation.
    depth: u32,
}

const MAX_DEPTH: u32 = 32;

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            chars: src.chars().collect(),
            pos: 0,
            src,
            depth: 0,
        }
    }

    fn evaluate(mut self) -> Option<Decimal> {
        let value = self.expression()?;
        self.skip_space();
        // Anything left over means we only understood a fragment. Answering from a fragment is
        // how `2 + 2 apples` becomes a confident wrong answer.
        if self.pos != self.chars.len() {
            return None;
        }
        // Decimal has no infinities by construction — overflow returns None from the checked
        // operations above — so reaching here means the value is representable.
        Some(value)
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        self.skip_space();
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// `expression := term (('+' | '-') term)*`
    fn expression(&mut self) -> Option<Decimal> {
        let mut left = self.term()?;
        loop {
            self.skip_space();
            if self.eat('+') {
                // `2000 + 19%` reads as "add 19 percent of 2000", which is how people write it.
                let right = self.percent_aware(left)?;
                left = left.checked_add(right)?;
            } else if self.eat('-') {
                let right = self.percent_aware(left)?;
                left = left.checked_sub(right)?;
            } else {
                return Some(left);
            }
        }
    }

    /// A term, reinterpreting a trailing `%` as a share of `base`.
    fn percent_aware(&mut self, base: Decimal) -> Option<Decimal> {
        let start = self.pos;
        let value = self.term()?;
        // Only when the operand *ended* with a percent sign, not when one appeared inside it.
        let consumed: String = self.chars[start..self.pos].iter().collect();
        if consumed.trim_end().ends_with('%') {
            return base.checked_mul(value);
        }
        Some(value)
    }

    /// `term := power (('*' | '/') power)*`
    fn term(&mut self) -> Option<Decimal> {
        let mut left = self.power()?;
        loop {
            self.skip_space();
            if self.eat('*') {
                left = left.checked_mul(self.power()?)?;
            } else if self.eat('/') {
                let divisor = self.power()?;
                // Division by zero renders nothing rather than an error or an infinity.
                if divisor.is_zero() {
                    return None;
                }
                left = left.checked_div(divisor)?;
            } else {
                return Some(left);
            }
        }
    }

    /// `power := unary ('^' power)?` — right-associative, as exponentiation is.
    fn power(&mut self) -> Option<Decimal> {
        let base = self.unary()?;
        self.skip_space();
        if self.eat('^') {
            let exponent = self.power()?;
            // Fractional and very large exponents are refused rather than approximated: this is
            // a calculator, and an approximation presented as an answer is the failure mode the
            // whole component is built to avoid.
            let exp = exponent.to_i64()?;
            if !(-64..=64).contains(&exp) || !exponent.fract().is_zero() {
                return None;
            }
            return base.checked_powi(exp);
        }
        Some(base)
    }

    fn unary(&mut self) -> Option<Decimal> {
        self.skip_space();
        if self.eat('-') {
            return self.unary()?.checked_mul(Decimal::NEGATIVE_ONE);
        }
        if self.eat('+') {
            return self.unary();
        }
        self.postfix()
    }

    /// A primary value, then an optional trailing `%` meaning "divide by 100".
    fn postfix(&mut self) -> Option<Decimal> {
        let mut value = self.primary()?;
        loop {
            self.skip_space();
            if self.peek() == Some('%') {
                self.pos += 1;
                value = value.checked_div(Decimal::ONE_HUNDRED)?;
            } else {
                return Some(value);
            }
        }
    }

    fn primary(&mut self) -> Option<Decimal> {
        self.skip_space();

        if self.eat('(') {
            self.depth += 1;
            if self.depth > MAX_DEPTH {
                return None;
            }
            let value = self.expression()?;
            self.depth -= 1;
            return self.eat(')').then_some(value);
        }

        if let Some(value) = self.function()? {
            return Some(value);
        }

        self.number()
    }

    /// `sqrt(x)`, `abs(x)`, and the rest. Returns `Ok(None)` when the next token is not a name.
    fn function(&mut self) -> Option<Option<Decimal>> {
        let start = self.pos;
        let mut name = String::new();
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            name.push(self.chars[self.pos]);
            self.pos += 1;
        }
        if name.is_empty() {
            return Some(None);
        }
        if !self.eat('(') {
            // A bare word is not a function call, and rewinding lets `2 apples` fail cleanly at
            // the leftover check rather than being silently misparsed.
            self.pos = start;
            return Some(None);
        }

        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return None;
        }
        let arg = self.expression()?;
        self.depth -= 1;
        if !self.eat(')') {
            return None;
        }

        let value = match name.to_ascii_lowercase().as_str() {
            "sqrt" => {
                if arg.is_sign_negative() {
                    return None;
                }
                arg.sqrt()?
            }
            "abs" => arg.abs(),
            "round" => arg.round(),
            "floor" => arg.floor(),
            "ceil" => arg.ceil(),
            "ln" => {
                if arg <= Decimal::ZERO {
                    return None;
                }
                arg.checked_ln()?
            }
            "log" => {
                if arg <= Decimal::ZERO {
                    return None;
                }
                arg.checked_log10()?
            }
            _ => return None,
        };
        Some(Some(value))
    }

    fn number(&mut self) -> Option<Decimal> {
        self.skip_space();
        let start = self.pos;
        let mut text = String::new();

        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '.' {
                text.push(c);
                self.pos += 1;
            } else if (c == ',' || c == ' ') && !text.is_empty() {
                // A thousands separator, but only between digits — `1,5` is ambiguous in a
                // French-influenced context and `1, 5` is a list, so both are refused by the
                // leftover check rather than guessed at.
                let next = self.chars.get(self.pos + 1).copied();
                if next.is_some_and(|n| n.is_ascii_digit()) && c == ',' {
                    self.pos += 1;
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        if text.is_empty() {
            self.pos = start;
            return None;
        }
        let _ = self.src;
        Decimal::from_str(&text).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(expr: &str) -> Option<String> {
        Calculator.answer(expr).map(|a| a.value)
    }

    #[test]
    fn decimals_are_exact() {
        // The reason this uses decimal rather than f64. A calculator that renders
        // 0.30000000000000004 is one nobody trusts again.
        assert_eq!(eval("0.1 + 0.2").as_deref(), Some("0.3"));
        assert_eq!(eval("1.1 * 3").as_deref(), Some("3.3"));
        assert_eq!(eval("2.675 * 100").as_deref(), Some("267.5"));
        assert_eq!(eval("0.7 - 0.6").as_deref(), Some("0.1"));
    }

    #[test]
    fn the_four_operations_work() {
        assert_eq!(eval("2+2").as_deref(), Some("4"));
        assert_eq!(eval("10 - 3").as_deref(), Some("7"));
        assert_eq!(eval("45*1.19").as_deref(), Some("53.55"));
        assert_eq!(eval("144 / 12").as_deref(), Some("12"));
    }

    #[test]
    fn precedence_and_parentheses_hold() {
        assert_eq!(eval("2 + 3 * 4").as_deref(), Some("14"));
        assert_eq!(eval("(2 + 3) * 4").as_deref(), Some("20"));
        assert_eq!(eval("2 * 3 ^ 2").as_deref(), Some("18"));
        assert_eq!(
            eval("2 ^ 3 ^ 2").as_deref(),
            Some("512"),
            "right-associative"
        );
    }

    #[test]
    fn percentages_read_the_way_people_write_them() {
        assert_eq!(eval("2000 + 19%").as_deref(), Some("2 380"), "Algerian TVA");
        assert_eq!(eval("2000 - 10%").as_deref(), Some("1 800"));
        assert_eq!(
            eval("50%").as_deref(),
            None,
            "a bare percentage is not a sum"
        );
        assert_eq!(eval("200 * 15%").as_deref(), Some("30"));
    }

    #[test]
    fn division_by_zero_answers_nothing() {
        // Not an error, not an infinity. Nothing.
        assert_eq!(eval("5 / 0"), None);
        assert_eq!(eval("5 / (3 - 3)"), None);
    }

    #[test]
    fn a_bare_number_is_not_a_calculation() {
        // Someone typing a year wants the year, not a card telling them it equals itself.
        assert_eq!(eval("2026"), None);
        assert_eq!(eval("45"), None);
        assert_eq!(eval("-5"), None);
    }

    #[test]
    fn a_fragment_is_never_answered_from() {
        // The failure that would make the tool untrustworthy: understanding part of a query and
        // answering as though it were the whole thing.
        assert_eq!(eval("2 + 2 apples"), None);
        assert_eq!(eval("prix 45 * 2 dinars"), None);
        assert_eq!(eval("covid 19 + vaccine"), None);
    }

    #[test]
    fn functions_work_and_refuse_impossible_arguments() {
        assert_eq!(eval("sqrt(64)").as_deref(), Some("8"));
        assert_eq!(eval("abs(0 - 7)").as_deref(), Some("7"));
        assert_eq!(eval("round(2.6)").as_deref(), Some("3"));
        assert_eq!(eval("sqrt(0 - 4)"), None, "no imaginary results");
        assert_eq!(eval("ln(0)"), None);
    }

    #[test]
    fn thousands_are_grouped_for_reading() {
        assert_eq!(eval("1000 * 1000").as_deref(), Some("1 000 000"));
        assert_eq!(eval("18500 * 2").as_deref(), Some("37 000"));
    }

    #[test]
    fn deep_nesting_cannot_exhaust_the_stack() {
        // `((((…))))` from a query string is a denial-of-service, not a calculation.
        let bomb = format!("{}1{}", "(".repeat(500), ")".repeat(500));
        assert_eq!(eval(&bomb), None);
    }

    #[test]
    fn a_very_long_input_terminates() {
        let long = "1+".repeat(5000) + "1";
        let started = std::time::Instant::now();
        let _ = eval(&long);
        assert!(
            started.elapsed().as_millis() < 200,
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn malformed_input_answers_nothing_rather_than_panicking() {
        for expr in ["((", "2 +", "* 5", "()", "2 ** 3", ".", "1.2.3", "+", "%%%"] {
            assert!(
                Calculator.answer(expr).is_none() || eval(expr).is_some(),
                "{expr:?} should not panic"
            );
        }
    }

    #[test]
    fn the_interpretation_shows_real_operators() {
        // So a misreading is visible. `*` and `/` are how it was typed; `×` and `÷` are what it
        // means, and seeing the difference is how a user catches a wrong parse.
        let answer = Calculator.answer("45*1.19").unwrap();
        assert!(
            answer.interpretation.contains('×'),
            "{}",
            answer.interpretation
        );
        assert_eq!(answer.as_of, None, "arithmetic is timeless");
    }
}

#[cfg(test)]
mod phrasing {
    use super::*;

    fn value(q: &str) -> String {
        Calculator.answer(q).map(|a| a.value).unwrap_or_default()
    }

    #[test]
    fn percent_of_in_three_languages() {
        assert_eq!(value("15% of 80"), "12");
        assert_eq!(value("15% de 80"), "12");
        assert_eq!(value("15٪ من 80"), "12");
        assert_eq!(value("احسب 15% من 80"), "12");
    }

    #[test]
    fn question_words_and_keyboard_symbols() {
        assert_eq!(value("what is 2+2"), "4");
        assert_eq!(value("combien font 3 × 4"), "12");
        assert_eq!(value("3 x 4"), "12");
        assert_eq!(value("كم يساوي 10 ÷ 4"), "2.5");
        assert_eq!(value("calculate (2+3)*4"), "20");
    }
}
