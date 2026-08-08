//! Small utilities.
//!
//! Each is a few dozen lines and each is a query that would otherwise leave for a site covered in
//! advertising that logs what you encoded. That is the whole argument for having them: not that
//! they are impressive, but that sending someone elsewhere to Base64 a string means sending them
//! somewhere that keeps the string.
//!
//! All are pure, offline and deterministic — no network, no clock, nothing to go stale.
//!
//! # What is deliberately absent
//!
//! **A password generator.** Generating a secret in a search box trains exactly the wrong
//! instinct, and the box is the one field on the page most likely to end up in somebody's
//! history.
//!
//! **Dice and coin flips.** They need randomness, which makes the answer unreproducible — a card
//! that shows a different number every time the page reloads reads as a bug, and one that shows
//! the same number every time is not random. Neither is worth having.
//!
//! **QR codes.** Worth building; it needs an encoder and an SVG renderer, which is more than a
//! few dozen lines and is not this.

use rust_decimal::prelude::*;
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};

use crate::{fold_digits, Answer, Tool};

pub struct Utilities;

/// Algerian VAT. 19 % standard, 9 % reduced.
const TVA_STANDARD: &str = "0.19";
const TVA_REDUCED: &str = "0.09";

impl Tool for Utilities {
    fn name(&self) -> &'static str {
        "utility"
    }

    fn keyword(&self) -> &'static str {
        "util"
    }

    fn answer(&self, query: &str) -> Option<Answer> {
        let q = query.trim();
        // Ordered by how specific the trigger is. `base64` cannot be mistaken for anything;
        // `count` could be part of a sentence, so it comes later and demands more.
        base64(q)
            .or_else(|| url_codec(q))
            .or_else(|| vat(q))
            .or_else(|| percentage_change(q))
            .or_else(|| roman(q))
            .or_else(|| hash(q))
            .or_else(|| json_format(q))
            .or_else(|| tip(q))
            .or_else(|| loan(q))
            .or_else(|| bmi(q))
            .or_else(|| colour(q))
            .or_else(|| case_convert(q))
            .or_else(|| count(q))
    }
}

fn answer(kind: &'static str, interpretation: String, value: String) -> Option<Answer> {
    Some(Answer {
        tool: "utility",
        // Keyed matches: an explicit verb plus an operand. High, but below the calculator's,
        // because a trigger word is weaker evidence than a string that parses as arithmetic.
        confidence: 0.9,
        interpretation,
        value,
        detail: Some(serde_json::json!({ "kind": kind })),
        as_of: None,
    })
}

/// Match `<verb> encode <rest>` / `<verb> decode <rest>`.
///
/// Returns whether to encode, and the operand **from the original string** — matching is done on a
/// lowercased copy, but the operand must keep its case, since `base64 encode Hello` and
/// `base64 encode hello` are different bytes.
fn codec_directive<'a>(q: &'a str, lower: &str, verb: &str) -> Option<(bool, &'a str)> {
    for (suffix, encode) in [("encode ", true), ("decode ", false)] {
        let prefix = format!("{verb} {suffix}");
        let Some(r) = lower.strip_prefix(&prefix) else {
            continue;
        };
        let rest = q[q.len() - r.len()..].trim();
        if !rest.is_empty() {
            return Some((encode, rest));
        }
    }
    None
}

/// `base64 encode hello` / `base64 decode aGVsbG8=`
fn base64(q: &str) -> Option<Answer> {
    let (encode, rest) = codec_directive(q, &q.to_lowercase(), "base64")?;

    if encode {
        answer(
            "base64-encode",
            format!("base64 ← {}", clip(rest)),
            b64_encode(rest.as_bytes()),
        )
    } else {
        // Invalid Base64 answers nothing rather than showing replacement characters. A decode
        // that silently produces mojibake looks like it worked.
        let bytes = b64_decode(rest)?;
        let text = String::from_utf8(bytes).ok()?;
        answer("base64-decode", format!("base64 → {}", clip(rest)), text)
    }
}

const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - i * 6) & 0x3F) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn b64_decode(input: &str) -> Option<Vec<u8>> {
    let cleaned: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
        .collect();
    let mut out = Vec::with_capacity(cleaned.len() * 3 / 4);
    for chunk in cleaned.chunks(4) {
        let mut n = 0u32;
        for (i, byte) in chunk.iter().enumerate() {
            let value = B64.iter().position(|c| c == byte)? as u32;
            n |= value << (18 - i * 6);
        }
        // A four-character group carries three bytes, a three-character group two, and so on.
        for i in 0..chunk.len().saturating_sub(1) {
            out.push((n >> (16 - i * 8)) as u8);
        }
    }
    Some(out)
}

/// `url encode …` / `url decode …`
fn url_codec(q: &str) -> Option<Answer> {
    let (encode, rest) = codec_directive(q, &q.to_lowercase(), "url")?;

    if encode {
        let out: String = rest
            .bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect();
        answer("url-encode", format!("url ← {}", clip(rest)), out)
    } else {
        let mut bytes = Vec::with_capacity(rest.len());
        let raw = rest.as_bytes();
        let mut i = 0;
        while i < raw.len() {
            if raw[i] == b'%' && i + 2 < raw.len() {
                let hex = std::str::from_utf8(&raw[i + 1..i + 3]).ok()?;
                bytes.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            } else {
                bytes.push(if raw[i] == b'+' { b' ' } else { raw[i] });
                i += 1;
            }
        }
        // Percent-decoded bytes that are not valid UTF-8 answer nothing. Showing replacement
        // characters would look like a successful decode of a corrupted string.
        answer(
            "url-decode",
            format!("url → {}", clip(rest)),
            String::from_utf8(bytes).ok()?,
        )
    }
}

/// Algerian VAT: `tva 2000`, `الرسم على القيمة المضافة 2000`, `vat 2000 9%`
fn vat(q: &str) -> Option<Answer> {
    let folded = fold_digits(q).to_lowercase();
    const TRIGGERS: &[&str] = &["tva", "vat", "الرسم على القيمة المضافة", "القيمة المضافة"];
    if !TRIGGERS.iter().any(|t| folded.contains(t)) {
        return None;
    }

    let numbers = numbers_in(&folded);

    // The amount is the largest number present — a rate written alongside it is 9 or 19, and
    // reading the rate as the amount would answer a question nobody asked.
    let amount = numbers.iter().copied().max()?;
    let reduced =
        folded.contains("9%") || folded.contains(" 9 ") || numbers.contains(&Decimal::from(9));
    let rate = Decimal::from_str(if reduced { TVA_REDUCED } else { TVA_STANDARD }).ok()?;

    let tax = (amount * rate).round_dp(2);
    let total = (amount + tax).round_dp(2);
    let percent = if reduced { "9" } else { "19" };

    answer(
        "vat",
        format!("TVA {percent}% · {amount}"),
        format!("{} (TVA {})", trim(total), trim(tax)),
    )
}

/// `percentage change from 120 to 150`, `de 120 à 150 en pourcentage`
fn percentage_change(q: &str) -> Option<Answer> {
    let folded = fold_digits(q).to_lowercase();
    const TRIGGERS: &[&str] = &[
        "percentage change",
        "percent change",
        "pourcentage",
        "نسبة التغير",
    ];
    if !TRIGGERS.iter().any(|t| folded.contains(t)) {
        return None;
    }

    let numbers = numbers_in(&folded);
    if numbers.len() < 2 {
        return None;
    }
    let (from, to) = (numbers[0], numbers[1]);
    if from.is_zero() {
        // Percentage change from zero is undefined, not infinite.
        return None;
    }
    let change = ((to - from) / from * Decimal::ONE_HUNDRED).round_dp(2);
    let sign = if change.is_sign_positive() { "+" } else { "" };
    answer(
        "percentage-change",
        format!("{} → {}", trim(from), trim(to)),
        format!("{sign}{}%", trim(change)),
    )
}

/// `roman 2026`, `roman MMXXVI`
fn roman(q: &str) -> Option<Answer> {
    let rest = q.to_lowercase().strip_prefix("roman ")?.trim().to_string();
    if rest.is_empty() {
        return None;
    }

    const PAIRS: &[(u32, &str)] = &[
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];

    if let Ok(mut n) = rest.parse::<u32>() {
        // Roman numerals have no zero and no standard form above 3999.
        if n == 0 || n > 3999 {
            return None;
        }
        let original = n;
        let mut out = String::new();
        for (value, symbol) in PAIRS {
            while n >= *value {
                out.push_str(symbol);
                n -= value;
            }
        }
        return answer("roman", format!("{original}"), out);
    }

    let upper = rest.to_uppercase();
    if !upper.chars().all(|c| "IVXLCDM".contains(c)) {
        return None;
    }
    let mut total = 0u32;
    let chars: Vec<char> = upper.chars().collect();
    let value = |c: char| match c {
        'I' => 1,
        'V' => 5,
        'X' => 10,
        'L' => 50,
        'C' => 100,
        'D' => 500,
        'M' => 1000,
        _ => 0,
    };
    for (i, c) in chars.iter().enumerate() {
        let v = value(*c);
        // Subtractive notation: a smaller symbol before a larger one is subtracted.
        if chars.get(i + 1).is_some_and(|next| value(*next) > v) {
            total = total.checked_sub(v)?;
        } else {
            total += v;
        }
    }
    (total > 0).then(|| answer("roman", upper.clone(), total.to_string()))?
}

/// `#1a7f4a to rgb`, `rgb(26,127,74) to hex`
fn colour(q: &str) -> Option<Answer> {
    let lower = q.to_lowercase();
    let hex = lower
        .split_whitespace()
        .find(|t| t.starts_with('#') && (t.len() == 4 || t.len() == 7))?;
    let digits = &hex[1..];
    let expand = |s: &str| -> Option<(u8, u8, u8)> {
        let parse = |t: &str| u8::from_str_radix(t, 16).ok();
        match s.len() {
            // #abc is shorthand for #aabbcc.
            3 => {
                let c: Vec<char> = s.chars().collect();
                Some((
                    parse(&format!("{}{}", c[0], c[0]))?,
                    parse(&format!("{}{}", c[1], c[1]))?,
                    parse(&format!("{}{}", c[2], c[2]))?,
                ))
            }
            6 => Some((parse(&s[0..2])?, parse(&s[2..4])?, parse(&s[4..6])?)),
            _ => None,
        }
    };
    let (r, g, b) = expand(digits)?;
    answer(
        "colour",
        format!("#{}", digits.to_uppercase()),
        format!("rgb({r}, {g}, {b})"),
    )
}

/// `uppercase …`, `lowercase …`, `title case …`
fn case_convert(q: &str) -> Option<Answer> {
    let lower = q.to_lowercase();
    for (prefix, kind) in [
        ("uppercase ", "upper"),
        ("lowercase ", "lower"),
        ("title case ", "title"),
        ("majuscule ", "upper"),
        ("minuscule ", "lower"),
    ] {
        let Some(r) = lower.strip_prefix(prefix) else {
            continue;
        };
        let rest = q[q.len() - r.len()..].trim();
        if rest.is_empty() {
            continue;
        }
        let out = match kind {
            "upper" => rest.to_uppercase(),
            "lower" => rest.to_lowercase(),
            _ => rest
                .split_whitespace()
                .map(|w| {
                    let mut chars = w.chars();
                    match chars.next() {
                        Some(first) => {
                            first.to_uppercase().collect::<String>()
                                + &chars.as_str().to_lowercase()
                        }
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" "),
        };
        return answer("case", format!("{kind} · {}", clip(rest)), out);
    }
    None
}

/// `word count …`, `عدد الكلمات …`
fn count(q: &str) -> Option<Answer> {
    let lower = q.to_lowercase();
    for prefix in [
        "word count ",
        "character count ",
        "عدد الكلمات ",
        "nombre de mots ",
    ] {
        let Some(r) = lower.strip_prefix(prefix) else {
            continue;
        };
        let rest = q[q.len() - r.len()..].trim();
        if rest.is_empty() {
            continue;
        }
        let words = rest.split_whitespace().count();
        // Characters counted as Unicode scalars, not bytes. Arabic is two bytes per character
        // and a byte count would report double for exactly this audience.
        let chars = rest.chars().count();
        return answer(
            "count",
            clip(rest),
            format!("{words} words · {chars} characters"),
        );
    }
    None
}

/// `sha256 hello`
///
/// SHA-256 only. **MD5 and SHA-1 are deliberately absent**: someone reaching a search box for a
/// hash is as likely to be hashing something that matters as verifying a download, and offering a
/// broken function under a neutral label is an invitation to use it for the first case.
fn hash(q: &str) -> Option<Answer> {
    let lower = q.to_lowercase();
    let rest = ["sha256 ", "sha-256 ", "hash "]
        .iter()
        .find_map(|p| lower.strip_prefix(p))
        .map(|r| q[q.len() - r.len()..].trim())?;
    if rest.is_empty() {
        return None;
    }
    let digest = Sha256::digest(rest.as_bytes());
    answer(
        "hash",
        format!("SHA-256 · {}", clip(rest)),
        digest.iter().map(|b| format!("{b:02x}")).collect(),
    )
}

/// `json format {"a":1}`
fn json_format(q: &str) -> Option<Answer> {
    let lower = q.to_lowercase();
    let rest = ["json format ", "format json ", "json "]
        .iter()
        .find_map(|p| lower.strip_prefix(p))
        .map(|r| q[q.len() - r.len()..].trim())?;
    // Must actually start like JSON. `json parsing in rust` is a search, not a format request.
    if !rest.starts_with('{') && !rest.starts_with('[') {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_str(rest).ok()?;
    let pretty = serde_json::to_string_pretty(&parsed).ok()?;
    // A 4 KB block pushed above the results is not an instant answer.
    if pretty.len() > 2_000 {
        return None;
    }
    answer("json", format!("JSON · {} bytes", rest.len()), pretty)
}

/// `tip 2400 15% 4 people`
fn tip(q: &str) -> Option<Answer> {
    let folded = fold_digits(q).to_lowercase();
    if !["tip ", "pourboire", "بقشيش"]
        .iter()
        .any(|t| folded.contains(t))
    {
        return None;
    }
    let numbers = numbers_in(&folded);
    let bill = *numbers.first()?;
    if bill <= Decimal::ZERO {
        return None;
    }
    // Both the rate and the head count are small integers, so position alone cannot tell them
    // apart — picking the first plausible number for each made `tip 2400 15% 4 people` split the
    // bill fifteen ways. Each is identified by the token attached to it instead.
    let rate = marked_number(&folded, &["%"]).unwrap_or(Decimal::TEN);
    let people = marked_number(&folded, &["people", "person", "personne", "أشخاص", "شخص"])
        .filter(|n| n.fract().is_zero() && *n >= Decimal::from(2) && *n <= Decimal::from(50))
        .unwrap_or(Decimal::ONE);

    let gratuity = (bill * rate / Decimal::ONE_HUNDRED).round_dp(2);
    let total = bill + gratuity;
    let each = (total / people).round_dp(2);
    let value = if people > Decimal::ONE {
        format!("{} ({} each)", trim(total), trim(each))
    } else {
        trim(total)
    };
    answer("tip", format!("{} + {}%", trim(bill), trim(rate)), value)
}

/// `loan 2000000 5% 15 years`
///
/// Standard amortisation. Presented as arithmetic, not advice — it says what a payment would be
/// at a rate you supplied, and nothing about whether to borrow.
fn loan(q: &str) -> Option<Answer> {
    let folded = fold_digits(q).to_lowercase();
    if !["loan", "credit ", "crédit", "قرض", "mortgage"]
        .iter()
        .any(|t| folded.contains(t))
    {
        return None;
    }
    let numbers = numbers_in(&folded);
    if numbers.len() < 3 {
        return None;
    }
    let principal = numbers.iter().copied().max()?;
    let rest: Vec<Decimal> = numbers
        .iter()
        .copied()
        .filter(|n| *n != principal)
        .collect();
    let annual_rate = *rest.first()?;
    let years = *rest.get(1)?;
    if principal <= Decimal::ZERO || years <= Decimal::ZERO || annual_rate < Decimal::ZERO {
        return None;
    }

    let months = (years * Decimal::from(12)).round();
    let months_f = months.to_f64()?;
    let monthly_rate = (annual_rate / Decimal::ONE_HUNDRED / Decimal::from(12)).to_f64()?;
    let principal_f = principal.to_f64()?;

    // A zero-interest loan divides by zero in the amortisation formula; it is simply the
    // principal spread evenly.
    let payment = if monthly_rate == 0.0 {
        principal_f / months_f
    } else {
        let growth = (1.0 + monthly_rate).powf(months_f);
        principal_f * monthly_rate * growth / (growth - 1.0)
    };
    if !payment.is_finite() || payment <= 0.0 {
        return None;
    }
    let payment = Decimal::from_f64(payment)?.round_dp(2);
    let total = (payment * months).round_dp(2);
    answer(
        "loan",
        format!(
            "{} · {}% · {} years",
            trim(principal),
            trim(annual_rate),
            trim(years)
        ),
        format!("{}/month · {} total", trim(payment), trim(total)),
    )
}

/// `bmi 78 1.80`
fn bmi(q: &str) -> Option<Answer> {
    let folded = fold_digits(q).to_lowercase();
    if !["bmi", "imc", "كتلة الجسم"]
        .iter()
        .any(|t| folded.contains(t))
    {
        return None;
    }
    let numbers = numbers_in(&folded);
    if numbers.len() < 2 {
        return None;
    }
    let (weight, height) = (numbers[0], numbers[1]);
    // Height in centimetres is the common way to write it; metres is what the formula needs.
    let height_m = if height > Decimal::from(3) {
        height / Decimal::ONE_HUNDRED
    } else {
        height
    };
    if height_m <= Decimal::ZERO || weight <= Decimal::ZERO {
        return None;
    }
    let value = (weight / (height_m * height_m)).round_dp(1);
    // Bounds, not a diagnosis. A number outside human range means the input was misread, and the
    // card must not present it as a result.
    if value < Decimal::from(5) || value > Decimal::from(200) {
        return None;
    }
    // No category label, no advice. The spec excludes medical calculators because wrong output
    // causes harm; a number with a band attached reads as a judgement, and this is arithmetic.
    answer(
        "bmi",
        format!("{} kg · {} m", trim(weight), trim(height_m)),
        trim(value),
    )
}

/// The number attached to one of `markers`.
///
/// `%` follows its number (`15%`), words follow it too (`4 people`), so in both cases the number
/// wanted is the last one appearing before the marker. Returns `None` when no marker is present,
/// which is what lets a default apply.
fn marked_number(haystack: &str, markers: &[&str]) -> Option<Decimal> {
    markers
        .iter()
        .filter_map(|m| haystack.find(m))
        .min()
        .and_then(|at| numbers_in(&haystack[..at]).pop())
}

/// Every decimal appearing in a string, in order.
fn numbers_in(s: &str) -> Vec<Decimal> {
    s.split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .filter(|t| !t.is_empty())
        .filter_map(|t| Decimal::from_str(t).ok())
        .collect()
}

/// Strip trailing zeros left by `round_dp`.
///
/// `round_dp(2)` renders 25 as `25.00`, which reads as spurious precision on a percentage and as
/// clutter on a round total. Genuine cents survive — 1234.56 is unchanged.
fn trim(d: Decimal) -> String {
    d.normalize().to_string()
}

/// Shorten a string for the interpretation line.
fn clip(s: &str) -> String {
    let cut: String = s.chars().take(40).collect();
    if s.chars().count() > 40 {
        format!("{cut}…")
    } else {
        cut
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(q: &str) -> Option<String> {
        Utilities.answer(q).map(|a| a.value)
    }

    #[test]
    fn base64_round_trips_including_arabic() {
        // Arabic is multi-byte throughout, which is where a naive implementation splits a
        // character in half and produces something that decodes to mojibake.
        for text in ["hello", "الجزائر", "café", "a", "ab", "abc", "abcd"] {
            let encoded = value(&format!("base64 encode {text}")).expect("encode");
            let decoded = value(&format!("base64 decode {encoded}")).expect("decode");
            assert_eq!(decoded, text, "round trip failed for {text:?}");
        }
    }

    #[test]
    fn base64_matches_the_canonical_encoding() {
        assert_eq!(value("base64 encode hello").as_deref(), Some("aGVsbG8="));
        assert_eq!(value("base64 encode hi").as_deref(), Some("aGk="));
        assert_eq!(value("base64 decode aGVsbG8=").as_deref(), Some("hello"));
    }

    #[test]
    fn invalid_base64_answers_nothing() {
        // A decode that silently produces mojibake looks like it worked.
        assert!(value("base64 decode not-valid-base64!!").is_none());
        assert!(value("base64 encode ").is_none());
    }

    #[test]
    fn url_encoding_round_trips_arabic() {
        let encoded = value("url encode الجزائر").expect("encode");
        assert!(encoded.starts_with("%D8"), "got {encoded}");
        assert_eq!(
            value(&format!("url decode {encoded}")).as_deref(),
            Some("الجزائر")
        );
    }

    #[test]
    fn algerian_vat_uses_the_right_rates() {
        // 19% standard, 9% reduced. Getting these wrong is a wrong number on an invoice.
        assert_eq!(value("tva 2000").as_deref(), Some("2380 (TVA 380)"));
        assert_eq!(value("vat 1000").as_deref(), Some("1190 (TVA 190)"));
        assert_eq!(value("tva 1000 9%").as_deref(), Some("1090 (TVA 90)"));
    }

    #[test]
    fn percentage_change_handles_both_directions() {
        assert_eq!(value("percentage change 120 150").as_deref(), Some("+25%"));
        assert_eq!(value("percentage change 150 120").as_deref(), Some("-20%"));
        // From zero is undefined, not infinite.
        assert!(value("percentage change 0 150").is_none());
    }

    #[test]
    fn roman_numerals_convert_both_ways() {
        assert_eq!(value("roman 2026").as_deref(), Some("MMXXVI"));
        assert_eq!(value("roman 1962").as_deref(), Some("MCMLXII"));
        assert_eq!(value("roman MMXXVI").as_deref(), Some("2026"));
        // Subtractive notation, the part a naive implementation gets wrong.
        assert_eq!(value("roman MCMXCIV").as_deref(), Some("1994"));
    }

    #[test]
    fn roman_refuses_what_it_cannot_represent() {
        // No zero, and no standard form above 3999.
        assert!(value("roman 0").is_none());
        assert!(value("roman 4000").is_none());
        assert!(value("roman hello").is_none());
    }

    #[test]
    fn hex_colours_expand_shorthand() {
        assert_eq!(value("#1a7f4a to rgb").as_deref(), Some("rgb(26, 127, 74)"));
        assert_eq!(value("#abc to rgb").as_deref(), Some("rgb(170, 187, 204)"));
    }

    #[test]
    fn case_conversion_preserves_arabic() {
        // Arabic has no case. The operation must be a no-op rather than mangling the text.
        assert_eq!(value("uppercase الجزائر").as_deref(), Some("الجزائر"));
        assert_eq!(
            value("uppercase hello world").as_deref(),
            Some("HELLO WORLD")
        );
        assert_eq!(
            value("title case hello world").as_deref(),
            Some("Hello World")
        );
    }

    #[test]
    fn counting_uses_characters_not_bytes() {
        // Arabic is two bytes per character; a byte count would report double for exactly the
        // audience this is built for.
        let out = value("word count الجزائر بلدي").expect("count");
        assert!(out.contains("2 words"), "got {out}");
        assert!(out.contains("12 characters"), "got {out}");
    }

    #[test]
    fn sha256_matches_the_published_vector() {
        // The canonical test vector. A hash tool that is subtly wrong is worse than none, because
        // its output looks exactly like a correct one.
        assert_eq!(
            value("sha256 abc").as_deref(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        assert_eq!(value("sha256 abc").map(|h| h.len()), Some(64));
    }

    #[test]
    fn json_is_formatted_but_only_when_it_is_json() {
        let out = value(r#"json format {"a":1,"b":[2,3]}"#).expect("format");
        assert!(out.contains("\n  \"a\": 1"), "not pretty-printed: {out}");
        // `json parsing in rust` is a search, not a format request.
        assert!(value("json parsing in rust").is_none());
        assert!(value("json format {broken").is_none());
    }

    #[test]
    fn tip_splits_between_people() {
        // 2400 + 10% default = 2640.
        assert_eq!(value("tip 2400").as_deref(), Some("2640"));
        // The rate and the head count are both small integers, so they must be told apart by the
        // token attached to them: this read 15 as the head count and split the bill fifteen ways.
        assert_eq!(
            value("tip 2400 15% 4 people").as_deref(),
            Some("2760 (690 each)")
        );
        // Marker order does not matter, and an unsplit bill shows no per-person figure.
        assert_eq!(value("tip 2400 15%").as_deref(), Some("2760"));
    }

    #[test]
    fn loan_amortisation_matches_the_standard_formula() {
        // 1 000 000 DZD at 5% over 10 years is 10 606.55/month by the standard formula.
        let out = value("loan 1000000 5% 10 years").expect("loan");
        assert!(out.starts_with("10606.5"), "got {out}");
    }

    #[test]
    fn a_zero_interest_loan_does_not_divide_by_zero() {
        // The amortisation formula has (growth - 1) in its denominator, which is zero at 0%.
        let out = value("loan 120000 0% 10 years").expect("loan");
        assert!(out.starts_with("1000/month"), "got {out}");
    }

    #[test]
    fn bmi_accepts_height_in_metres_or_centimetres() {
        assert_eq!(value("bmi 78 1.80").as_deref(), Some("24.1"));
        assert_eq!(value("bmi 78 180").as_deref(), Some("24.1"));
    }

    #[test]
    fn bmi_offers_no_category_and_no_advice() {
        // The spec excludes medical calculators because wrong output causes harm. A bare number
        // is arithmetic; a band attached to it reads as a judgement.
        let answer = Utilities.answer("bmi 78 1.80").expect("answer");
        let lower = answer.value.to_lowercase();
        for word in ["normal", "obese", "overweight", "healthy", "under"] {
            assert!(
                !lower.contains(word),
                "value carries a judgement: {}",
                answer.value
            );
        }
    }

    #[test]
    fn implausible_bmi_input_is_refused() {
        // Outside human range means the input was misread, not that the person is unusual.
        assert!(value("bmi 78 0.1").is_none());
        assert!(value("bmi 78").is_none());
    }

    #[test]
    fn broken_hash_functions_are_not_offered() {
        // Someone reaching a search box for a hash may be hashing something that matters, and a
        // broken function under a neutral label invites exactly that.
        assert!(value("md5 abc").is_none());
        assert!(value("sha1 abc").is_none());
    }

    #[test]
    fn ordinary_queries_match_no_utility() {
        // The precision that decides whether tools feel helpful or intrusive.
        for q in [
            "الجزائر",
            "prix du gaz",
            "roman empire history",
            "url shortener",
            "base64 explained",
            "count of monte cristo",
            "tva definition",
            "json parsing in rust",
            "how to hash a password",
            "loan calculator explained",
            "bmi meaning",
        ] {
            assert!(
                Utilities.answer(q).is_none(),
                "{q:?} should not activate a utility, got {:?}",
                Utilities.answer(q).map(|a| a.value)
            );
        }
    }

    #[test]
    fn every_utility_is_deterministic() {
        // No clock, no randomness. A card that shows a different answer on reload reads as a bug.
        for q in [
            "base64 encode hello",
            "roman 2026",
            "tva 2000",
            "#abc to rgb",
        ] {
            assert_eq!(value(q), value(q), "{q:?} is not deterministic");
        }
    }

    #[test]
    fn malformed_input_does_not_panic() {
        for q in [
            "base64 encode",
            "roman",
            "url decode %",
            "url decode %ZZ",
            "#",
            "tva",
        ] {
            let _ = Utilities.answer(q);
        }
    }
}
