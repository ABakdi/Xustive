//! Detection accuracy against a labelled set.
//!
//! The unit tests in `detect.rs` check individual rules. This checks the thing that actually
//! matters: how often the detector is right, per language, on realistic input.
//!
//! Targets come from the milestone exit gate: ≥ 92 % overall, ≥ 85 % on `ary`, and — the one
//! that would break the product — ≤ 3 % of Darija misclassified as French, because narrowing
//! retrieval to French on a Darija query returns nothing at all.
//!
//! This set is a starter. It needs expanding and reviewing by a native Algerian speaker
//! (blocker B7), and every real-world misdetection reported should become a row here.

use xustive_core::Lang;
use xustive_lang::Detector;

/// `(text, expected)`. `Und` means "we should decline to guess".
const LABELLED: &[(&str, Lang)] = &[
    // --- Darija, Arabic script -------------------------------------------------------
    ("واش راك خويا", Lang::Ary),
    ("واش راكم خاوتي", Lang::Ary),
    ("شحال يدوم الملف باش يخرج", Lang::Ary),
    ("وين نلقى المكتب نتاع هاد الخدمة", Lang::Ary),
    ("راني قلت ليكم بلي الملف لازم يكون كامل", Lang::Ary),
    ("كاين واحد الطريقة سهلة بزاف غير اتبع الخطوات", Lang::Ary),
    ("بزاف الناس راهم يسقسيو على هاد الموضوع", Lang::Ary),
    ("دروك جاوبوني شحال يدوم", Lang::Ary),
    ("خويا حاب نعرف وين نلقى المكتب", Lang::Ary),
    ("ماكاش والو في هاد البلاصة", Lang::Ary),
    ("كيفاش ندير باش نجيب الوثيقة", Lang::Ary),
    ("علاش ما جاوبونيش على الطلب تاعي", Lang::Ary),
    ("هاد الحاجة واعرة بزاف", Lang::Ary),
    ("صحيت خويا على المعلومة", Lang::Ary),
    ("ياخي لازم نروح للسبيطار", Lang::Ary),
    // --- Darija, Latin script (Arabizi) ------------------------------------------------
    ("wach rak khouya", Lang::Ary),
    ("wach rakom khawti", Lang::Ary),
    ("ch7al ydoum lmalaf bach yekhroj", Lang::Ary),
    ("win nelqa lmaktab nta3 had lkhedma", Lang::Ary),
    ("rani goltlkom bli lmalaf lazem ykoun kamel", Lang::Ary),
    ("kayen wahed tari9a sahla bezaf", Lang::Ary),
    ("bezaf nas rahom yesqsiw 3la had lmawdou3", Lang::Ary),
    ("drok jawbouni ch7al ydoum", Lang::Ary),
    ("khoya 7ab n3ref win nelqa lmaktab", Lang::Ary),
    ("makach walou f had lblasa", Lang::Ary),
    ("kifach ndir bach njib lwatiqa", Lang::Ary),
    ("3lach ma jawbounich 3la talab ta3i", Lang::Ary),
    ("sahit khoya 3la lma3louma", Lang::Ary),
    ("labas 3lik khouya", Lang::Ary),
    ("ndiro chwiya sabr", Lang::Ary),
    // --- Modern Standard Arabic ----------------------------------------------------------
    ("أعلنت المصالح المعنية عن إجراءات جديدة", Lang::Ar),
    ("تشهد الولاية حركية كبيرة مع انطلاق المشاريع", Lang::Ar),
    ("وزارة التربية الوطنية تعلن نتائج الامتحانات", Lang::Ar),
    ("افتتاح معرض دولي للكتاب بمشاركة دور نشر عربية", Lang::Ar),
    ("ارتفاع أسعار المواد الاستهلاكية خلال الأشهر الأخيرة", Lang::Ar),
    ("دعت الجهات المعنية المواطنين إلى استكمال الملفات", Lang::Ar),
    ("انطلاق حملة التلقيح عبر المؤسسات الصحية", Lang::Ar),
    ("توزيع حصص جديدة من السكنات الاجتماعية", Lang::Ar),
    ("المنتخب الوطني يستعد للمباراة المقبلة", Lang::Ar),
    ("تحسين تدفق الإنترنت عبر عدة ولايات", Lang::Ar),
    // --- French ---------------------------------------------------------------------------
    ("comment payer sa facture d electricite en ligne", Lang::Fr),
    (
        "les services concernes ont annonce de nouvelles mesures",
        Lang::Fr,
    ),
    ("offre emploi pour ingenieur en informatique", Lang::Fr),
    ("ouverture des inscriptions universitaires", Lang::Fr),
    ("renforcement du reseau de transport urbain", Lang::Fr),
    ("distribution de nouveaux quotas de logements", Lang::Fr),
    (
        "le responsable a precise que l operation touchera",
        Lang::Fr,
    ),
    (
        "amelioration du debit internet dans plusieurs wilayas",
        Lang::Fr,
    ),
    ("prix du carburant a la pompe cette semaine", Lang::Fr),
    ("demande de passeport biometrique en ligne", Lang::Fr),
    // --- English ------------------------------------------------------------------------------
    ("how to apply for a student visa", Lang::En),
    ("the government announced new measures today", Lang::En),
    ("best restaurants in the city centre", Lang::En),
    ("weather forecast for the coming week", Lang::En),
    ("how much does it cost to travel there", Lang::En),
    // --- Deliberately undetermined --------------------------------------------------------------
    ("2026", Lang::Und),
    ("!!! ???", Lang::Und),
    ("", Lang::Und),
    ("😀😀", Lang::Und),
];

#[derive(Default)]
struct Stats {
    correct: usize,
    total: usize,
}

/// A misdetection: what it was, what we expected, what we said.
struct Miss {
    text: String,
    want: Lang,
    got: Lang,
}

/// The full outcome of one evaluation run.
struct Report {
    overall: Stats,
    per_lang: std::collections::HashMap<Lang, Stats>,
    misses: Vec<Miss>,
}

impl Stats {
    fn accuracy(&self) -> f32 {
        if self.total == 0 {
            1.0
        } else {
            self.correct as f32 / self.total as f32
        }
    }
}

fn evaluate() -> Report {
    use std::collections::HashMap;

    let d = Detector::default();
    let mut overall = Stats::default();
    let mut per_lang: HashMap<Lang, Stats> = HashMap::new();
    let mut misses = Vec::new();

    for (text, expected) in LABELLED {
        let got = d.detect(text).lang;
        let entry = per_lang.entry(*expected).or_default();
        entry.total += 1;
        overall.total += 1;
        if got == *expected {
            entry.correct += 1;
            overall.correct += 1;
        } else {
            misses.push(Miss {
                text: (*text).to_string(),
                want: *expected,
                got,
            });
        }
    }
    Report {
        overall,
        per_lang,
        misses,
    }
}

#[test]
fn overall_accuracy_meets_the_gate() {
    let r = evaluate();

    let mut report = String::new();
    report.push_str(&format!(
        "\noverall {:.1}% ({}/{})\n",
        r.overall.accuracy() * 100.0,
        r.overall.correct,
        r.overall.total
    ));
    let mut langs: Vec<_> = r.per_lang.iter().collect();
    langs.sort_by_key(|(l, _)| l.as_str());
    for (lang, s) in langs {
        report.push_str(&format!(
            "  {:5} {:.1}% ({}/{})\n",
            lang.as_str(),
            s.accuracy() * 100.0,
            s.correct,
            s.total
        ));
    }
    if !r.misses.is_empty() {
        report.push_str("\nmisdetections:\n");
        for m in &r.misses {
            report.push_str(&format!(
                "  want {:5} got {:5}  {:?}\n",
                m.want.as_str(),
                m.got.as_str(),
                m.text
            ));
        }
    }
    println!("{report}");

    assert!(
        r.overall.accuracy() >= 0.92,
        "overall accuracy below the 92% gate{report}"
    );
}

#[test]
fn darija_accuracy_meets_the_gate() {
    let r = evaluate();
    let ary = r
        .per_lang
        .get(&Lang::Ary)
        .expect("labelled set must contain Darija");
    assert!(
        ary.accuracy() >= 0.85,
        "Darija accuracy {:.1}% is below the 85% gate ({}/{})",
        ary.accuracy() * 100.0,
        ary.correct,
        ary.total
    );
}

#[test]
fn darija_is_almost_never_called_french() {
    // The failure that breaks the product. Narrowing retrieval to French on a Darija query
    // returns nothing, so the user concludes the search engine does not know their language.
    let r = evaluate();
    let ary_total = LABELLED.iter().filter(|(_, l)| *l == Lang::Ary).count();
    let ary_as_fr = r
        .misses
        .iter()
        .filter(|m| m.want == Lang::Ary && m.got == Lang::Fr)
        .count();

    let rate = ary_as_fr as f32 / ary_total as f32;
    assert!(
        rate <= 0.03,
        "{ary_as_fr}/{ary_total} ({:.1}%) of Darija was called French, gate is 3%",
        rate * 100.0
    );
}

#[test]
fn nothing_confidently_wrong_on_undetermined_input() {
    // Refusing to guess is a correct answer. Guessing confidently on noise is not.
    let d = Detector::default();
    for (text, expected) in LABELLED.iter().filter(|(_, l)| *l == Lang::Und) {
        let got = d.detect(text);
        assert_eq!(
            got.lang, *expected,
            "{text:?} should be undetermined, got {got:?}"
        );
        assert!(!got.is_actionable());
    }
}
