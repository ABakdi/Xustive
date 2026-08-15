//! Language detection.
//!
//! The hard problem is not French versus English. It is **Darija**, which appears in Arabic
//! script, in Latin script (Arabizi), and code-switched with French mid-sentence. Off-the-shelf
//! detectors call all of these `ar` or `fr` and lose the distinction that matters most here.
//!
//! # The cascade
//!
//! 1. **Script** — Unicode block counting. Microseconds, and it settles most of the question.
//! 2. **Statistical** — `whatlang` trigrams, used only where script leaves genuine ambiguity
//!    (French vs English on Latin text).
//! 3. **Lexicon** — Darija and Arabizi marker lists. This is the layer that produces `ary` at
//!    all, and it is the only one specific to Algeria.
//!
//! # `Und` is a good answer
//!
//! A wrong confident answer is much worse than admitting ignorance. Narrowing to `fr` on a
//! Darija query returns nothing; `Und` returns slightly noisier but non-empty results. Short
//! queries — which is most of them — are scaled toward `Und` deliberately.

use xustive_core::Lang;
use xustive_text::script::{self, Script};
use xustive_text::{normalize, tokens};

use crate::lexicon::{self, Lexicon};

/// A detection result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Detection {
    pub lang: Lang,
    /// 0.0 ..= 1.0. Already adjusted for input length.
    pub confidence: f32,
    pub script: Script,
    /// Present when the text is code-switched.
    pub secondary: Option<(Lang, f32)>,
}

impl Detection {
    fn und(script: Script) -> Self {
        Self {
            lang: Lang::Und,
            confidence: 0.0,
            script,
            secondary: None,
        }
    }

    /// Whether the caller should narrow retrieval to this language.
    pub fn is_actionable(&self) -> bool {
        self.lang != Lang::Und && self.confidence >= 0.5
    }
}

/// Tunables. Defaults are deliberately conservative toward `Und`.
#[derive(Debug, Clone, Copy)]
pub struct DetectorConfig {
    /// Below this, the answer becomes `Und`.
    pub min_confidence: f32,
    /// Token count at which length no longer discounts confidence.
    pub full_confidence_tokens: usize,
    /// Fraction of one script needed to call the text as that script.
    pub script_dominance: f32,
    /// Secondary-language share needed to report `Mixed`.
    pub mixed_secondary_min: f32,
    /// Truncation before detection, to bound work on hostile input.
    pub max_chars: usize,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            min_confidence: 0.55,
            full_confidence_tokens: 5,
            script_dominance: script::DEFAULT_DOMINANCE,
            mixed_secondary_min: 0.30,
            max_chars: 4096,
        }
    }
}

/// Where a verdict came from. Determines whether the length discount applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Evidence {
    /// Marker words matched. Reliable on short input.
    Lexicon,
    /// Trigram statistics. Unreliable on short input.
    Statistical,
}

/// An un-finalised detection, before the confidence floor is applied.
#[derive(Debug, Clone, Copy)]
struct Verdict {
    lang: Lang,
    confidence: f32,
    secondary: Option<(Lang, f32)>,
    source: Evidence,
}

impl Verdict {
    fn lexicon(lang: Lang, confidence: f32) -> Self {
        Self {
            lang,
            confidence,
            secondary: None,
            source: Evidence::Lexicon,
        }
    }
    fn with_secondary(mut self, secondary: Option<(Lang, f32)>) -> Self {
        self.secondary = secondary;
        self
    }
}

/// The detector. Build once and share; it is immutable and `Sync`.
pub struct Detector {
    darija_ar: Lexicon,
    arabizi: Lexicon,
    french: Lexicon,
    english: Lexicon,
    config: DetectorConfig,
}

impl Default for Detector {
    fn default() -> Self {
        Self::new(DetectorConfig::default())
    }
}

impl Detector {
    pub fn new(config: DetectorConfig) -> Self {
        Self {
            darija_ar: lexicon::darija_arabic(),
            arabizi: lexicon::arabizi(),
            french: lexicon::french_common(),
            english: lexicon::english_common(),
            config,
        }
    }

    pub fn config(&self) -> &DetectorConfig {
        &self.config
    }

    /// Detect the language of already-normalised text.
    ///
    /// Callers that have not normalised should use [`Detector::detect`], which does it for them.
    pub fn detect_normalized(&self, normalized: &str) -> Detection {
        let text: String = normalized.chars().take(self.config.max_chars).collect();
        let token_count = tokens(&text).count();
        if token_count == 0 {
            return Detection::und(Script::Unknown);
        }

        let ratio = script::ratio(&text);
        let scr = script::detect_with(&text, self.config.script_dominance);

        let raw = match scr {
            Script::Arabic => self.detect_arabic_script(&text),
            Script::Latin => self.detect_latin_script(&text),
            Script::Mixed => self.detect_mixed(&text, ratio.arabic_fraction()),
            // Digits, punctuation or emoji only. Nothing to go on.
            Script::Unknown => return Detection::und(Script::Unknown),
        };

        self.finish(raw, scr, token_count)
    }

    /// Normalise, then detect.
    pub fn detect(&self, text: &str) -> Detection {
        self.detect_normalized(&normalize(text))
    }

    // --- per-script branches --------------------------------------------------------

    /// Arabic script: the question is Darija or MSA.
    fn detect_arabic_script(&self, text: &str) -> Verdict {
        let d = self.darija_ar.score(text);
        if d.is_evidence() {
            // More markers, and a higher share of the text being markers, both raise confidence.
            let conf = 0.6 + (d.weight * 0.12).min(0.3) + (d.coverage() * 0.2).min(0.1);
            return Verdict::lexicon(Lang::Ary, conf.min(0.97));
        }
        // No Darija evidence. Arabic script with no dialect markers is MSA in practice —
        // but say so with moderate confidence, because absence of markers is weak evidence.
        Verdict::lexicon(Lang::Ar, 0.65)
    }

    /// Latin script: Arabizi, French, or English.
    fn detect_latin_script(&self, text: &str) -> Verdict {
        let ara = self.arabizi.score(text);
        let fre = self.french.score(text);
        let digit_signal = arabizi_digit_score(text);

        // Two independent signals are required before calling Arabizi: marker words, and the
        // digit-as-consonant convention. Either alone is too eager — `3` appears in ordinary
        // French text, and `dar` is a French word too.
        let arabizi_signals =
            (ara.is_evidence() as u8) + (digit_signal >= 1 && ara.hits() > 0) as u8;

        if arabizi_signals >= 1 && ara.hits() >= fre.hits() {
            let conf =
                0.6 + (ara.weight * 0.1).min(0.25) + if digit_signal > 0 { 0.1 } else { 0.0 };
            let secondary = (fre.hits() > 0).then(|| (Lang::Fr, fre.coverage()));
            return Verdict::lexicon(Lang::Ary, conf.min(0.95)).with_secondary(secondary);
        }

        let eng = self.english.score(text);

        if fre.is_evidence() && fre.hits() > ara.hits() && fre.hits() >= eng.hits() {
            let secondary = (ara.hits() > 0).then(|| (Lang::Ary, ara.coverage()));
            return Verdict::lexicon(Lang::Fr, (0.6 + (fre.weight * 0.08).min(0.3)).min(0.95))
                .with_secondary(secondary);
        }

        // English, on the same footing as French.
        //
        // Without this, English fell through to trigram statistics, which is exactly where a
        // general-purpose detector is weakest: at query length whatlang reported an unreliable,
        // narrow margin for most English queries, so they landed under the confidence floor and
        // became `Und`. English was detected 53% of the time on the labelled set — worse than the
        // languages with a lexicon, and for the same reason.
        //
        // Ordered after French because the two lists are disjoint by construction (there is a test
        // for it), so whichever has more hits is the answer; the tie goes to French, which carries
        // far more Algerian traffic than English does.
        if eng.is_evidence() && eng.hits() > ara.hits() && eng.hits() > fre.hits() {
            let secondary = (fre.hits() > 0).then(|| (Lang::Fr, fre.coverage()));
            return Verdict::lexicon(Lang::En, (0.6 + (eng.weight * 0.08).min(0.3)).min(0.95))
                .with_secondary(secondary);
        }

        // No lexicon verdict. Fall back to trigram statistics, which is the one place a
        // general-purpose detector genuinely helps: separating French from English.
        match statistical(text) {
            Some((lang, confidence)) => Verdict {
                lang,
                confidence,
                secondary: None,
                source: Evidence::Statistical,
            },
            None => Verdict {
                lang: Lang::Und,
                confidence: 0.0,
                secondary: None,
                source: Evidence::Statistical,
            },
        }
    }

    /// Both scripts present in quantity: code-switching, which is routine here.
    fn detect_mixed(&self, text: &str, arabic_fraction: f32) -> Verdict {
        let d = self.darija_ar.score(text);
        let ara = self.arabizi.score(text);
        let fre = self.french.score(text);

        // Darija in either script plus French is the classic Algerian sentence. Report the
        // dominant one and record the other rather than flattening to `Mixed` and losing both.
        let darija_evidence = d.is_evidence() || ara.is_evidence();
        if darija_evidence && fre.hits() > 0 {
            let secondary_share = fre.coverage();
            if secondary_share >= self.config.mixed_secondary_min {
                return Verdict::lexicon(Lang::Mixed, 0.7)
                    .with_secondary(Some((Lang::Fr, secondary_share)));
            }
            return Verdict::lexicon(Lang::Ary, 0.75)
                .with_secondary(Some((Lang::Fr, secondary_share)));
        }
        if darija_evidence {
            return Verdict::lexicon(Lang::Ary, 0.75);
        }
        if fre.is_evidence() && arabic_fraction < 0.5 {
            return Verdict::lexicon(Lang::Fr, 0.65)
                .with_secondary(Some((Lang::Ar, arabic_fraction)));
        }
        Verdict::lexicon(Lang::Mixed, 0.6)
    }

    /// Apply the length discount and the confidence floor.
    fn finish(&self, raw: Verdict, scr: Script, token_count: usize) -> Detection {
        let Verdict {
            lang,
            confidence,
            secondary,
            source,
        } = raw;

        // The length discount applies to *statistical* verdicts only.
        //
        // Trigram detection genuinely is unreliable on two or three tokens, so discounting it
        // there is right. Lexicon evidence is the opposite: `wach rak khouya` is three tokens
        // that are all strong Darija markers, which is stronger evidence than the same three
        // markers buried in a long document, not weaker. Discounting it made every short Darija
        // query fall through to `Und` — the exact failure this detector exists to prevent.
        let adjusted = match source {
            Evidence::Lexicon => confidence,
            Evidence::Statistical => {
                let length_factor =
                    (token_count as f32 / self.config.full_confidence_tokens as f32).min(1.0);
                confidence * length_factor
            }
        };

        if adjusted < self.config.min_confidence {
            return Detection {
                lang: Lang::Und,
                confidence: adjusted,
                script: scr,
                secondary: None,
            };
        }

        Detection {
            lang,
            confidence: adjusted,
            script: scr,
            secondary,
        }
    }
}

/// Count the Arabizi digit-as-consonant convention: `3`=ع, `7`=ح, `9`=ق, `2`=ء, `5`=خ.
///
/// Only digits *adjacent to letters* count. A bare year like `2026` is not evidence of anything,
/// and treating it as such would classify every dated French query as Darija.
fn arabizi_digit_score(text: &str) -> usize {
    let mut score = 0;
    for token in tokens(text) {
        let chars: Vec<char> = token.chars().collect();
        let has_letter = chars.iter().any(|c| c.is_ascii_alphabetic());
        if !has_letter {
            continue;
        }
        for (i, c) in chars.iter().enumerate() {
            if !matches!(c, '2' | '3' | '5' | '7' | '9') {
                continue;
            }
            let prev_letter = i > 0 && chars[i - 1].is_ascii_alphabetic();
            let next_letter = i + 1 < chars.len() && chars[i + 1].is_ascii_alphabetic();
            if prev_letter || next_letter {
                score += 1;
            }
        }
    }
    score
}

thread_local! {
    /// Built once per thread: constructing a detector allocates its model, and this is on the
    /// query path.
    static DETECTOR: whatlang::Detector = whatlang::Detector::with_allowlist(vec![
        whatlang::Lang::Ara,
        whatlang::Lang::Fra,
        whatlang::Lang::Eng,
    ]);
}

/// Trigram statistics, restricted to the three languages we actually serve.
///
/// # Reading whatlang's confidence correctly
///
/// `Info::confidence()` is the *margin between candidate languages*, not the probability that
/// the choice is right. On short text the margin is naturally small even when the answer is
/// obviously correct — measured on the labelled set, whatlang picks the right language for
/// every English query while reporting confidences between 0.36 and 0.63.
///
/// Rescaling that margin and comparing it against our confidence floor conflates two different
/// scales, and had the effect of sending correct short English detections to `Und`. So we take
/// whatlang's *choice* as the signal and attach our own calibrated confidence to it, which stays
/// deliberately below what lexicon evidence earns.
fn statistical(text: &str) -> Option<(Lang, f32)> {
    use whatlang::Lang as W;

    // Restricted to the three languages we serve.
    //
    // Unrestricted, whatlang ranks a short query against every language it knows, and on
    // query-length text the winner is routinely one we do not serve at all: measured on this
    // corpus, "best places to visit in algeria" came back Latin, "what documents do i need for a
    // passport" Catalan, and "software engineer jobs remote" Norwegian. Each of those fell through
    // the arm below and became `Und`, so **ordinary English queries were not being detected at
    // all** — 53% accuracy on the English portion of the labelled set.
    //
    // We only ever serve ar, fr and en, so asking "which of these three" is both the question we
    // actually have and a far easier one on short text. It cannot introduce a wrong answer that
    // the unrestricted call would have got right, because any other winner was discarded anyway.
    let info = DETECTOR.with(|d| d.detect(text))?;
    let lang = match info.lang() {
        W::Ara => Lang::Ar,
        W::Fra => Lang::Fr,
        W::Eng => Lang::En,
        // A language we do not serve. Saying `Und` is more honest than forcing it into one
        // of ours, and it keeps retrieval wide rather than wrong.
        _ => return None,
    };

    let margin = info.confidence() as f32;
    let confidence = if info.is_reliable() {
        0.80
    } else if margin >= 0.30 {
        // A clear winner among three languages, on text with no lexicon markers.
        0.62
    } else {
        // Genuinely ambiguous. Fall below the floor and let it become `Und`.
        0.40
    };
    Some((lang, confidence))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn det() -> Detector {
        Detector::default()
    }

    #[test]
    fn darija_in_arabic_script() {
        for q in [
            "واش راك خويا",
            "شحال يدوم الملف",
            "وين نلقى المكتب نتاع هاد الخدمة",
            "راني قلت ليكم بلي لازم",
            "كاين واحد الطريقة سهلة بزاف",
        ] {
            let d = det().detect(q);
            assert_eq!(d.lang, Lang::Ary, "expected Darija for {q:?}, got {d:?}");
            assert_eq!(d.script, Script::Arabic);
        }
    }

    #[test]
    fn msa_is_not_mistaken_for_darija() {
        for q in [
            "أعلنت المصالح المعنية عن إجراءات جديدة لتبسيط العملية على المواطنين",
            "تشهد الولاية حركية كبيرة مع انطلاق المشاريع الجديدة",
            "وزارة التربية الوطنية تعلن عن نتائج الامتحانات",
        ] {
            let d = det().detect(q);
            assert_eq!(d.lang, Lang::Ar, "expected MSA for {q:?}, got {d:?}");
        }
    }

    #[test]
    fn arabizi_in_latin_script() {
        for q in [
            "wach rak khouya",
            "ch7al ydoum lmalaf",
            "win nelqa lmaktab nta3 had lkhedma",
            "rani goltlkom bli lazem",
            "kayen wahed tari9a sahla bezaf",
        ] {
            let d = det().detect(q);
            assert_eq!(d.lang, Lang::Ary, "expected Arabizi for {q:?}, got {d:?}");
            assert_eq!(d.script, Script::Latin);
        }
    }

    #[test]
    fn french_is_not_mistaken_for_arabizi() {
        for q in [
            "comment payer sa facture d electricite en ligne",
            "les services concernes ont annonce de nouvelles mesures",
            "offre emploi pour ingenieur en informatique",
        ] {
            let d = det().detect(q);
            assert_eq!(d.lang, Lang::Fr, "expected French for {q:?}, got {d:?}");
        }
    }

    #[test]
    fn a_bare_year_is_not_arabizi_evidence() {
        // `2026` contains a digit we treat as a consonant elsewhere. Counting it would make
        // every dated French query look like Darija.
        assert_eq!(arabizi_digit_score(&normalize("facture 2026")), 0);
        assert_eq!(arabizi_digit_score(&normalize("prix 3000 da")), 0);
        // ...but a digit inside a word does count.
        assert!(arabizi_digit_score(&normalize("ch7al")) > 0);
        assert!(arabizi_digit_score(&normalize("3andi")) > 0);
    }

    #[test]
    fn french_query_with_a_year_stays_french() {
        let d = det().detect("inscription universitaire 2026 pour les etudiants");
        assert_eq!(d.lang, Lang::Fr);
    }

    #[test]
    fn code_switched_arabic_and_french() {
        let d = det().detect("راني في la gare نتاع وهران مع les amis");
        assert!(
            matches!(d.lang, Lang::Ary | Lang::Mixed),
            "expected Darija or Mixed, got {d:?}"
        );
        assert!(
            d.secondary.is_some(),
            "code-switching should record a secondary language"
        );
    }

    #[test]
    fn short_ambiguous_input_is_undetermined_rather_than_guessed() {
        // Guessing here is what makes a Darija query return nothing.
        for q in ["ok", "test", "abc", "xyz"] {
            let d = det().detect(q);
            assert_eq!(d.lang, Lang::Und, "expected Und for {q:?}, got {d:?}");
        }
    }

    #[test]
    fn empty_and_symbol_only_input() {
        assert_eq!(det().detect("").lang, Lang::Und);
        assert_eq!(det().detect("   ").lang, Lang::Und);
        assert_eq!(det().detect("!!! ??? 😀").lang, Lang::Und);
        assert_eq!(det().detect("2026").lang, Lang::Und);
    }

    #[test]
    fn lexicon_evidence_is_not_discounted_for_short_input() {
        // Two tokens that are both strong Darija markers is decisive evidence, not weak
        // evidence. Discounting it by length is what previously made every short Darija query
        // fall through to Und — the precise failure this detector exists to prevent.
        let short = det().detect("واش راك");
        assert_eq!(short.lang, Lang::Ary);
        assert!(
            short.confidence >= 0.9,
            "short but unambiguous input should stay confident, got {}",
            short.confidence
        );
    }

    #[test]
    fn statistical_evidence_is_discounted_for_short_input() {
        // The trigram detector genuinely is unreliable on a couple of tokens, so a verdict
        // that rests on it alone must be held to a lower confidence.
        let d = det();
        let short = d.detect("hello world");
        let long = d.detect("hello world this is a longer sentence written in plain english");
        assert!(
            long.confidence > short.confidence,
            "statistical confidence should grow with length: {} vs {}",
            long.confidence,
            short.confidence
        );
    }

    #[test]
    fn more_markers_never_lower_confidence() {
        let d = det();
        let few = d.detect("واش راك");
        let many = d.detect("واش راك خويا شحال هاد الحاجة نتاع الخدمة بزاف");
        assert!(many.confidence >= few.confidence);
    }

    #[test]
    fn a_single_strong_marker_is_enough_in_arabic() {
        let d = det().detect("بزاف الناس راهم يسقسيو على هاد الموضوع");
        assert_eq!(d.lang, Lang::Ary);
    }

    #[test]
    fn detection_is_deterministic() {
        let d = det();
        let q = "wach rak khouya ch7al";
        assert_eq!(d.detect(q), d.detect(q));
    }

    #[test]
    fn normalisation_noise_does_not_change_the_answer() {
        let d = det();
        assert_eq!(d.detect("واش راك").lang, d.detect("واااش راك").lang);
        assert_eq!(d.detect("بزاف").lang, d.detect("بَزَاف").lang);
        assert_eq!(d.detect("wach rak").lang, d.detect("  WACH   RAK  ").lang);
    }

    #[test]
    fn is_actionable_gates_on_confidence() {
        assert!(!Detection::und(Script::Latin).is_actionable());
        let confident = det().detect("واش راك خويا شحال هاد الحاجة");
        assert!(confident.is_actionable());
    }

    #[test]
    fn respects_the_length_cap() {
        // A hostile document must not make detection unbounded work.
        let huge = "واش راك ".repeat(50_000);
        let d = Detector::new(DetectorConfig {
            max_chars: 100,
            ..Default::default()
        });
        assert_eq!(d.detect(&huge).lang, Lang::Ary);
    }
}
