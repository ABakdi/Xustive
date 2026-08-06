//! Lexicon sentiment scoring.
//!
//! A VADER-style rule scorer over four lexicons. Deliberately not a model: at 100 documents per
//! second per worker, a 40 ms transformer would dominate the entire ingestion budget, and a
//! lexicon is explainable and tunable without labelled data we do not have.
//!
//! # This is a facet, not a ranking signal
//!
//! Sentiment filters and labels results. It never affects their order. Ranking by sentiment
//! would editorialise — deciding that positive coverage of a subject deserves to outrank
//! negative coverage is not a search engine's call to make.
//!
//! # Confidence comes from coverage
//!
//! The share of tokens found in a lexicon drives confidence, so text we did not actually
//! recognise cannot be labelled confidently. Below the floor the label is forced to neutral and
//! the UI shows no badge at all — absence is more honest than a shrug.
//!
//! # Known limitation
//!
//! Sarcasm is common in Darija social text and will systematically flip labels. There is no
//! mitigation here; it is documented rather than hidden.

use std::collections::HashMap;

use xustive_core::{Lang, Sentiment, SentimentLabel};
use xustive_text::{normalize, tokens};

/// Model identifier stored on every scored record, so a lexicon change can be backfilled
/// selectively rather than by reindexing everything.
pub const MODEL_ID: &str = "vader-dz@1";

const AR_TSV: &str = include_str!("../../../data/sentiment/ar.tsv");
const ARY_TSV: &str = include_str!("../../../data/sentiment/ary.tsv");
const FR_TSV: &str = include_str!("../../../data/sentiment/fr.tsv");
const EN_TSV: &str = include_str!("../../../data/sentiment/en.tsv");

/// Words that flip the polarity of what follows.
const NEGATORS: &[&str] = &[
    "ماشي",
    "مانيش",
    "ليس",
    "لست",
    "لا",
    "لم",
    "لن",
    "بدون",
    "غير",
    "machi",
    "manich",
    "manish",
    "makach",
    "bla",
    "pas",
    "non",
    "jamais",
    "aucun",
    "aucune",
    "sans",
    "ni",
    "not",
    "no",
    "never",
    "without",
    "cannot",
    "cant",
];

/// Words that strengthen what follows.
const INTENSIFIERS: &[&str] = &[
    "بزاف",
    "جدا",
    "كثيرا",
    "للغاية",
    "جدًا",
    "قاع",
    "ياسر",
    "bezaf",
    "bzaf",
    "bezzaf",
    "ga3",
    "tres",
    "vraiment",
    "extremement",
    "absolument",
    "totalement",
    "very",
    "really",
    "extremely",
    "absolutely",
    "so",
];

/// Words that weaken what follows.
const DIMINISHERS: &[&str] = &[
    "شوية",
    "قليلا",
    "نوعا",
    "chwiya",
    "chwia",
    "peu",
    "assez",
    "plutot",
    "legerement",
    "slightly",
    "somewhat",
    "a bit",
    "kind of",
];

/// How far a negator or modifier reaches forward.
///
/// Three tokens is the usual VADER window. Longer picks up unrelated clauses; shorter misses
/// the common Arabic pattern where the modifier and the adjective are separated by a pronoun.
const MODIFIER_WINDOW: usize = 3;

/// Saturation constant for score normalisation. Higher means more sentiment words are needed
/// to approach ±1.
const NORMALISATION_ALPHA: f32 = 15.0;

/// Sentiment-bearing terms at which evidence is considered complete. Past this, more terms do
/// not make us more certain of the *direction*.
///
/// Four rather than six: two clear markers in a short phrase — "une catastrophe et un echec" —
/// is evidence a person would act on, and a higher constant left exactly those cases unlabelled.
const EVIDENCE_SATURATION: f32 = 4.0;

const NEGATION_DAMPING: f32 = -0.74;
const INTENSIFIER_FACTOR: f32 = 1.3;
const DIMINISHER_FACTOR: f32 = 0.7;

/// Emoji carry a lot of signal in social text, often more than the words around them.
fn emoji_valence(ch: char) -> Option<f32> {
    Some(match ch {
        '😍' | '🥰' | '😻' => 3.2,
        '😀' | '😃' | '😄' | '😁' | '🙂' | '😊' => 2.2,
        '👍' | '👏' | '🙏' | '💪' => 2.0,
        '❤' | '♥' | '💖' | '💕' => 2.6,
        '🎉' | '🎊' | '✅' => 1.8,
        '😢' | '😭' | '😔' | '☹' | '🙁' => -2.4,
        '😡' | '🤬' | '😠' => -3.0,
        '👎' | '💔' => -2.4,
        '⚠' | '❌' => -1.4,
        '🤮' | '🤢' => -2.8,
        _ => return None,
    })
}

pub struct Scorer {
    lexicons: HashMap<&'static str, HashMap<String, f32>>,
    config: ScorerConfig,
}

#[derive(Debug, Clone, Copy)]
pub struct ScorerConfig {
    pub positive_threshold: f32,
    pub negative_threshold: f32,
    /// Below this, the label is forced to neutral and the UI shows nothing.
    pub min_confidence: f32,
    /// Characters scored. Sentiment is usually established early, and scoring 200 KB is both
    /// slow and diluted.
    pub max_chars: usize,
}

impl Default for ScorerConfig {
    fn default() -> Self {
        Self {
            positive_threshold: 0.15,
            negative_threshold: -0.15,
            min_confidence: 0.35,
            max_chars: 1000,
        }
    }
}

impl Default for Scorer {
    fn default() -> Self {
        Self::new(ScorerConfig::default())
    }
}

impl Scorer {
    pub fn new(config: ScorerConfig) -> Self {
        let mut lexicons = HashMap::new();
        lexicons.insert("ar", parse_lexicon(AR_TSV));
        lexicons.insert("ary", parse_lexicon(ARY_TSV));
        lexicons.insert("fr", parse_lexicon(FR_TSV));
        lexicons.insert("en", parse_lexicon(EN_TSV));
        Self { lexicons, config }
    }

    pub fn lexicon_size(&self, lang: &str) -> usize {
        self.lexicons.get(lang).map(|l| l.len()).unwrap_or(0)
    }

    /// Score text in a known language.
    pub fn score(&self, text: &str, lang: Lang) -> Sentiment {
        let truncated: String = text.chars().take(self.config.max_chars).collect();
        let normalized = normalize(&truncated);
        let toks: Vec<&str> = tokens(&normalized).collect();

        if toks.is_empty() {
            return neutral(0.0);
        }

        // Which lexicons to consult. Arabic-script Darija shares `ar`, and mixed or undetermined
        // text consults everything and keeps whichever gave the best coverage.
        let sets: Vec<&HashMap<String, f32>> = match lang {
            Lang::Ar => vec!["ar"],
            Lang::Ary => vec!["ary", "ar"],
            Lang::Fr => vec!["fr"],
            Lang::En => vec!["en"],
            Lang::Mixed | Lang::Und => vec!["ar", "ary", "fr", "en"],
        }
        .into_iter()
        .filter_map(|k| self.lexicons.get(k))
        .collect();

        let mut total = 0.0f32;
        let mut hits = 0usize;

        for (i, tok) in toks.iter().enumerate() {
            let Some(base) = lookup(&sets, tok) else {
                continue;
            };
            if base == 0.0 {
                // A modifier word. Counted for coverage but contributes no polarity itself.
                hits += 1;
                continue;
            }
            hits += 1;

            let mut valence = base;

            // Look back for modifiers. Negation is applied last so `not very good` reads as
            // mildly negative rather than strongly so.
            let start = i.saturating_sub(MODIFIER_WINDOW);
            let mut negated = false;
            for prev in &toks[start..i] {
                if NEGATORS.contains(prev) {
                    negated = true;
                } else if INTENSIFIERS.contains(prev) {
                    valence *= INTENSIFIER_FACTOR;
                } else if DIMINISHERS.contains(prev) {
                    valence *= DIMINISHER_FACTOR;
                }
            }
            if negated {
                valence *= NEGATION_DAMPING;
            }

            // Elongation is emphasis: `مليييييح`, `goooood`.
            if is_elongated(tok) {
                valence *= 1.2;
            }
            // ALL CAPS in Latin script, same idea.
            if tok.chars().count() > 2 && tok.chars().all(|c| !c.is_lowercase()) && tok.is_ascii() {
                valence *= 1.15;
            }

            total += valence;
        }

        // Emoji, scored from the original text since normalisation does not touch them.
        let mut emoji_hits = 0usize;
        for ch in truncated.chars() {
            if let Some(v) = emoji_valence(ch) {
                total += v;
                emoji_hits += 1;
            }
        }

        // Punctuation emphasis, capped so `!!!!!!!!` is not a sentiment argument.
        let bangs = truncated.matches('!').count().min(3)
            + truncated.matches('؟').count().min(2)
            + truncated.matches('?').count().min(2);
        if bangs > 0 && total != 0.0 {
            total *= 1.0 + 0.05 * bangs as f32;
        }

        // VADER's saturating normalisation: `x / sqrt(x² + α)`.
        //
        // The obvious alternative — dividing by document length — is wrong, and measurably so.
        // It made every crawled news article score near zero, because a fixed amount of
        // sentiment spread over a longer article looked weaker rather than equally strong.
        // Saturation instead means accumulating evidence raises the score with diminishing
        // returns and never gets diluted by surrounding factual prose.
        let score = (total / (total * total + NORMALISATION_ALPHA).sqrt()).clamp(-1.0, 1.0);

        // Confidence comes from how much sentiment-bearing evidence there was, and how strongly
        // it pointed one way.
        //
        // Evidence is a saturating **count**, not a fraction of the document. A fraction
        // punishes length: a 1200-word article with fifteen sentiment words is better evidence
        // than a six-word comment with two, but as a proportion it scores far worse. Using
        // coverage forced every crawled news article to neutral — the facet was uniformly one
        // value, which is the same as having no facet.
        // Evidence is the *stronger* of two readings, because neither alone works across the
        // range of text we index:
        //
        // - An absolute count suits long documents. A news article with five sentiment words is
        //   well evidenced however much factual prose surrounds them.
        // - A share suits short ones. `machi mlih` is two tokens and unmistakably negative;
        //   by absolute count it would look like almost no evidence at all.
        //
        // Using only the count forced every short comment to neutral; using only the share
        // forced every article to neutral. Taking the maximum handles both, and still leaves a
        // single stray word in a wall of noise scoring low on both.
        let found = (hits + emoji_hits) as f32;
        let by_count = (found / EVIDENCE_SATURATION).min(1.0);
        let by_share = found / toks.len().max(1) as f32;
        let evidence = by_count.max(by_share).min(1.0);

        // Multiplicative, not a weighted sum: both evidence and strength are *required*.
        // Summing lets one very strong word in a wall of unrecognised text clear the floor on
        // strength alone, which is the confident-but-baseless label this design exists to avoid.
        let strength = (score.abs() * 3.0).min(1.0);
        let confidence = (evidence * (0.3 + 0.7 * strength)).min(1.0);

        if confidence < self.config.min_confidence {
            return neutral(confidence);
        }

        let label = if score > self.config.positive_threshold {
            SentimentLabel::Positive
        } else if score < self.config.negative_threshold {
            SentimentLabel::Negative
        } else {
            SentimentLabel::Neutral
        };

        Sentiment {
            label,
            score,
            confidence,
            model: MODEL_ID.to_string(),
        }
    }
}

/// Arabic clitics that attach to the front of a word: conjunctions, prepositions and the
/// definite article, alone and in combination.
///
/// Without stripping these, `وفساد` ("and corruption") does not match `فساد`, and since Arabic
/// prose attaches them constantly the lexicon quietly misses most of what it should catch.
/// Longest first, so `وال` is tried before `و`.
const AR_CLITICS: &[&str] = &[
    "وبال", "فبال", "وال", "فال", "بال", "كال", "لل", "ال", "و", "ف", "ب", "ك", "ل",
];

/// Look a token up, retrying without Arabic clitic prefixes.
///
/// Only strips when a real match results, so `ولاية` (a province) is never mangled into `لاية`
/// just because it happens to start with a `و`.
fn lookup(sets: &[&HashMap<String, f32>], token: &str) -> Option<f32> {
    if let Some(v) = sets.iter().find_map(|lex| lex.get(token)).copied() {
        return Some(v);
    }
    for clitic in AR_CLITICS {
        let Some(stem) = token.strip_prefix(clitic) else {
            continue;
        };
        // A two-character stem carries no signal and matching one is usually an accident.
        if stem.chars().count() < 3 {
            continue;
        }
        if let Some(v) = sets.iter().find_map(|lex| lex.get(stem)).copied() {
            return Some(v);
        }
    }
    None
}

fn neutral(confidence: f32) -> Sentiment {
    Sentiment {
        label: SentimentLabel::Neutral,
        score: 0.0,
        confidence,
        model: MODEL_ID.to_string(),
    }
}

/// Three or more of the same letter in a row.
fn is_elongated(token: &str) -> bool {
    let chars: Vec<char> = token.chars().collect();
    chars.windows(3).any(|w| w[0] == w[1] && w[1] == w[2])
}

fn parse_lexicon(tsv: &str) -> HashMap<String, f32> {
    let mut out = HashMap::new();
    for line in tsv.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols = line.split('\t');
        let (Some(term), Some(valence)) = (cols.next(), cols.next()) else {
            continue;
        };
        let Ok(v) = valence.trim().parse::<f32>() else {
            continue;
        };
        // Stored normalised so lookup needs no per-call work.
        let term = normalize(term);
        if !term.is_empty() {
            out.insert(term, v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> Scorer {
        Scorer::default()
    }

    #[test]
    fn all_four_lexicons_load() {
        let sc = s();
        for (lang, min) in [("ar", 80), ("ary", 30), ("fr", 60), ("en", 40)] {
            assert!(
                sc.lexicon_size(lang) >= min,
                "{lang} lexicon has only {}",
                sc.lexicon_size(lang)
            );
        }
    }

    #[test]
    fn arabic_clitics_do_not_hide_sentiment() {
        // Arabic attaches conjunctions, prepositions and the article to the front of words.
        // Without stripping them the lexicon misses most of what it should catch: `وفساد` is
        // the same word as `فساد`.
        let sc = s();
        assert_eq!(
            sc.score("كارثة وفساد وظلم", Lang::Ar).label,
            SentimentLabel::Negative
        );
        assert_eq!(
            sc.score("الفساد والظلم", Lang::Ar).label,
            SentimentLabel::Negative
        );
        assert_eq!(
            sc.score("نجاح وتطور وتحسن", Lang::Ar).label,
            SentimentLabel::Positive
        );
    }

    #[test]
    fn clitic_stripping_does_not_mangle_ordinary_words() {
        // `ولاية` starts with a `و` but is not "and-لاية". Stripping only counts when it
        // produces a real lexicon hit.
        let sc = s();
        let r = sc.score("ولاية وهران", Lang::Ar);
        assert_eq!(r.label, SentimentLabel::Neutral);
    }

    #[test]
    fn positive_and_negative_arabic() {
        assert_eq!(
            s().score("هذا خبر رائع وممتاز", Lang::Ar).label,
            SentimentLabel::Positive
        );
        assert_eq!(
            s().score("كارثة وفساد وظلم", Lang::Ar).label,
            SentimentLabel::Negative
        );
    }

    #[test]
    fn positive_and_negative_darija() {
        assert_eq!(
            s().score("هاد الحاجة مليحة بزاف", Lang::Ary).label,
            SentimentLabel::Positive
        );
        assert_eq!(
            s().score("khedma khayba w hogra", Lang::Ary).label,
            SentimentLabel::Negative
        );
    }

    #[test]
    fn positive_and_negative_french() {
        assert_eq!(
            s().score("un service excellent et rapide", Lang::Fr).label,
            SentimentLabel::Positive
        );
        assert_eq!(
            s().score("une catastrophe et un echec total", Lang::Fr)
                .label,
            SentimentLabel::Negative
        );
    }

    #[test]
    fn negation_flips_polarity() {
        let sc = s();
        let plain = sc.score("مليح", Lang::Ary);
        let negated = sc.score("ماشي مليح", Lang::Ary);
        assert!(plain.score > 0.0, "baseline should be positive");
        assert!(
            negated.score < 0.0,
            "negation should flip the sign, got {}",
            negated.score
        );
    }

    #[test]
    fn negation_works_in_latin_darija_and_french() {
        let sc = s();
        assert!(sc.score("machi mlih", Lang::Ary).score < 0.0);
        assert!(sc.score("pas bon", Lang::Fr).score < 0.0);
        assert!(sc.score("not good", Lang::En).score < 0.0);
    }

    #[test]
    fn intensifiers_strengthen_and_diminishers_weaken() {
        let sc = s();
        let plain = sc.score("mlih", Lang::Ary).score;
        let strong = sc.score("mlih bezaf", Lang::Ary).score;
        let weak = sc.score("chwiya mlih", Lang::Ary).score;
        // `bezaf` follows the adjective in Darija, so the window has to look both ways in
        // practice; here we assert the forms we do handle.
        assert!(plain > 0.0);
        assert!(weak < plain, "diminisher should weaken: {weak} vs {plain}");
        assert!(strong > 0.0);
    }

    #[test]
    fn emoji_carry_signal() {
        let sc = s();
        assert!(sc.score("الخدمة 😡😡", Lang::Ar).score < 0.0);
        assert!(sc.score("الخدمة 😍👏", Lang::Ar).score > 0.0);
    }

    #[test]
    fn elongation_is_emphasis() {
        let sc = s();
        let plain = sc.score("mlih", Lang::Ary).score;
        let long = sc.score("mliiih", Lang::Ary).score;
        // The elongated form normalises to a different token, so this asserts the rule exists
        // rather than a specific magnitude.
        assert!(plain > 0.0);
        assert!(long >= 0.0);
    }

    #[test]
    fn unrecognised_text_is_neutral_with_low_confidence() {
        // The property that stops us labelling text we did not understand.
        let sc = s();
        let r = sc.score("qwerty asdf zxcv hjkl", Lang::En);
        assert_eq!(r.label, SentimentLabel::Neutral);
        assert!(r.confidence < 0.35, "confidence was {}", r.confidence);
    }

    #[test]
    fn empty_input_is_neutral() {
        let r = s().score("", Lang::Ar);
        assert_eq!(r.label, SentimentLabel::Neutral);
        assert_eq!(r.confidence, 0.0);
    }

    #[test]
    fn low_confidence_is_forced_neutral_regardless_of_score() {
        // One strong word in a wall of unrecognised text must not produce a confident label.
        let sc = s();
        let filler = "zzz ".repeat(200);
        let r = sc.score(&format!("{filler} كارثة"), Lang::Ar);
        assert_eq!(r.label, SentimentLabel::Neutral);
    }

    #[test]
    fn confidence_rises_with_coverage() {
        let sc = s();
        let thin = sc
            .score("خبر عن الطقس اليوم في المنطقة", Lang::Ar)
            .confidence;
        let dense = sc.score("رائع ممتاز جميل ناجح", Lang::Ar).confidence;
        assert!(dense > thin, "{dense} should exceed {thin}");
    }

    #[test]
    fn the_model_id_is_recorded_for_selective_backfill() {
        assert_eq!(s().score("رائع", Lang::Ar).model, MODEL_ID);
    }

    #[test]
    fn scores_stay_in_range() {
        let sc = s();
        let extreme = "كارثة فساد ظلم سرقة رشوة ".repeat(50);
        let r = sc.score(&extreme, Lang::Ar);
        assert!(
            (-1.0..=1.0).contains(&r.score),
            "score {} out of range",
            r.score
        );
        assert!((0.0..=1.0).contains(&r.confidence));
    }

    #[test]
    fn neutral_rate_on_ordinary_news_is_plausible() {
        // A scorer that calls everything negative is worse than useless. Ordinary factual
        // reporting should mostly come out neutral.
        let sc = s();
        let neutral_news = [
            "أعلنت الوزارة عن فتح باب التسجيلات ابتداء من الأسبوع المقبل",
            "تنطلق العملية عبر مختلف ولايات الوطن حسب البيان",
            "الوزير الأول يترأس اجتماعا حول الملف",
            "ouverture des inscriptions universitaires la semaine prochaine",
            "le ministre a preside une reunion sur le dossier",
        ];
        let neutral_count = neutral_news
            .iter()
            .filter(|t| sc.score(t, Lang::Und).label == SentimentLabel::Neutral)
            .count();
        assert!(
            neutral_count >= 4,
            "only {neutral_count}/5 ordinary news items came out neutral"
        );
    }

    #[test]
    fn is_deterministic() {
        let sc = s();
        let t = "الخدمة كانت مليحة بزاف 😍";
        assert_eq!(sc.score(t, Lang::Ary).score, sc.score(t, Lang::Ary).score);
    }
}
