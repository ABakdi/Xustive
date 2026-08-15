//! Sentiment accuracy and calibration against a labelled set.
//!
//! M1-T07.5 and M1-T07.6. The unit tests in `sentiment.rs` check individual rules — negation,
//! intensifiers, emoji. This checks the thing that decides whether the feature is usable: how
//! often the label is right, and whether the confidence it reports means anything.
//!
//! # What the numbers here mean
//!
//! These rows are **machine-generated and unreviewed** (blocker B7), and the lexicons were written
//! by the same process. A score measured this way says the lexicon covers this set — it is not an
//! accuracy measurement, and it must not be quoted as one. It is a regression guard, and a
//! scaffold for annotators to correct and extend.
//!
//! The gates are set below what the current implementation achieves, on purpose. A gate pinned to
//! today's number turns every future lexicon edit into a test failure, which trains people to edit
//! the gate. They are floors that catch real breakage, not targets.
//!
//! # Why calibration is checked separately from accuracy
//!
//! A scorer can be accurate and still badly calibrated: if it reports 0.9 confidence on everything
//! including its mistakes, downstream code that filters on confidence gets no protection at all.
//! Ranking and the admin console both do exactly that, so the ordering property — right answers
//! should carry more confidence than wrong ones — matters independently of the hit rate.

use xustive_core::{Lang, SentimentLabel as L};
use xustive_lang::sentiment::Scorer;

/// `(text, language, expected label)`.
///
/// Neutral rows are deliberately over-represented relative to a real corpus. A lexicon scorer's
/// characteristic failure is seeing sentiment in ordinary factual text, and a set made only of
/// clearly-polar sentences cannot detect that at all.
const LABELLED: &[(&str, Lang, L)] = &[
    // --- Darija, Latin script: positive -------------------------------------------------
    ("mlih bezaf had khedma", Lang::Ary, L::Positive),
    ("sahit khouya 3la lma3louma", Lang::Ary, L::Positive),
    ("wa3er had lprojet", Lang::Ary, L::Positive),
    ("rabi ykhalik ya khouya", Lang::Ary, L::Positive),
    ("tbarkallah 3likom khedma nadia", Lang::Ary, L::Positive),
    ("farhan bezaf bhad lkhbar", Lang::Ary, L::Positive),
    ("chbab bezaf had lblasa", Lang::Ary, L::Positive),
    ("yaatik saha 3la lmajhoud", Lang::Ary, L::Positive),
    ("rkhis w mlih", Lang::Ary, L::Positive),
    ("top had service", Lang::Ary, L::Positive),
    // --- Darija, Latin script: negative -------------------------------------------------
    ("khayeb bezaf had lkhedma", Lang::Ary, L::Negative),
    ("hogra w tahgir kamel", Lang::Ary, L::Negative),
    ("rani mahgour f had lidara", Lang::Ary, L::Negative),
    ("ghali bezaf machi ma39oul", Lang::Ary, L::Negative),
    ("makanch walou f had lblasa", Lang::Ary, L::Negative),
    ("kadhab w sarraq", Lang::Ary, L::Negative),
    ("karitha had ttanzim", Lang::Ary, L::Negative),
    ("rani sakhet men had sda3", Lang::Ary, L::Negative),
    ("fassad f kol blasa", Lang::Ary, L::Negative),
    ("z3afni bezaf had lmawdou3", Lang::Ary, L::Negative),
    ("machakel kbira f dossier", Lang::Ary, L::Negative),
    ("ta3tila w tsawil bark", Lang::Ary, L::Negative),
    // --- Darija: neutral, the case a lexicon most often gets wrong ------------------------
    ("weqtach teftah lbaladia", Lang::Ary, L::Neutral),
    (
        "win nelqa lbureau ta3 lhala lmadania",
        Lang::Ary,
        L::Neutral,
    ),
    ("ch7al ydoum lmalaf", Lang::Ary, L::Neutral),
    ("wach lazem men wata2iq", Lang::Ary, L::Neutral),
    ("rani nkhemem nbaddel lkhedma", Lang::Ary, L::Neutral),
    ("ghedwa nrouh l alger", Lang::Ary, L::Neutral),
    // --- Arabic script: positive ----------------------------------------------------------
    ("مليح بزاف هاد المشروع", Lang::Ary, L::Positive),
    ("صحيت خويا على المعلومة", Lang::Ary, L::Positive),
    ("تبارك الله عليكم خدمة واعرة", Lang::Ary, L::Positive),
    ("الحمد لله كلش مليح", Lang::Ary, L::Positive),
    // --- Arabic script: negative -----------------------------------------------------------
    ("خايب بزاف هاد التنظيم", Lang::Ary, L::Negative),
    ("حقرة و تحقير في كل بلاصة", Lang::Ary, L::Negative),
    ("غالي بزاف ماشي معقول", Lang::Ary, L::Negative),
    ("ماكاش والو في هاد البلاصة", Lang::Ary, L::Negative),
    // --- MSA: positive -----------------------------------------------------------------------
    (
        "نتائج ممتازة وتحسن ملحوظ في المؤشرات",
        Lang::Ar,
        L::Positive,
    ),
    ("خطوة إيجابية ومبادرة ناجحة", Lang::Ar, L::Positive),
    ("تحسن كبير في جودة الخدمات", Lang::Ar, L::Positive),
    // --- MSA: negative -----------------------------------------------------------------------
    ("فشل ذريع وتدهور خطير في الوضع", Lang::Ar, L::Negative),
    ("أزمة حادة وارتفاع مقلق في الأسعار", Lang::Ar, L::Negative),
    ("تراجع كبير وخسائر فادحة", Lang::Ar, L::Negative),
    // --- MSA: neutral, ordinary news copy -----------------------------------------------------
    (
        "تعقد الحكومة اجتماعا لدراسة مشروع قانون المالية",
        Lang::Ar,
        L::Neutral,
    ),
    ("افتتح الوزير المعرض الدولي للكتاب", Lang::Ar, L::Neutral),
    ("انطلقت عملية التسجيل في الجامعات", Lang::Ar, L::Neutral),
    (
        "سجلت المصالح المعنية العدد الإجمالي للملفات",
        Lang::Ar,
        L::Neutral,
    ),
    // --- French -------------------------------------------------------------------------------
    (
        "excellent service je recommande vivement",
        Lang::Fr,
        L::Positive,
    ),
    ("tres bonne qualite et prix correct", Lang::Fr, L::Positive),
    ("merci beaucoup pour votre aide", Lang::Fr, L::Positive),
    ("une catastrophe et un echec total", Lang::Fr, L::Negative),
    (
        "service horrible et personnel desagreable",
        Lang::Fr,
        L::Negative,
    ),
    (
        "beaucoup trop cher pour ce que c est",
        Lang::Fr,
        L::Negative,
    ),
    ("horaires d ouverture de la mairie", Lang::Fr, L::Neutral),
    ("inscription universitaire en ligne", Lang::Fr, L::Neutral),
    (
        "le ministre a annonce de nouvelles mesures",
        Lang::Fr,
        L::Neutral,
    ),
    // --- English -------------------------------------------------------------------------------
    (
        "excellent work and very helpful staff",
        Lang::En,
        L::Positive,
    ),
    ("great service i would recommend it", Lang::En, L::Positive),
    (
        "terrible experience and a complete waste",
        Lang::En,
        L::Negative,
    ),
    (
        "awful quality and very disappointing",
        Lang::En,
        L::Negative,
    ),
    ("how to renew a driving licence", Lang::En, L::Neutral),
    (
        "weather forecast for algiers tomorrow",
        Lang::En,
        L::Neutral,
    ),
    // --- Negation: the rule most likely to regress silently -------------------------------------
    ("machi mlih had lkhedma", Lang::Ary, L::Negative),
    ("ce n est pas bon du tout", Lang::Fr, L::Negative),
    ("this is not good at all", Lang::En, L::Negative),
    ("ماشي مليح هاد الشي", Lang::Ary, L::Negative),
];

struct Report {
    right: usize,
    total: usize,
    misses: Vec<String>,
    /// Mean confidence on correct and on incorrect predictions.
    conf_right: f32,
    conf_wrong: f32,
}

fn evaluate() -> Report {
    let scorer = Scorer::default();
    let (mut right, mut cr, mut cw, mut nw) = (0usize, 0.0f32, 0.0f32, 0usize);
    let mut misses = Vec::new();

    for (text, lang, want) in LABELLED {
        let got = scorer.score(text, *lang);
        if got.label == *want {
            right += 1;
            cr += got.confidence;
        } else {
            nw += 1;
            cw += got.confidence;
            misses.push(format!(
                "  want {:8} got {:8} conf {:.2}  {text:?}",
                want.as_str(),
                got.label.as_str(),
                got.confidence
            ));
        }
    }
    Report {
        right,
        total: LABELLED.len(),
        conf_right: if right > 0 { cr / right as f32 } else { 0.0 },
        conf_wrong: if nw > 0 { cw / nw as f32 } else { 0.0 },
        misses,
    }
}

#[test]
fn accuracy_meets_the_gate() {
    let r = evaluate();
    let pct = 100.0 * r.right as f32 / r.total as f32;
    println!("sentiment accuracy {pct:.1}% ({}/{})", r.right, r.total);
    for m in &r.misses {
        println!("{m}");
    }
    assert!(
        pct >= 75.0,
        "sentiment accuracy {pct:.1}% is below the 75% floor"
    );
}

/// The failure that matters most: calling something positive when it is negative.
///
/// A miss into `Neutral` costs a signal. A miss across the sign shows a complaint as praise, which
/// is worse than declining to judge — the admin console and any future moderation view both read
/// this label directly.
#[test]
fn polarity_is_never_inverted() {
    let scorer = Scorer::default();
    let mut inverted = Vec::new();
    for (text, lang, want) in LABELLED {
        let got = scorer.score(text, *lang).label;
        let flipped = matches!(
            (want, got),
            (L::Positive, L::Negative) | (L::Negative, L::Positive)
        );
        if flipped {
            inverted.push(format!(
                "  want {:8} got {:8}  {text:?}",
                want.as_str(),
                got.as_str()
            ));
        }
    }
    assert!(
        inverted.len() * 100 <= LABELLED.len(),
        "polarity inverted on {} of {} cases:\n{}",
        inverted.len(),
        LABELLED.len(),
        inverted.join("\n")
    );
}

/// Confidence must carry information (M1-T07.6).
///
/// Not a probability calibration — the scorer makes no such claim, and asserting one against a set
/// this size would be measuring noise. The weaker, genuinely useful property is ordering: if the
/// scorer is no less confident when wrong than when right, then every downstream confidence filter
/// is decorative, and code that trusts it is trusting nothing.
#[test]
fn confidence_is_higher_when_right_than_when_wrong() {
    let r = evaluate();
    if r.misses.is_empty() {
        return; // Nothing to compare against, which is a pass rather than a skip.
    }
    println!(
        "mean confidence: right {:.3}, wrong {:.3}",
        r.conf_right, r.conf_wrong
    );
    assert!(
        r.conf_right > r.conf_wrong,
        "mean confidence when right ({:.3}) is not above when wrong ({:.3}); a confidence that \
         does not separate the two makes every downstream filter on it decorative",
        r.conf_right,
        r.conf_wrong
    );
}

/// Ordinary factual text must not acquire a polarity.
///
/// The characteristic lexicon failure. `تحسن` and `ارتفاع` are ordinary news vocabulary before they
/// are sentiment, and a scorer that reads every administrative sentence as an opinion produces a
/// corpus where sentiment means nothing.
#[test]
fn neutral_text_stays_neutral() {
    let scorer = Scorer::default();
    let neutrals: Vec<_> = LABELLED
        .iter()
        .filter(|(_, _, l)| *l == L::Neutral)
        .collect();
    assert!(
        neutrals.len() >= 15,
        "too few neutral cases to be meaningful"
    );

    let wrong = neutrals
        .iter()
        .filter(|(t, lang, _)| scorer.score(t, *lang).label != L::Neutral)
        .count();
    let pct = 100.0 * (neutrals.len() - wrong) as f32 / neutrals.len() as f32;
    println!(
        "neutral held {pct:.1}% ({}/{})",
        neutrals.len() - wrong,
        neutrals.len()
    );
    assert!(
        pct >= 70.0,
        "only {pct:.1}% of neutral text stayed neutral; a scorer that finds sentiment in \
         administrative copy produces a corpus where sentiment means nothing"
    );
}
