//! The query mix — what a load test actually sends.
//!
//! A load test is only as honest as its inputs. Firing one query a million times measures a cache,
//! not the engine; firing random strings measures the empty-result path. This is a weighted, mostly
//! Algerian mix across the four languages, so the load looks like real traffic: wilaya names, public
//! services (Sonelgaz, CNAS), health and how-to queries, French and Darija phrasings of the same
//! intents, and a few long-tail misses that a real stream always contains.
//!
//! Selection is deterministic given a seed (a small LCG), so a run is reproducible and the picker is
//! unit-testable — the same seed yields the same sequence, which matters when comparing two runs.

/// One query and how often it should appear, relative to the others.
pub struct WeightedQuery {
    pub text: &'static str,
    /// BCP-47-ish tag, for reporting the language distribution actually exercised.
    pub lang: &'static str,
    pub weight: u32,
}

/// The default mix. Weights are relative; heavier = more frequent. Arabic and Darija dominate, as
/// real traffic does, with French second and English a minority.
pub const DEFAULT_QUERIES: &[WeightedQuery] = &[
    // High-frequency Arabic — services, cities, everyday needs.
    WeightedQuery {
        text: "الطقس في الجزائر",
        lang: "ar",
        weight: 10,
    },
    WeightedQuery {
        text: "مواقيت الصلاة وهران",
        lang: "ar",
        weight: 9,
    },
    WeightedQuery {
        text: "سونلغاز فاتورة",
        lang: "ar",
        weight: 7,
    },
    WeightedQuery {
        text: "نتائج البكالوريا",
        lang: "ar",
        weight: 8,
    },
    WeightedQuery {
        text: "سعر الدولار اليوم",
        lang: "ar",
        weight: 7,
    },
    WeightedQuery {
        text: "قسنطينة أخبار",
        lang: "ar",
        weight: 5,
    },
    WeightedQuery {
        text: "cnas تعويض",
        lang: "ar",
        weight: 5,
    },
    WeightedQuery {
        text: "باراسيتامول جرعة",
        lang: "ar",
        weight: 4,
    },
    // Darija phrasings — the same intents, spoken.
    WeightedQuery {
        text: "وين نلقى خدمة",
        lang: "ary",
        weight: 5,
    },
    WeightedQuery {
        text: "كيفاش نخلص الضو",
        lang: "ary",
        weight: 4,
    },
    WeightedQuery {
        text: "شحال سعر البنزين",
        lang: "ary",
        weight: 4,
    },
    // French — the other everyday register.
    WeightedQuery {
        text: "météo alger",
        lang: "fr",
        weight: 8,
    },
    WeightedQuery {
        text: "horaires priere oran",
        lang: "fr",
        weight: 6,
    },
    WeightedQuery {
        text: "sonelgaz facture en ligne",
        lang: "fr",
        weight: 6,
    },
    WeightedQuery {
        text: "resultats bac 2025",
        lang: "fr",
        weight: 6,
    },
    WeightedQuery {
        text: "prix carburant algerie",
        lang: "fr",
        weight: 5,
    },
    WeightedQuery {
        text: "cnas remboursement",
        lang: "fr",
        weight: 4,
    },
    WeightedQuery {
        text: "université constantine inscription",
        lang: "fr",
        weight: 3,
    },
    // English — a smaller share.
    WeightedQuery {
        text: "algeria visa requirements",
        lang: "en",
        weight: 3,
    },
    WeightedQuery {
        text: "paracetamol dosage",
        lang: "en",
        weight: 2,
    },
    // Long-tail misses — every real stream has queries that return little.
    WeightedQuery {
        text: "قطع غيار طوموبيل مستعملة سطيف",
        lang: "ary",
        weight: 2,
    },
    WeightedQuery {
        text: "obscure nonexistent term xyzzy",
        lang: "en",
        weight: 1,
    },
];

/// A prepared mix: the queries plus a cumulative-weight table for O(log n) weighted selection.
pub struct Mix {
    queries: Vec<(String, String)>, // (text, lang)
    cumulative: Vec<u32>,
    total: u32,
}

impl Mix {
    /// Build from the default corpus.
    pub fn default_mix() -> Self {
        Self::from_weighted(DEFAULT_QUERIES.iter().map(|q| (q.text, q.lang, q.weight)))
    }

    /// Build from an iterator of `(text, lang, weight)`. Zero-weight entries are dropped; an empty
    /// or all-zero input yields an empty mix (which [`Self::pick`] handles by returning `""`).
    pub fn from_weighted<'a>(it: impl Iterator<Item = (&'a str, &'a str, u32)>) -> Self {
        let mut queries = Vec::new();
        let mut cumulative = Vec::new();
        let mut total = 0u32;
        for (text, lang, weight) in it {
            if weight == 0 {
                continue;
            }
            total = total.saturating_add(weight);
            queries.push((text.to_string(), lang.to_string()));
            cumulative.push(total);
        }
        Self {
            queries,
            cumulative,
            total,
        }
    }

    #[allow(dead_code)] // used in tests and by callers building custom mixes
    pub fn len(&self) -> usize {
        self.queries.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.queries.is_empty()
    }

    /// Pick a query, advancing `seed`. Deterministic: the same seed sequence yields the same picks.
    pub fn pick(&self, seed: &mut u64) -> &str {
        if self.total == 0 {
            return "";
        }
        let r = next_random(seed) % self.total as u64;
        // First cumulative bound strictly greater than r.
        let idx = self.cumulative.partition_point(|&c| (c as u64) <= r);
        &self.queries[idx.min(self.queries.len() - 1)].0
    }

    /// A search prefix (first `n` characters of a picked query), for the suggest scenario — suggest
    /// fires on partial input, not whole queries.
    pub fn pick_prefix(&self, seed: &mut u64, n: usize) -> String {
        self.pick(seed).chars().take(n.max(1)).collect()
    }
}

/// A tiny LCG (PCG-style multiplier). Not cryptographic — it only needs a well-spread, reproducible
/// sequence, and it avoids a dependency and the non-determinism of a global RNG.
fn next_random(seed: &mut u64) -> u64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    // Return the high bits, which have the best statistical quality in an LCG.
    (*seed >> 16) ^ *seed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_mix_is_non_empty_and_multilingual() {
        let m = Mix::default_mix();
        assert!(m.len() >= 15);
        let langs: std::collections::HashSet<_> = DEFAULT_QUERIES.iter().map(|q| q.lang).collect();
        for l in ["ar", "ary", "fr", "en"] {
            assert!(langs.contains(l), "mix missing language {l}");
        }
    }

    #[test]
    fn selection_is_deterministic_for_a_seed() {
        let m = Mix::default_mix();
        let mut a = 42;
        let mut b = 42;
        let seq_a: Vec<_> = (0..50).map(|_| m.pick(&mut a).to_string()).collect();
        let seq_b: Vec<_> = (0..50).map(|_| m.pick(&mut b).to_string()).collect();
        assert_eq!(seq_a, seq_b, "same seed must give the same sequence");
    }

    #[test]
    fn different_seeds_diverge() {
        let m = Mix::default_mix();
        let mut a = 1;
        let mut b = 999;
        let seq_a: Vec<_> = (0..50).map(|_| m.pick(&mut a).to_string()).collect();
        let seq_b: Vec<_> = (0..50).map(|_| m.pick(&mut b).to_string()).collect();
        assert_ne!(seq_a, seq_b);
    }

    #[test]
    fn weighting_is_respected() {
        // "heavy" has 100x the weight of "light" — over many draws it must dominate, proving the
        // cumulative-weight selection actually weights rather than picking uniformly.
        let m = Mix::from_weighted([("heavy", "x", 100u32), ("light", "x", 1u32)].into_iter());
        let mut seed = 7;
        let mut heavy = 0;
        for _ in 0..10_000 {
            if m.pick(&mut seed) == "heavy" {
                heavy += 1;
            }
        }
        assert!(
            heavy > 9_500,
            "heavy won only {heavy}/10000 — weighting is off"
        );
    }

    #[test]
    fn an_empty_mix_yields_empty_string() {
        let m = Mix::from_weighted(std::iter::empty());
        assert!(m.is_empty());
        let mut seed = 1;
        assert_eq!(m.pick(&mut seed), "");
    }

    #[test]
    fn a_prefix_is_a_bounded_head_of_a_query() {
        let m = Mix::from_weighted([("météo alger", "fr", 1u32)].into_iter());
        let mut seed = 3;
        let p = m.pick_prefix(&mut seed, 4);
        assert_eq!(p.chars().count(), 4);
        assert!("météo alger".starts_with(&p));
    }
}
