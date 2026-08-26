//! Deciding which entity — if any — a query means.
//!
//! The governing rule is precision, and it is not a close call. A panel sits in the most trusted
//! space on the page, so a confident panel about the wrong thing is worse than no panel and much
//! worse than it looks: the reader has no reason to doubt it. Everything here is built to say
//! *nothing* rather than to guess.
//!
//! Pure and index-free. The caller does the searching; this decides what the results mean, so the
//! judgement can be tested against fixed candidate sets rather than a live index.

use crate::entity::Entity;

/// Shortest and longest query worth resolving. Below two characters there is nothing to match;
/// above sixty a person is describing rather than naming.
const MIN_LEN: usize = 2;
const MAX_LEN: usize = 60;
/// More words than this and it is a sentence, not a name.
const MAX_WORDS: usize = 8;

/// Openings that mark a query as a question rather than a name.
///
/// Carried over verbatim from the web-tier panel this replaces, including the Darija forms
/// (`كيفاش`, `علاش`, `وين`) that the Modern Standard list would miss entirely — the audience asks
/// in Darija and a gate that only knows MSA would let those through to a wrong panel.
const QUESTION_MARKERS: &[&str] = &[
    "how ",
    "what ",
    "why ",
    "when ",
    "where ",
    "who is ",
    "comment ",
    "pourquoi ",
    "quand ",
    "qu'est",
    "كيف",
    "كيفاش",
    "لماذا",
    "علاش",
    "وين",
    "شنو",
    "متى",
    "أين",
];

/// Whether a query is shaped like a name at all.
///
/// A cheap gate that runs before any lookup. A question belongs to the summariser — it wants a
/// paragraph — and a noun phrase belongs to the panel.
pub fn is_panel_shaped(query: &str) -> bool {
    let q = query.trim();
    if q.chars().count() < MIN_LEN || q.chars().count() > MAX_LEN {
        return false;
    }
    if q.contains('?') || q.contains('؟') {
        return false;
    }
    if q.split_whitespace().count() > MAX_WORDS {
        return false;
    }
    let padded = format!(" {} ", q.to_lowercase());
    !QUESTION_MARKERS.iter().any(|m| padded.contains(m))
}

/// One entity the index offered, with the signals used to judge it.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub entity: Entity,
    /// How many documents in our own crawled corpus mention this name. The signal that makes an
    /// Algeria-first engine behave like one: a name the Algerian web talks about outranks a
    /// same-named entity it has never mentioned.
    pub corpus_mentions: u32,
}

/// What the resolver decided.
#[derive(Debug, Clone)]
pub struct Resolution {
    pub entity: Entity,
    /// In `[0, 1]`. Not a probability — a margin, used only against [`MIN_CONFIDENCE`].
    pub confidence: f32,
    /// The runner-up, when it was close enough that picking silently would be dishonest. The panel
    /// offers it as "did you mean" rather than hiding the ambiguity.
    pub also: Option<Entity>,
}

/// Below this, render nothing.
///
/// Deliberately high. The cost of a missing panel is a reader who reads the results, which is what
/// they came for; the cost of a wrong one is a reader who believes something false.
pub const MIN_CONFIDENCE: f32 = 0.55;

/// How close the runner-up must be before the ambiguity is surfaced instead of swallowed.
const AMBIGUOUS_WITHIN: f32 = 0.15;

/// Score one candidate against the query.
///
/// Every term is bounded and the total is clamped, so no single signal can carry a candidate over
/// the floor alone — an entity with an enormous sitelink count still needs to match the name.
fn score(query: &str, c: &Candidate) -> f32 {
    let q = normalise(query);
    let names: Vec<String> = c
        .entity
        .names
        .all_strings()
        .iter()
        .map(|n| normalise(n))
        .collect();

    // The dominant signal: did they type this thing's name, exactly?
    let exact = names.iter().any(|n| *n == q);
    // A prefix match covers "Riyad Mahrez" typed as "Mahrez" and little else.
    let prefix = !exact
        && names
            .iter()
            .any(|n| n.starts_with(&q) || q.starts_with(n.as_str()));

    let mut s = if exact {
        0.70
    } else if prefix {
        0.35
    } else {
        // The index matched something — a description, a fuzzy name — but not the name itself.
        0.10
    };

    // Prominence and corpus agreement are tie-breakers, capped low on purpose. They decide
    // *which* Oran, never *whether* this is Oran.
    s += (c.entity.prominence as f32 / 200.0).min(0.15);
    s += (c.corpus_mentions as f32 / 50.0).min(0.15);

    s.clamp(0.0, 1.0)
}

/// Lower-case, strip the Arabic definite article and common punctuation, and collapse whitespace.
///
/// `الجزائر` and `الجزائر ` are the same name; so are `Oran` and `oran`. The definite article is
/// stripped because Arabic writes it attached, so `الجزائر` typed without it never matches a label
/// that carries it.
fn normalise(s: &str) -> String {
    let lowered = s.trim().to_lowercase();
    let stripped = lowered.strip_prefix("ال").unwrap_or(&lowered);
    stripped
        .chars()
        .filter(|c| !matches!(c, '.' | ',' | '"' | '\'' | '«' | '»' | '(' | ')'))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Choose an entity, or decline.
///
/// Candidates arrive in whatever order the index ranked them; this re-ranks by the signals above
/// and refuses anything under [`MIN_CONFIDENCE`].
pub fn choose(query: &str, candidates: &[Candidate]) -> Option<Resolution> {
    if !is_panel_shaped(query) || candidates.is_empty() {
        return None;
    }
    // Thin entities are excluded before scoring rather than penalised within it. A penalty is a
    // weight that a strong enough name match plus prominence can outvote, and "a bare label is not
    // knowledge" is meant as a guarantee, not a preference — the first version scored it and a
    // maximally-prominent bare label still won.
    let mut scored: Vec<(f32, &Candidate)> = candidates
        .iter()
        .filter(|c| c.entity.is_renderable())
        .map(|c| (score(query, c), c))
        .collect();
    if scored.is_empty() {
        return None;
    }
    // Descending, with the id as a tie-break so an identical pair resolves the same way every
    // time rather than depending on the order the index happened to return.
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.entity.id.cmp(&b.1.entity.id))
    });

    let (confidence, best) = scored[0];
    if confidence < MIN_CONFIDENCE {
        return None;
    }
    let also = scored
        .get(1)
        .filter(|(s, _)| confidence - s < AMBIGUOUS_WITHIN)
        .map(|(_, c)| c.entity.clone());

    Some(Resolution {
        entity: best.entity.clone(),
        confidence,
        also,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::Names;
    use crate::kind::Kind;

    fn candidate(id: &str, label: &str, prominence: u32, mentions: u32) -> Candidate {
        let mut e = Entity::new(id, Kind::Place, 0);
        e.names = Names {
            labels: vec![("en".into(), label.into())],
            aliases: vec![],
        };
        e.descriptions = vec![("en".into(), "a place".into())];
        e.prominence = prominence;
        Candidate {
            entity: e,
            corpus_mentions: mentions,
        }
    }

    #[test]
    fn a_question_gets_no_panel_in_any_of_the_four_languages() {
        // A question wants a paragraph, which is the summariser's job. Darija is in the list
        // because the audience asks in Darija and an MSA-only gate would let those through.
        for q in [
            "how to cook couscous",
            "pourquoi le ciel est bleu",
            "كيف أطبخ الكسكس",
            "كيفاش نطيب الكسكس",
            "علاش السما زرقة",
            "وين رانا",
            "what is oran?",
        ] {
            assert!(!is_panel_shaped(q), "{q} should not get a panel");
        }
    }

    #[test]
    fn a_name_is_panel_shaped() {
        for q in ["Oran", "وهران", "The Battle of Algiers", "Riyad Mahrez"] {
            assert!(is_panel_shaped(q), "{q} should be panel shaped");
        }
    }

    #[test]
    fn a_sentence_is_not_a_name() {
        assert!(!is_panel_shaped(
            "the best restaurants to visit in oran this summer"
        ));
        assert!(!is_panel_shaped("a"));
        assert!(!is_panel_shaped(&"x".repeat(61)));
    }

    #[test]
    fn an_exact_name_beats_a_far_more_famous_near_match() {
        // The signal weighting that matters: prominence decides *which* Oran, never *whether* this
        // is Oran. A hugely prominent entity that merely resembles the query must not win.
        let hits = vec![
            candidate("Q_FAMOUS", "Oran Province", 5000, 900),
            candidate("Q_EXACT", "Oran", 115, 3),
        ];
        let r = choose("Oran", &hits).unwrap();
        assert_eq!(r.entity.id, "Q_EXACT");
    }

    #[test]
    fn the_corpus_breaks_a_tie_towards_what_algeria_talks_about() {
        // Two entities, one name. The one the crawled Algerian web mentions is the one a reader
        // here almost certainly means.
        let hits = vec![
            candidate("Q_ELSEWHERE", "Constantine", 200, 0),
            candidate("Q_ALGERIA", "Constantine", 200, 60),
        ];
        assert_eq!(choose("Constantine", &hits).unwrap().entity.id, "Q_ALGERIA");
    }

    #[test]
    fn a_close_runner_up_is_offered_rather_than_swallowed() {
        let hits = vec![
            candidate("Q_A", "Constantine", 200, 10),
            candidate("Q_B", "Constantine", 200, 8),
        ];
        let r = choose("Constantine", &hits).unwrap();
        assert!(r.also.is_some(), "a near-identical rival must be surfaced");
    }

    #[test]
    fn a_clear_winner_offers_no_alternative() {
        let hits = vec![
            candidate("Q_A", "Constantine", 400, 90),
            candidate("Q_B", "Something Else Entirely", 1, 0),
        ];
        assert!(choose("Constantine", &hits).unwrap().also.is_none());
    }

    #[test]
    fn a_weak_match_renders_nothing_at_all() {
        // The floor doing its job: the index will always return *something*, and something is not
        // the same as an answer.
        let hits = vec![candidate("Q_X", "Utterly Unrelated Thing", 3, 0)];
        assert!(choose("Oran", &hits).is_none());
    }

    #[test]
    fn a_thin_entity_does_not_win_a_panel_it_cannot_fill() {
        let mut bare = candidate("Q_BARE", "Oran", 900, 900);
        bare.entity.descriptions.clear();
        bare.entity.facts.clear();
        bare.entity.extracts.clear();
        assert!(choose("Oran", &[bare]).is_none());
    }

    #[test]
    fn the_arabic_definite_article_does_not_break_an_exact_match() {
        // Arabic attaches the article, so a reader typing الجزائر must match a label stored as
        // الجزائر and one stored without it.
        let mut e = Entity::new("Q262", Kind::Place, 0);
        e.names = Names {
            labels: vec![("ar".into(), "الجزائر".into())],
            aliases: vec![],
        };
        e.descriptions = vec![("ar".into(), "بلد".into())];
        let hits = vec![Candidate {
            entity: e,
            corpus_mentions: 40,
        }];
        assert!(choose("جزائر", &hits).is_some());
        assert!(choose("الجزائر", &hits).is_some());
    }

    #[test]
    fn an_identical_pair_resolves_the_same_way_every_time() {
        // Without a deterministic tie-break the winner would depend on the order the index
        // happened to return, and the panel would flicker between two entities on reload.
        let hits = vec![
            candidate("Q_B", "Setif", 100, 5),
            candidate("Q_A", "Setif", 100, 5),
        ];
        let first = choose("Setif", &hits).unwrap().entity.id;
        let reversed: Vec<Candidate> = hits.iter().rev().cloned().collect();
        assert_eq!(first, choose("Setif", &reversed).unwrap().entity.id);
    }

    #[test]
    fn no_candidates_means_no_panel_rather_than_an_error() {
        assert!(choose("Oran", &[]).is_none());
    }
}
