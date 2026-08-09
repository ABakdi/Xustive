//! Is this query a question?
//!
//! A question deserves a different page. Someone typing `سونلغاز` wants a list of pages about
//! Sonelgaz; someone typing `كيفاش نخلص فاتورة سونلغاز` wants **an answer**, and putting ten blue
//! links above it makes them do the work themselves.
//!
//! # Why this is not a language model
//!
//! It runs on every search, so it has microseconds. It is also the gate on a decision that must
//! not be wrong often in one particular direction: promoting a summary above the results for a
//! query that was not a question pushes the thing the reader wanted off the screen.
//!
//! So it is conservative and lexical — question words, question marks, and a small set of
//! Algerian phrasings. It will miss questions phrased as statements (`طريقة دفع فاتورة`), which
//! costs a reader nothing: they still get results.
//!
//! # Darija matters more than the others here
//!
//! Algerians ask questions in Darija far more than in MSA, and Darija questions are written in
//! both scripts — `chhal`, `شحال`, `kifach`, `كيفاش`. A detector covering only formal Arabic and
//! French would miss most real questions this engine sees.

use xustive_text::normalize;

/// Question words and phrasings, by language.
///
/// Matched as whole words after normalisation, not as substrings — `منين` inside a longer word is
/// not a question, and `who` inside `whose` is not either.
const QUESTION_WORDS: &[&str] = &[
    // Modern Standard Arabic
    "كيف",
    "لماذا",
    "متى",
    "اين",
    "أين",
    "ماذا",
    "من",
    "هل",
    "كم",
    "ما",
    "لمن",
    "أي",
    "اي",
    "ايهما",
    "أيهما",
    // Darija, Arabic script. The forms people actually type.
    "كيفاش",
    "شحال",
    "علاش",
    "وين",
    "منين",
    "وقتاش",
    "شكون",
    "واش",
    "شنو",
    "شنا",
    "كاش",
    "وشنو",
    "قداش",
    "فين",
    // Darija, Latin script — a large share of queries arrive this way.
    "kifach",
    "kifash",
    "chhal",
    "ch7al",
    "9adech",
    "kadech",
    "3lach",
    "alach",
    "win",
    "wine",
    "mnin",
    "waqtach",
    "chkoun",
    "wach",
    "chnou",
    "chno",
    "wchnou",
    // French
    "comment",
    "pourquoi",
    "quand",
    "où",
    "ou",
    "quoi",
    "qui",
    "combien",
    "quel",
    "quelle",
    "quels",
    "quelles",
    "est-ce",
    // English
    "how",
    "why",
    "when",
    "where",
    "what",
    "who",
    "which",
    "whose",
    "whom",
    "is",
    "are",
    "does",
    "do",
    "can",
    "should",
    "will",
];

/// Multi-word openings that are questions even though their first word is ambiguous.
///
/// `ما هو` is a question; `ما` alone is also a negation particle and far too common to trust.
const QUESTION_PHRASES: &[&str] = &[
    "ما هو",
    "ما هي",
    "من هو",
    "من هي",
    "ما معنى",
    "كم من",
    "هل يمكن",
    "qu'est-ce",
    "est-ce que",
    "c'est quoi",
    "how do",
    "how to",
    "how much",
    "how many",
    "what is",
    "what are",
    "kifach n",
    "chhal men",
];

/// Words that are questions only at the start.
///
/// `من` means both "who" and "from", and `is`/`do`/`can` are ordinary verbs mid-sentence. Leading
/// position is what distinguishes an interrogative from a preposition, and getting this wrong
/// would fire on a large share of ordinary queries.
const LEADING_ONLY: &[&str] = &[
    "من", "ما", "اي", "أي", "ou", "qui", "quoi", "is", "are", "does", "do", "can", "should",
    "will", "which", "what", "who", "win", "wine",
];

/// Whether a query reads as a question.
pub fn is_question(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return false;
    }

    // An explicit question mark settles it, in either script. `؟` is the Arabic one and a reader
    // typing it has been unambiguous.
    if trimmed.ends_with('?') || trimmed.ends_with('؟') {
        return true;
    }

    // Normalised so Arabic diacritics, alef forms and digit sets do not defeat a match — the same
    // normalisation the index uses, so "what the query says" means one thing throughout.
    let norm = normalize(trimmed);
    let lower = norm.to_lowercase();

    for phrase in QUESTION_PHRASES {
        if lower.contains(phrase) {
            return true;
        }
    }

    let words: Vec<&str> = lower.split_whitespace().collect();
    if words.is_empty() {
        return false;
    }
    // A single word is a topic, not a question, whatever that word is. `كيف` alone is somebody
    // exploring, and answering it would be answering a question they did not ask.
    if words.len() < 2 {
        return false;
    }

    for (i, word) in words.iter().enumerate() {
        let clean = word.trim_matches(|c: char| !c.is_alphanumeric());
        if clean.is_empty() {
            continue;
        }
        if !QUESTION_WORDS.contains(&clean) {
            continue;
        }
        if LEADING_ONLY.contains(&clean) && i != 0 {
            continue;
        }
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_mark_settles_it_in_either_script() {
        assert!(is_question("سونلغاز؟"));
        assert!(is_question("what is this?"));
        // Even a single word, because the reader was explicit.
        assert!(is_question("الجزائر؟"));
    }

    #[test]
    fn darija_questions_are_recognised_in_both_scripts() {
        // The case that matters most: Algerians ask in Darija far more than in MSA, and write it
        // in both scripts. A detector covering only formal Arabic would miss most real questions.
        for q in [
            "كيفاش نخلص فاتورة سونلغاز",
            "شحال سعر البنزين",
            "علاش الانترنت بطيء",
            "وين نلقى استمارة",
            "kifach ndir passport",
            "chhal taman essence",
            "3lach internet bati",
            "wach rahi lois",
        ] {
            assert!(is_question(q), "{q:?} should read as a question");
        }
    }

    #[test]
    fn msa_french_and_english_questions_are_recognised() {
        for q in [
            "كيف اجدد جواز السفر",
            "ما هو رقم الضمان الاجتماعي",
            "متى تبدأ العطلة",
            "comment renouveler le passeport",
            "pourquoi les prix augmentent",
            "combien coute le gasoil",
            "how to renew a passport",
            "what is the capital of algeria",
        ] {
            assert!(is_question(q), "{q:?} should read as a question");
        }
    }

    #[test]
    fn a_topic_is_not_a_question() {
        // The direction that must not be wrong often: promoting an answer above the results for a
        // query that was not a question pushes what the reader wanted off the screen.
        for q in [
            "سونلغاز",
            "الجزائر",
            "prix carburant",
            "passeport biometrique",
            "elkhabar",
            "meteo alger",
            "football algerie",
            "recrutement sonatrach",
        ] {
            assert!(!is_question(q), "{q:?} is a topic, not a question");
        }
    }

    #[test]
    fn a_single_word_is_a_topic_whatever_the_word() {
        // `كيف` alone is someone exploring. Answering it would answer a question they did not ask.
        for q in ["كيف", "how", "comment", "شحال", "why"] {
            assert!(!is_question(q), "{q:?} alone should not be a question");
        }
    }

    #[test]
    fn ambiguous_words_only_count_at_the_start() {
        // `من` is both "who" and "from"; `is` and `do` are ordinary verbs mid-sentence. Without
        // this rule the detector fires on a large share of perfectly ordinary queries.
        assert!(!is_question("مديرية الضرائب من ولاية وهران"));
        assert!(!is_question("liste des produits do brasil"));
        assert!(is_question("من هو رئيس الجمهورية"));
    }

    #[test]
    fn diacritics_and_digit_sets_do_not_defeat_it() {
        // Normalised with the same function the index uses, so "what the query says" means one
        // thing throughout the system.
        assert!(is_question("كَيْفَ أجدد جواز السفر"));
        assert!(is_question("شحال سعر ٩٥"));
    }

    #[test]
    fn nothing_panics() {
        for q in ["", "   ", "?", "؟", "؟؟؟", "\0", "من", "a"] {
            let _ = is_question(q);
        }
    }
}
