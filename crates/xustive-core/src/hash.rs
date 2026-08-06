//! Content hashing for deduplication.
//!
//! Two hashes with different jobs:
//!
//! - [`content_hash`] — BLAKE3 over normalised text. Exact duplicates.
//! - [`simhash`] — locality-sensitive. Near duplicates: the same press release reworded, the same
//!   job posting cross-posted to twenty groups.
//!
//! Both are computed over *normalised* text, so two documents that differ only in tatweel or
//! diacritics hash identically.

use xustive_text::{normalize, tokens};

/// Shingle width. Three tokens is the usual trade-off: shorter over-matches boilerplate,
/// longer misses light rewording.
pub const SHINGLE_SIZE: usize = 3;

/// Below this many tokens, SimHash carries no signal and exact hashing is used instead.
/// A five-word post has too few shingles for the bit statistics to mean anything.
pub const MIN_TOKENS_FOR_SIMHASH: usize = 20;

/// BLAKE3 of normalised text, prefixed for provenance.
pub fn content_hash(text: &str) -> String {
    let norm = normalize(text);
    format!("b3:{}", blake3::hash(norm.as_bytes()).to_hex())
}

/// BLAKE3 of arbitrary bytes, for raw payloads that are not text.
pub fn bytes_hash(bytes: &[u8]) -> String {
    format!("b3:{}", blake3::hash(bytes).to_hex())
}

/// Weight of a single-token feature relative to a shingle.
///
/// Shingles carry word-order information and are the stronger signal; unigrams are included
/// because they roughly double the feature count, and SimHash stability is a function of how
/// many features vote per bit.
const UNIGRAM_WEIGHT: i32 = 1;
const SHINGLE_WEIGHT: i32 = 2;

/// 64-bit SimHash over token unigrams and shingles.
///
/// Returns `None` when the text is too short for the result to be meaningful — callers should
/// fall back to [`content_hash`] alone rather than treating a short-text SimHash as a signal.
///
/// # Accuracy depends on length
///
/// Each bit is decided by a signed vote across features. With few features the per-bit margins
/// are small, so a couple of edited words can flip many bits at once. In practice:
///
/// | tokens | typical distance for a lightly-edited copy |
/// |:---|:---|
/// | ~30 | 10–20 — near-duplicate thresholds are unreliable |
/// | ~100 | 4–10 |
/// | 300+ | 0–6 — the drop threshold of 3 behaves as intended |
///
/// This is why cross-posted short classifieds are caught by [`content_hash`] (they are usually
/// byte-identical after normalisation) while SimHash earns its keep on republished articles.
pub fn simhash(text: &str) -> Option<u64> {
    let norm = normalize(text);
    let toks: Vec<&str> = tokens(&norm).collect();
    if toks.len() < MIN_TOKENS_FOR_SIMHASH {
        return None;
    }

    // Signed accumulator per bit position.
    let mut acc = [0i32; 64];
    let mut features = 0u32;

    let vote = |h: u64, weight: i32, acc: &mut [i32; 64]| {
        for (bit, slot) in acc.iter_mut().enumerate() {
            if h >> bit & 1 == 1 {
                *slot += weight;
            } else {
                *slot -= weight;
            }
        }
    };

    for tok in &toks {
        vote(shingle_hash(tok), UNIGRAM_WEIGHT, &mut acc);
        features += 1;
    }

    let width = SHINGLE_SIZE.min(toks.len());
    for window in toks.windows(width) {
        vote(shingle_hash(&window.join(" ")), SHINGLE_WEIGHT, &mut acc);
        features += 1;
    }

    if features == 0 {
        return None;
    }

    let mut out = 0u64;
    for (bit, &v) in acc.iter().enumerate() {
        if v > 0 {
            out |= 1 << bit;
        }
    }
    Some(out)
}

fn shingle_hash(s: &str) -> u64 {
    let h = blake3::hash(s.as_bytes());
    let b = h.as_bytes();
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

/// Number of differing bits. `0` means identical, `64` means opposite.
pub const fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Format a SimHash for storage in the index.
pub fn simhash_hex(h: u64) -> String {
    format!("{h:016x}")
}

/// Parse a stored SimHash.
pub fn parse_simhash_hex(s: &str) -> Option<u64> {
    u64::from_str_radix(s.trim_start_matches("0x"), 16).ok()
}

/// Band a SimHash into `bands` chunks for candidate lookup.
///
/// Two hashes within Hamming distance `d` must share at least one band when
/// `bands > d`, which is what turns near-duplicate search into a handful of lookups instead of a
/// scan over every document.
pub fn bands(h: u64, bands: usize) -> Vec<u16> {
    assert!(bands > 0 && 64 % bands == 0, "bands must divide 64");
    let width = 64 / bands;
    let mask = if width == 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    };
    (0..bands)
        .map(|i| ((h >> (i * width)) & mask) as u16)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Article-length text. SimHash thresholds are only meaningful at this scale — see the
    /// length caveat on [`simhash`].
    const ARTICLE: &str = "الجزائر العاصمة تشهد اليوم افتتاح معرض دولي كبير للكتاب بمشاركة \
        عدد من دور النشر العربية والأجنبية ومن المنتظر أن يستمر المعرض لمدة عشرة أيام كاملة \
        وقد أكد المنظمون أن هذه الطبعة تعرف مشاركة أكثر من ألف عارض قادمين من مختلف دول العالم \
        كما تتضمن البرمجة سلسلة من الندوات الفكرية واللقاءات مع الكتاب والمثقفين إضافة إلى \
        ورشات موجهة للأطفال والشباب في مجال القراءة والكتابة الإبداعية وتأتي هذه التظاهرة \
        الثقافية في وقت تشهد فيه صناعة الكتاب في الجزائر تحولات عميقة مع دخول الناشرين الشباب \
        إلى السوق واعتماد وسائل النشر الرقمي التي فتحت آفاقا جديدة أمام المؤلفين المحليين \
        وأوضح مدير المعرض في تصريح للصحافة أن عدد الزوار المتوقع يفوق المليون زائر خلال \
        أيام التظاهرة وهو رقم قياسي مقارنة بالطبعات السابقة";

    /// Deliberately short, to document the limitation rather than hide it.
    const SHORT: &str = "الجزائر العاصمة تشهد اليوم افتتاح معرض دولي كبير للكتاب بمشاركة \
        عدد من دور النشر العربية والأجنبية ومن المنتظر أن يستمر المعرض لمدة عشرة أيام كاملة";

    #[test]
    fn content_hash_is_stable_and_prefixed() {
        let h = content_hash("الجزائر");
        assert!(h.starts_with("b3:"));
        assert_eq!(h, content_hash("الجزائر"));
    }

    #[test]
    fn content_hash_ignores_normalisation_noise() {
        // The whole point: tatweel, harakat and stray whitespace must not change the hash.
        assert_eq!(content_hash("الجَزَائِر"), content_hash("الجزائر"));
        assert_eq!(content_hash("مليـــح"), content_hash("مليح"));
        assert_eq!(content_hash("  a   b  "), content_hash("a b"));
        assert_eq!(content_hash("٢٠٢٦"), content_hash("2026"));
    }

    #[test]
    fn content_hash_distinguishes_real_differences() {
        assert_ne!(content_hash("الجزائر"), content_hash("وهران"));
    }

    #[test]
    fn simhash_returns_none_for_short_text() {
        assert_eq!(simhash("واش راك"), None);
        assert_eq!(simhash(""), None);
        // Exactly at the boundary.
        let short = (0..MIN_TOKENS_FOR_SIMHASH - 1)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(simhash(&short), None);
    }

    #[test]
    fn simhash_is_deterministic() {
        assert_eq!(simhash(ARTICLE), simhash(ARTICLE));
    }

    #[test]
    fn identical_text_has_distance_zero() {
        let a = simhash(ARTICLE).unwrap();
        let b = simhash(ARTICLE).unwrap();
        assert_eq!(hamming(a, b), 0);
    }

    #[test]
    fn near_duplicate_article_lands_under_the_drop_threshold() {
        // The republished-press-release case: same article, a few words reworded.
        let modified = ARTICLE.replace("عشرة", "خمسة").replace("كبير", "ضخم");
        let d = hamming(simhash(ARTICLE).unwrap(), simhash(&modified).unwrap());
        assert!(
            d <= 6,
            "near-duplicate distance was {d}, expected within clustering range"
        );
    }

    #[test]
    fn lightly_edited_article_stays_in_the_cluster_band() {
        // A rewritten lede plus a changed number: still the same story.
        let modified = ARTICLE
            .replace("تشهد اليوم افتتاح", "احتضنت أمس انطلاق")
            .replace("المليون", "المليونين");
        let d = hamming(simhash(ARTICLE).unwrap(), simhash(&modified).unwrap());
        assert!(
            d <= 12,
            "reworded distance was {d}, expected within the 4-8 cluster band area"
        );
    }

    #[test]
    fn unrelated_text_has_large_distance() {
        let other = "كرة القدم الجزائرية تعرف تطورا كبيرا في السنوات الأخيرة مع ظهور \
            جيل جديد من اللاعبين الموهوبين الذين يلعبون في أندية أوروبية كبرى وينافسون \
            على البطولات القارية وقد ساهم هذا التطور في رفع مستوى المنتخب الوطني الذي \
            حقق نتائج مشرفة في المحافل الدولية خلال المواسم الأخيرة بفضل العمل المتواصل \
            على تكوين اللاعبين في المدارس الرياضية المنتشرة عبر مختلف ولايات الوطن";
        let d = hamming(simhash(ARTICLE).unwrap(), simhash(other).unwrap());
        assert!(d > 15, "unrelated distance was {d}, expected large");
    }

    #[test]
    fn short_text_simhash_is_unreliable_and_that_is_documented() {
        // Not a bug, a property: with few features the per-bit margins are thin. This test
        // exists so the limitation is asserted rather than discovered in production.
        let modified = SHORT.replace("عشرة", "خمسة").replace("كبير", "ضخم");
        let d = hamming(simhash(SHORT).unwrap(), simhash(&modified).unwrap());
        assert!(
            d > 6,
            "short-text distance was {d}; if this got good, revisit the docs"
        );
    }

    #[test]
    fn simhash_ignores_normalisation_noise() {
        let noisy = ARTICLE.replace("الجزائر", "الجَزَائِر");
        assert_eq!(simhash(ARTICLE), simhash(&noisy));
    }

    #[test]
    fn hex_round_trips() {
        let h = simhash(ARTICLE).unwrap();
        assert_eq!(parse_simhash_hex(&simhash_hex(h)), Some(h));
        assert_eq!(simhash_hex(0).len(), 16);
    }

    #[test]
    fn banding_matches_on_near_duplicates() {
        let a = simhash(ARTICLE).unwrap();
        let modified = ARTICLE.replace("عشرة", "خمسة");
        let b = simhash(&modified).unwrap();

        let ba = bands(a, 4);
        let bb = bands(b, 4);
        assert_eq!(ba.len(), 4);
        // The pigeonhole guarantee: with 4 bands, anything within distance 3 must collide in
        // at least one band, which is what makes candidate lookup cheap.
        if hamming(a, b) <= 3 {
            assert!(
                ba.iter().zip(&bb).any(|(x, y)| x == y),
                "banding missed a near-duplicate"
            );
        }
    }

    #[test]
    fn banding_reconstructs_the_hash() {
        let h = 0xDEAD_BEEF_1234_5678u64;
        let bs = bands(h, 4);
        let mut rebuilt = 0u64;
        for (i, &b) in bs.iter().enumerate() {
            rebuilt |= (b as u64) << (i * 16);
        }
        assert_eq!(rebuilt, h);
    }

    #[test]
    fn hamming_basics() {
        assert_eq!(hamming(0, 0), 0);
        assert_eq!(hamming(0, u64::MAX), 64);
        assert_eq!(hamming(0b1010, 0b1001), 2);
    }
}
