//! Deduplication quality evaluation (M2-T05.9).
//!
//! The gate: over 500 duplicate pairs and 500 distinct pairs, precision ≥ 0.95 and recall ≥ 0.85.
//!
//! # What is measured
//!
//! The dedup stack calls two texts the same when their `content_hash` matches (byte-identical body)
//! or their SimHash is within [`NEAR_DISTANCE`] bits (reworded body). This runs that exact
//! classifier over generated pairs whose ground truth is known.
//!
//! - **Recall** is the half SimHash exists for. Exact hashing alone recalls only byte-identical
//!   duplicates; the reworded ones — a syndicated story with a few words changed — are what the
//!   near-duplicate band must catch. A low recall means those slip through and the same story shows
//!   up twice in results.
//! - **Precision** is the half that keeps it honest. The dangerous false positive is two *distinct*
//!   articles on the same topic — two different reports on the same BAC results — which share
//!   vocabulary and so drift close in SimHash. Collapsing those loses a real document, so the
//!   distinct set deliberately includes same-topic pairs, not just unrelated ones.
//!
//! # On generation
//!
//! The pairs are generated, not hand-labelled, so this measures the classifier against a model of
//! how duplicates and distinct articles differ — not against the real web. It is a regression guard
//! and a sanity check on the SimHash distance threshold, not a claim about production precision;
//! that needs the real labelled set the milestone's exit gate asks for. Generation is deterministic
//! (a simple LCG, no `rand`) so the number does not wander run to run.

use xustive_core::hash;
use xustive_ingest::simhash_index::is_near;

/// The dedup stack's verdict: are these the same document?
fn are_duplicate(a: &str, b: &str) -> bool {
    if hash::content_hash(a) == hash::content_hash(b) {
        return true;
    }
    match (hash::simhash(a), hash::simhash(b)) {
        (Some(x), Some(y)) => is_near(x, y),
        _ => false,
    }
}

/// A tiny deterministic PRNG, so the corpus is fixed across runs without pulling in `rand`.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 16
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[(self.next() as usize) % xs.len()]
    }
    fn chance(&mut self, n: u64) -> bool {
        self.next() % n == 0
    }
}

/// Sentence fragments by topic. A base article is several fragments from one topic, so two articles
/// on the same topic share vocabulary — the hard case for precision.
fn topics() -> Vec<Vec<&'static str>> {
    // Each topic has ~11 fragments so two articles on one topic can be built from different
    // subsets — sharing vocabulary without being reorderings of the same sentences.
    vec![
        vec![
            "the ministry of energy announced the launch of a new integrated phosphate project",
            "the investment is estimated at seven billion dollars over the coming years",
            "officials expect the project to create thousands of direct and indirect jobs",
            "the works will begin in the wilaya of tebessa according to the statement",
            "the minister confirmed that financing is fully secured by national partners",
            "environmental studies were completed ahead of the ground breaking ceremony",
            "the site was chosen for its proximity to existing rail and road links",
            "local authorities pledged support for the recruitment of skilled workers",
            "the first phase focuses on extraction before processing capacity is added",
            "exports are expected to begin once the second phase reaches completion",
            "a training centre will open nearby to prepare technicians for the plant",
        ],
        vec![
            "the ministry of education announced the baccalaureate pass rate for this year",
            "the wilaya of tizi ouzou recorded the highest success rate nationally",
            "results are available through the official online portal for candidates",
            "the minister thanked teachers and supervisors for their efforts",
            "the session passed without any major incident across examination centres",
            "candidates with disabilities were given additional time and support",
            "invigilators reported smooth conduct in the vast majority of centres",
            "appeals may be lodged within a set window through the same portal",
            "the ministry published statistics broken down by stream and region",
            "vocational tracks recorded gains over the previous academic year",
            "families gathered outside schools awaiting the announcement",
        ],
        vec![
            "the national football team qualified for the next round of the competition",
            "the side won the match by two goals to one in front of a full stadium",
            "the coach said the lineup will see changes for the coming fixture",
            "supporters celebrated across several cities late into the evening",
            "the federation praised the players for their determination on the pitch",
            "the goalkeeper was named man of the match by the broadcast panel",
            "the result lifted the side above its rivals in the group standings",
            "injuries to two defenders will be assessed before the next call up",
            "ticket sales for the following fixture opened the next morning",
            "the captain dedicated the win to supporters who travelled to the game",
            "analysts noted the midfield controlled possession throughout",
        ],
        vec![
            "algerie telecom announced the connection of new subscribers to the fibre network",
            "the operation covers urban and peri urban areas in a first phase",
            "the operator said speeds will improve markedly for connected households",
            "the rollout is part of a wider plan to expand digital infrastructure",
            "technical teams are working to complete installations before the year end",
            "a call centre was set up to handle subscription requests",
            "prices for the new tier were published on the operator website",
            "rural districts are scheduled for a later phase of the rollout",
            "the regulator welcomed the expansion of high speed access",
            "engineers laid several kilometres of cable in the first weeks",
            "the operator reported strong early demand from businesses",
        ],
        vec![
            "the water authority announced three new desalination plants entering service",
            "total production capacity will rise to cover demand in coastal cities",
            "the plants were built as part of a programme to secure drinking water",
            "the authority said reliance on rainfall will fall in the affected regions",
            "maintenance crews have been trained to operate the new facilities",
            "the plants use reverse osmosis to treat seawater at scale",
            "energy for the units is drawn partly from renewable sources",
            "the authority signed supply agreements with neighbouring wilayas",
            "storage reservoirs were expanded to buffer periods of peak demand",
            "water quality is monitored continuously by an automated system",
            "the programme aims to end seasonal shortages within two years",
        ],
        vec![
            "the customs service reported a rise in non hydrocarbon exports last year",
            "the value reached five billion dollars driven by industrial goods",
            "officials attributed the gain to improved competitiveness of local products",
            "several new markets were opened for agricultural exports in the period",
            "the ministry set a higher target for the coming year in its plan",
            "trade missions are planned to several african and european capitals",
            "a support fund was announced for small exporting enterprises",
            "customs procedures were simplified to speed clearance at ports",
            "the chamber of commerce welcomed the measures in a statement",
            "logistics costs remain a concern raised by several exporters",
            "the figures exclude re exports and transit shipments",
        ],
    ]
}

/// Build a base article at a realistic length — every fragment of the topic, in a shuffled order,
/// repeated so the body is 150+ tokens like a real news article rather than a headline. SimHash is
/// designed for document-length text; measuring it on a two-sentence stub understates it, because a
/// single edit is then a large fraction of a tiny body.
fn base_article(lcg: &mut Lcg, topic: &[&str]) -> String {
    // A distinct subset of the topic's fragments, so two articles on one topic share vocabulary but
    // are genuinely different stories — not reorderings of the same sentences. This is what makes
    // the same-topic distinct pairs a real precision test rather than a disguised duplicate.
    let mut frags: Vec<&str> = topic.to_vec();
    for i in (1..frags.len()).rev() {
        let j = (lcg.next() as usize) % (i + 1);
        frags.swap(i, j);
    }
    let take = 6.min(frags.len());
    let chosen = &frags[..take];
    // Two passes for a document-length body.
    let mut parts: Vec<&str> = Vec::new();
    for _ in 0..2 {
        parts.extend_from_slice(chosen);
    }
    parts.join(". ")
}

/// A *duplicate* of an article: the same body, reworded the way a syndication does — a few filler
/// words inserted, small punctuation and whitespace differences. Not a rewrite; the same story.
fn reword(lcg: &mut Lcg, article: &str) -> String {
    const FILLER: &[&str] = &[
        "reportedly",
        "meanwhile",
        "notably",
        "in addition",
        "moreover",
    ];
    let mut out = String::new();
    for word in article.split_whitespace() {
        out.push_str(word);
        out.push(' ');
        // A rare filler insertion — about one per article. A genuine near-duplicate ('same story',
        // distance <= 3) is a body with a trivial edit: a dropped word, an added qualifier. A
        // heavier rewrite lands in the 4-8 cluster band, which the project defines as related but
        // not duplicate (T05.6), so modelling it as a duplicate here would be testing the wrong
        // thing — measuring clusters against the duplicate threshold.
        if lcg.chance(200) {
            out.push_str(lcg.pick(FILLER));
            out.push(' ');
        }
    }
    out.trim().to_string()
}

struct Score {
    tp: usize,
    fp: usize,
    fn_: usize,
}

impl Score {
    fn precision(&self) -> f64 {
        if self.tp + self.fp == 0 {
            1.0
        } else {
            self.tp as f64 / (self.tp + self.fp) as f64
        }
    }
    fn recall(&self) -> f64 {
        if self.tp + self.fn_ == 0 {
            1.0
        } else {
            self.tp as f64 / (self.tp + self.fn_) as f64
        }
    }
}

fn evaluate() -> Score {
    let mut lcg = Lcg(0x5EED_1234_ABCD_0001);
    let topics = topics();
    let (mut tp, mut fp, mut fn_) = (0usize, 0usize, 0usize);

    // 500 duplicate pairs: an article and a reworded copy of it.
    for _ in 0..500 {
        let topic = lcg.pick(&topics).clone();
        let a = base_article(&mut lcg, &topic);
        let b = reword(&mut lcg, &a);
        if are_duplicate(&a, &b) {
            tp += 1;
        } else {
            fn_ += 1;
        }
    }

    // 500 distinct pairs, half of them on the *same* topic — the hard negatives.
    for i in 0..500 {
        let ta = lcg.pick(&topics).clone();
        let a = base_article(&mut lcg, &ta);
        let tb = if i % 2 == 0 {
            ta.clone() // same topic: shared vocabulary, close SimHash — the precision hazard
        } else {
            lcg.pick(&topics).clone()
        };
        let b = base_article(&mut lcg, &tb);
        // Guard against the generator producing the identical body by chance; a real distinct pair
        // is what this measures.
        if a == b {
            continue;
        }
        if are_duplicate(&a, &b) {
            fp += 1;
        }
    }

    Score { tp, fp, fn_ }
}

#[test]
fn dedup_meets_the_precision_and_recall_gate() {
    let s = evaluate();
    println!(
        "dedup quality: precision {:.3} ({} tp, {} fp), recall {:.3} ({} tp, {} fn)",
        s.precision(),
        s.tp,
        s.fp,
        s.recall(),
        s.tp,
        s.fn_
    );
    assert!(
        s.precision() >= 0.95,
        "precision {:.3} below 0.95 — distinct articles are being collapsed, which loses documents",
        s.precision()
    );
    assert!(
        s.recall() >= 0.85,
        "recall {:.3} below 0.85 — reworded duplicates are slipping through as separate documents",
        s.recall()
    );
}

#[test]
#[ignore]
fn distance_distribution() {
    use std::collections::BTreeMap;
    let mut lcg = Lcg(0x5EED_1234_ABCD_0001);
    let topics = topics();
    let mut hist: BTreeMap<u32, usize> = BTreeMap::new();
    let mut none = 0;
    for _ in 0..500 {
        let topic = lcg.pick(&topics).clone();
        let a = base_article(&mut lcg, &topic);
        let b = reword(&mut lcg, &a);
        match (hash::simhash(&a), hash::simhash(&b)) {
            (Some(x), Some(y)) => {
                *hist.entry(hash::hamming(x, y)).or_insert(0) += 1;
            }
            _ => none += 1,
        }
    }
    eprintln!("none={none}");
    for (d, c) in hist {
        eprintln!("dist {d}: {c}");
    }
}
