//! Topic labelling (M2-T06.6).
//!
//! A coarse subject label — politics, economy, sport, and so on — so results can be grouped and a
//! vertical like "news" can be sliced by subject. Not a fine taxonomy: a handful of labels a reader
//! recognises, assigned by the vocabulary a document actually uses.
//!
//! A keyword classifier, for the same reasons as the gazetteer and the spam scorer: the label set
//! is small and stable, a lookup is cheap and explainable, and it never invents a topic that has no
//! evidence in the text. A document can carry more than one label — an article on a sports ministry
//! budget is both sport and economy — and one with no clear subject carries none, which is honest:
//! a wrong label is worse than no label.
//!
//! Matching is folded and whole-token ([[xustive_text]]), the same as everywhere else, so Arabic
//! orthography and French casing do not matter.

use std::collections::HashMap;

use xustive_text::{fold, tokens};

/// `(label, keywords)`. Keywords are single tokens, folded on use. Kept specific: a keyword common
/// across topics labels nothing.
const TOPICS: &[(&str, &[&str])] = &[
    (
        "politics",
        &[
            "الحكومة",
            "الوزير",
            "البرلمان",
            "الرئيس",
            "المجلس",
            "الانتخابات",
            "قانون",
            "سياسة",
            "gouvernement",
            "ministre",
            "parlement",
            "president",
            "election",
            "politique",
            "loi",
        ],
    ),
    (
        "economy",
        &[
            "اقتصاد",
            "استثمار",
            "التجارة",
            "الصادرات",
            "الأسعار",
            "الدينار",
            "ميزانية",
            "مؤسسة",
            "economie",
            "investissement",
            "exportations",
            "budget",
            "dinar",
            "entreprise",
            "inflation",
            "marche",
        ],
    ),
    (
        "sport",
        &[
            "مباراة",
            "الفريق",
            "البطولة",
            "كرة",
            "المنتخب",
            "لاعب",
            "المدرب",
            "الدوري",
            "match",
            "equipe",
            "championnat",
            "football",
            "joueur",
            "entraineur",
            "selection",
        ],
    ),
    (
        "culture",
        &[
            "ثقافة",
            "فيلم",
            "مهرجان",
            "الفنان",
            "كتاب",
            "معرض",
            "مسرح",
            "موسيقى",
            "culture",
            "film",
            "festival",
            "artiste",
            "livre",
            "exposition",
            "theatre",
            "musique",
        ],
    ),
    (
        "health",
        &[
            "الصحة",
            "المستشفى",
            "الطبيب",
            "المرض",
            "لقاح",
            "علاج",
            "وباء",
            "الأدوية",
            "sante",
            "hopital",
            "medecin",
            "maladie",
            "vaccin",
            "traitement",
            "epidemie",
        ],
    ),
    (
        "education",
        &[
            "التعليم",
            "المدرسة",
            "الجامعة",
            "التلاميذ",
            "الطلبة",
            "البكالوريا",
            "الأستاذ",
            "الامتحان",
            "education",
            "ecole",
            "universite",
            "eleves",
            "etudiants",
            "baccalaureat",
            "examen",
        ],
    ),
    (
        "technology",
        &[
            "التكنولوجيا",
            "الإنترنت",
            "الرقمنة",
            "تطبيق",
            "هاتف",
            "الشبكة",
            "المعلوماتية",
            "technologie",
            "internet",
            "numerique",
            "application",
            "reseau",
            "informatique",
        ],
    ),
    (
        "society",
        &[
            "المجتمع",
            "السكان",
            "الشباب",
            "الأسرة",
            "الحادث",
            "الأمن",
            "الهجرة",
            "البيئة",
            "societe",
            "population",
            "jeunesse",
            "famille",
            "accident",
            "securite",
            "environnement",
        ],
    ),
];

/// The minimum keyword hits before a topic is assigned.
///
/// Two, not one: a single keyword is too easily incidental — one mention of "law" does not make an
/// article about politics. Two distinct-or-repeated hits is a subject, not a passing reference.
const MIN_HITS: usize = 2;

/// The most labels a document may carry, most-evidenced first.
///
/// Three: a story can genuinely span economy, politics and society, but past that the labels stop
/// meaning anything and the list becomes noise.
const MAX_LABELS: usize = 3;

/// Label a document from its title and body. Returns the topics with the most evidence, up to
/// [`MAX_LABELS`], each with at least [`MIN_HITS`] keyword hits. Empty when nothing is clear.
pub fn label(title: &str, body: &str) -> Vec<String> {
    let folded = fold(&format!("{title} {body}"));
    let toks: Vec<&str> = tokens(&folded).collect();
    if toks.is_empty() {
        return Vec::new();
    }
    let mut freq: HashMap<&str, usize> = HashMap::new();
    for t in &toks {
        *freq.entry(*t).or_insert(0) += 1;
    }

    let mut scored: Vec<(&str, usize)> = TOPICS
        .iter()
        .map(|(label, kws)| {
            let hits: usize = kws
                .iter()
                .map(|kw| *freq.get(fold(kw).as_str()).unwrap_or(&0))
                .sum();
            (*label, hits)
        })
        .filter(|(_, hits)| *hits >= MIN_HITS)
        .collect();

    // Most-evidenced first; a stable label order breaks ties so the result is deterministic.
    scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    scored
        .into_iter()
        .take(MAX_LABELS)
        .map(|(label, _)| label.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sports_report_is_labelled_sport() {
        let body = "فاز الفريق الوطني في المباراة وسجل اللاعب هدفين، وأشاد المدرب بأداء المنتخب في البطولة.";
        assert_eq!(label("مباراة", body), vec!["sport"]);
    }

    #[test]
    fn an_economic_story_is_labelled_economy() {
        let body = "le ministre de l economie a annonce une hausse des exportations et un nouveau \
                    budget pour soutenir l investissement des entreprises cette annee";
        assert!(label("economie", body).contains(&"economy".to_string()));
    }

    #[test]
    fn a_cross_cutting_story_gets_more_than_one_label() {
        // A sports-budget story: sport and economy both present.
        let body = "خصصت الحكومة ميزانية جديدة للرياضة، وأكد الوزير أن الاستثمار في الفريق الوطني \
                    والبطولة سيرفع من مستوى كرة القدم، في إطار سياسة اقتصادية داعمة.";
        let labels = label("رياضة", body);
        assert!(labels.contains(&"sport".to_string()));
        assert!(
            labels.len() >= 2,
            "a cross-cutting story should carry several labels: {labels:?}"
        );
    }

    #[test]
    fn a_single_keyword_is_not_enough() {
        // One mention of "law" in an otherwise non-political story must not label it politics.
        let body = "the new road was opened today after two years of works and a law was cited \
                    once in passing by an official at the ceremony attended by local residents";
        assert!(
            !label("road", body).contains(&"politics".to_string()),
            "a single incidental keyword should not assign a topic"
        );
    }

    #[test]
    fn a_subjectless_text_gets_no_label() {
        assert!(label("", "").is_empty());
        assert!(label(
            "hello",
            "this is a short neutral sentence with no clear subject"
        )
        .is_empty());
    }

    #[test]
    fn at_most_three_labels() {
        // Stuff keywords from many topics; the result is still capped.
        let body =
            "الحكومة الوزير اقتصاد استثمار مباراة الفريق ثقافة فيلم الصحة المستشفى التعليم المدرسة";
        assert!(label("x", body).len() <= MAX_LABELS);
    }
}
