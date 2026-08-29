//! "Did you mean": spelling correction from what the corpus and the readers actually write.
//!
//! Meilisearch already tolerates typos *inside* retrieval; what it cannot do is tell the reader
//! that `couscuos` found what `couscous` would have found better, or that a query that found
//! nothing has an obvious neighbour that finds plenty. This does that, in two parts:
//!
//! 1. A **vocabulary** — word → frequency — built from document titles and excerpts and from the
//!    first-party search log's queries that found results ([[ADR-0030]]), refreshed with the
//!    suggestion index. Titles and excerpts rather than bodies: a typo in a body is noise, a word
//!    in a title is a word people search for.
//! 2. A **corrector**: each query token that is rare or unknown is replaced by the most frequent
//!    vocabulary word within Damerau–Levenshtein distance 1 (2 for longer words) that is
//!    markedly more frequent. Numbers, short tokens and operators are left alone.
//!
//! The corrector proposes; the search decides. [[Query Pipeline]] runs the corrected query and
//! only applies it when the typed query found nothing (or a weak top result) and the corrected
//! one finds more — otherwise it is offered as "did you mean". A correction that is not verified
//! by results is never shown.

use std::collections::HashMap;
use std::sync::Arc;

/// Word frequencies plus the buckets a candidate search walks.
///
/// Everything is keyed on a word's **shape**: the word with Arabic orthographic variants folded
/// ([`xustive_text::fold`]) and Latin accents removed, so `algerien` and `algérien`, `الجزائر`
/// and `الجزاءر` are the same word. That is what makes "is this a real word?" answerable for a
/// reader who types without accents — the common case on a phone keyboard — and it keeps the
/// corrector from "fixing" a correctly spelled unaccented word into a different one.
#[derive(Default)]
pub struct Vocabulary {
    /// Canonical (normalised, accents kept) word → how often it was seen.
    freq: HashMap<String, u32>,
    /// Shape → total frequency of every word with that shape.
    shapes: HashMap<String, u32>,
    /// (first shape char, shape length) → candidates as (shape, canonical word).
    buckets: HashMap<(char, usize), Vec<(Arc<str>, Arc<str>)>>,
}

/// A word the corpus has never seen is corrected on modest evidence; a word it has seen **at
/// all** is only corrected on overwhelming evidence.
///
/// The corpus is not Algeria-shaped — it holds far more of the English and French web than of
/// the Algerian one — and a French word typed without its accents is rare in it while its
/// English cousin is common: `hopital` in 3 documents against `hospital` in 13, `probleme` in 7
/// against `problems` in 64, `universite` in 21 against `university` in 138, `alger` in 44
/// against `aller` in 135. Frequency alone cannot tell those from typos; twenty times can,
/// because a real typo appears once or not at all and its correct spelling appears everywhere.
const RATIO_KNOWN: u32 = 20;

/// A candidate must be this many times more frequent than the token it replaces.
///
/// Three, not ten. The vocabulary is built from crawled pages, and pages contain misspellings:
/// `couscuos` occurs a dozen times in a 20 000-document sample against `couscous`'s fifty-six.
/// A ten-times bar is unreachable at that scale and left every real typo uncorrected; three
/// separates "the corpus overwhelmingly writes it the other way" from "both spellings are in
/// use" — `algerien` (54) against `algérie` (54) stays exactly as typed.
const RATIO: u32 = 3;

/// Names the vocabulary is seeded with — the wilayas — so they are never corrected away and
/// are always available as corrections. Weighted above [`KNOWN`] for exactly that reason.
const SEED_WEIGHT: u32 = KNOWN + 1;
/// A word must be seen this often to be a correction candidate at all.
const MIN_FREQ: u32 = 4;
/// A word this common is never corrected, whatever its neighbours: at this frequency it is a
/// word of the corpus and the risk of being wrong about it outweighs the typo it might be. The
/// ratio does the work below this line — an absolute cutoff cannot, because the counts scale
/// with however much of the corpus the vocabulary sampled.
const KNOWN: u32 = 200;
/// Tokens shorter than this are never corrected: too many neighbours, too little signal.
const MIN_LEN: usize = 3;

impl Vocabulary {
    /// Build from texts — one per document — and from past queries, which add weight to words
    /// the corpus already has (see the loop below for why they cannot add new ones).
    pub fn build(
        texts: impl Iterator<Item = String>,
        queries: impl Iterator<Item = String>,
    ) -> Self {
        // Counted **once per document**, not once per occurrence. A word repeated down one
        // page's navigation is one page's opinion, and counting it twenty times let a single
        // misspelling look as established as the right spelling on twenty different sites —
        // which is exactly the comparison the corrector makes.
        let mut freq: HashMap<String, u32> = HashMap::new();
        // The places of the country this engine is for, before anything crawled. Without them a
        // globally-shaped corpus decides that `alger` is a misspelling of `aller`.
        for w in xustive_tools::wilaya::WILAYAS.iter().flat_map(|w| {
            words(w.name_ar)
                .into_iter()
                .chain(words(w.name_fr))
                .chain(words(&deaccent_str(w.name_fr)))
        }) {
            let e = freq.entry(w).or_insert(0);
            *e = (*e).max(SEED_WEIGHT);
        }
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for text in texts {
            seen.clear();
            for w in words(&text) {
                if seen.insert(w.clone()) {
                    *freq.entry(w).or_insert(0) += 1;
                }
            }
        }
        // Past queries **reinforce** words the corpus already knows; they never introduce new
        // ones. A misspelled query still returns results — that is what typo tolerance is for —
        // so feeding query words in blind teaches the vocabulary every typo anyone ever typed,
        // and the corrector then defends the typo as a word. (Measured: seven test searches for
        // `tlemcan` gave it a frequency of fourteen, above the real `tlemcen`.)
        for q in queries {
            seen.clear();
            for w in words(&q) {
                if seen.insert(w.clone()) {
                    if let Some(n) = freq.get_mut(&w) {
                        *n += 2;
                    }
                }
            }
        }
        let mut shapes: HashMap<String, u32> = HashMap::new();
        let mut buckets: HashMap<(char, usize), Vec<(Arc<str>, Arc<str>)>> = HashMap::new();
        for (w, &n) in &freq {
            let sh = shape(w);
            if sh.is_empty() {
                continue;
            }
            *shapes.entry(sh.clone()).or_insert(0) += n;
            if n < MIN_FREQ {
                continue;
            }
            let Some(first) = sh.chars().next() else {
                continue;
            };
            let len = sh.chars().count();
            buckets
                .entry((first, len))
                .or_default()
                .push((Arc::from(sh.as_str()), Arc::from(w.as_str())));
        }
        Self {
            freq,
            shapes,
            buckets,
        }
    }

    pub fn len(&self) -> usize {
        self.freq.len()
    }

    pub fn is_empty(&self) -> bool {
        self.freq.is_empty()
    }

    /// The corrected query, when at least one token changes. The output keeps the reader's token
    /// order and the untouched tokens as typed.
    pub fn correct(&self, normalized_query: &str) -> Option<String> {
        if self.buckets.is_empty() {
            return None;
        }
        let mut changed = false;
        let out: Vec<String> = normalized_query
            .split_whitespace()
            .map(|tok| match self.correct_token(tok) {
                Some(c) => {
                    changed = true;
                    c
                }
                None => tok.to_string(),
            })
            .collect();
        changed.then(|| out.join(" "))
    }

    /// What the corrector sees for one token, for the operator endpoint and for tuning: the
    /// shape it compares on, how well the corpus knows it, and the candidates it weighed.
    pub fn explain(&self, tok: &str) -> serde_json::Value {
        let sh = shape(tok);
        let own = self.shapes.get(&sh).copied().unwrap_or(0);
        let mut cands: Vec<serde_json::Value> = Vec::new();
        let len = sh.chars().count();
        let max_d = if len <= 6 { 1 } else { 2 };
        if let Some(first) = sh.chars().next() {
            for l in len.saturating_sub(max_d)..=len + max_d {
                for (cand_shape, canonical) in self.buckets.get(&(first, l)).into_iter().flatten() {
                    let d = damerau(&sh, cand_shape);
                    if d == 0 || d > max_d {
                        continue;
                    }
                    cands.push(serde_json::json!({
                        "word": canonical.as_ref(),
                        "freq": self.freq.get(canonical.as_ref()).copied().unwrap_or(0),
                        "distance": d,
                    }));
                }
            }
        }
        cands.sort_by_key(|c| {
            (
                c["distance"].as_u64().unwrap_or(9),
                std::cmp::Reverse(c["freq"].as_u64().unwrap_or(0)),
            )
        });
        cands.truncate(8);
        serde_json::json!({
            "token": tok,
            "shape": sh,
            "own_frequency": own,
            "known": own >= KNOWN,
            "max_distance": max_d,
            "floor": MIN_FREQ.max(if own >= MIN_FREQ { own.saturating_mul(RATIO) } else { 0 }),
            "candidates": cands,
            "chosen": self.correct_token(tok),
        })
    }

    fn correct_token(&self, tok: &str) -> Option<String> {
        let len = tok.chars().count();
        if len < MIN_LEN
            || tok.chars().any(|c| c.is_ascii_digit())
            || !tok.chars().all(char::is_alphanumeric)
        {
            return None;
        }
        let tok_shape = shape(tok);
        let own = self.shapes.get(&tok_shape).copied().unwrap_or(0);
        // A word the corpus knows well is a word — including one typed without its accents,
        // which is why the test is on the shape. `algerien` is not a misspelling of `algerian`.
        if own >= KNOWN {
            return None;
        }
        let shape_len = tok_shape.chars().count();
        // Two edits only for a long word. On a short one, two edits reach words with nothing to
        // do with what was typed — `qwerty` is two from `query` — and the corrector would be
        // inventing rather than correcting.
        let max_d = if shape_len <= 6 { 1 } else { 2 };
        // The first letter is never changed. Typists miss, double and swap letters; they rarely
        // hit the wrong first key, and allowing it turns `wehran` (Oran, in Arabizi) into
        // `tehran` — a different place, offered with confidence.
        let first = tok_shape.chars().next()?;
        // A word seen in fewer documents than this is not a word of the corpus, it is noise —
        // the web is full of typos, and one page's misspelling must not defend itself. Above
        // it, the twenty-times bar applies.
        let own = if own >= MIN_FREQ { own } else { 0 };
        // Known → the high bar; unknown → the modest one.
        let ratio = if own == 0 { RATIO } else { RATIO_KNOWN };
        let floor = MIN_FREQ.max(own.saturating_mul(ratio));
        let mut best: Option<(u32, usize, &str)> = None; // (freq, distance, canonical)
        for l in shape_len.saturating_sub(max_d)..=shape_len + max_d {
            let Some(bucket) = self.buckets.get(&(first, l)) else {
                continue;
            };
            for (cand_shape, canonical) in bucket {
                if cand_shape.as_ref() == tok_shape {
                    continue;
                }
                let cf = self.freq.get(canonical.as_ref()).copied().unwrap_or(0);
                if cf < floor {
                    continue;
                }
                let d = damerau(&tok_shape, cand_shape);
                if d == 0 || d > max_d {
                    continue;
                }
                // Closer first, then more frequent.
                let better = match best {
                    None => true,
                    Some((bf, bd, _)) => d < bd || (d == bd && cf > bf),
                };
                if better {
                    best = Some((cf, d, canonical.as_ref()));
                }
            }
        }
        best.map(|(_, _, w)| w.to_string())
    }
}

/// A word's shape: Arabic orthographic variants folded and Latin accents removed, so the same
/// word written two ways compares equal.
pub fn shape(word: &str) -> String {
    xustive_text::fold(word).chars().map(deaccent).collect()
}

/// Every accented letter of a string replaced by its base, keeping everything else — so
/// `Béjaïa` also seeds `bejaia`, which is how people type it.
fn deaccent_str(s: &str) -> String {
    s.chars().map(deaccent).collect()
}

/// Latin accents to their base letter. A table rather than a Unicode decomposition: this needs
/// French, Spanish and the handful of forms Algerian pages actually use, and a table costs
/// nothing and pulls in nothing.
fn deaccent(c: char) -> char {
    match c {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        'ç' => 'c',
        'ñ' => 'n',
        _ => c,
    }
}

/// Words of a text, lowercased, letters and digits only, at least `MIN_LEN` long.
fn words(text: &str) -> Vec<String> {
    xustive_text::normalize(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= MIN_LEN)
        .map(str::to_string)
        .collect()
}

/// Optimal-string-alignment (restricted Damerau–Levenshtein) distance over chars.
pub fn damerau(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=m {
        d[0][j] = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[n][m]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab() -> Vocabulary {
        // Repeated so the words clear KNOWN; a real corpus has thousands of each.
        let titles = [
            "Couscous algérien: la recette traditionnelle",
            "Le couscous, plat national",
            "Couscous aux légumes",
            "Recette de couscous au poulet",
            "Couscous royal",
            "Alger, capitale de l'Algérie",
            "الجزائر العاصمة",
            "تاريخ الجزائر",
            "الجزائر اليوم",
            "خريطة الجزائر",
        ];
        let repeated: Vec<String> = titles
            .iter()
            .flat_map(|t| std::iter::repeat_n(t.to_string(), 4))
            .collect();
        Vocabulary::build(
            repeated.into_iter(),
            ["algérie foot".to_string()].into_iter(),
        )
    }

    #[test]
    fn a_transposition_and_a_missing_letter_are_corrected() {
        let v = vocab();
        assert_eq!(
            v.correct("couscuos algérien").as_deref(),
            Some("couscous algérien")
        );
        assert_eq!(v.correct("coucous").as_deref(), Some("couscous"));
        assert_eq!(v.correct("الجزائئر").as_deref(), Some("الجزائر"));
    }

    #[test]
    fn known_words_numbers_and_short_tokens_are_left_alone() {
        let v = vocab();
        assert_eq!(v.correct("couscous algérien"), None);
        assert_eq!(v.correct("2026 alger"), None);
        assert_eq!(v.correct("al"), None);
    }

    #[test]
    fn a_word_typed_without_its_accents_is_not_a_misspelling() {
        // The shape test: `algerien` is `algérien` written on a phone keyboard, not a typo for
        // some other word the corpus happens to know better.
        let v = vocab();
        assert_eq!(v.correct("couscous algerien"), None);
    }

    #[test]
    fn a_place_of_the_country_is_never_corrected_away() {
        // `alger` against `aller`: a corpus with more of the French web than of the Algerian one
        // makes the capital look like a typo. The wilaya seed and the known-word ratio both
        // stand in the way.
        let noise: Vec<String> = (0..200).map(|_| "il faut aller vite".to_string()).collect();
        let v = Vocabulary::build(noise.into_iter(), std::iter::empty());
        assert_eq!(v.correct("alger"), None);
        assert_eq!(v.correct("bejaia"), None);
    }

    #[test]
    fn the_first_letter_is_never_changed() {
        // `wehran` is Oran in Arabizi. Correcting it to `tehran` — a real, more frequent word
        // one edit away — would be confidently wrong, which is worse than saying nothing.
        let mut titles: Vec<String> = (0..30)
            .map(|_| "Tehran, capitale de l'Iran".to_string())
            .collect();
        titles.push("Oran".into());
        let v = Vocabulary::build(titles.into_iter(), std::iter::empty());
        assert_eq!(v.correct("wehran"), None);
    }

    #[test]
    fn distance_is_damerau() {
        assert_eq!(damerau("couscuos", "couscous"), 1);
        assert_eq!(damerau("abc", "abc"), 0);
        assert_eq!(damerau("abc", "axbyc"), 2);
    }
}
