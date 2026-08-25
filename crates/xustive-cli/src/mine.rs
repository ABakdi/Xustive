//! `xustive-cli mine-synonyms` — data-driven synonym candidates (M7-T01.2 / T07.3).
//!
//! The expansion lexicon is hand-curated and small; this mines **candidates** for it from two
//! sources the engine already has:
//!
//! - **The corpus**: tokens that co-occur in document titles far more than chance (PMI). In this
//!   bilingual corpus the high-precision slice is the **cross-script** pair — an Arabic token and a
//!   Latin token that keep appearing in the same titles are usually the same name or the same word
//!   in two scripts (`الجزائر`/`algerie`), which is exactly what the lexicon exists to bridge.
//!   Same-script co-occurrence is dominated by collocations (phrase halves, not synonyms), so it is
//!   deliberately excluded rather than dumped on the reviewer.
//! - **Federated results (T07.3)**: a `calibrate` capture records SearXNG's hit titles per query;
//!   a query token co-occurring with external title tokens is the same signal, seeded by what the
//!   wider web says these queries mean.
//!
//! The output is a **review file** in the lexicon's own TSV format, written next to the curated
//! files but loaded by nothing: the `Expander` compiles in `entities.tsv` + `synonyms.tsv` only.
//! Candidates become live synonyms when a human moves them into those files (blocker B7 — the same
//! native-speaker review the curated entries already require) and reruns `make migrate`. Mining
//! proposes; it never promotes.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::Value;

use xustive_core::Config;
use xustive_search::MeiliClient;
use xustive_text::script::{self, Script};

pub struct MineOptions {
    /// A `calibrate` capture (`external-ref-*.jsonl`) to mine federated co-occurrence from.
    pub reference: Option<PathBuf>,
    /// Ceiling on corpus titles scanned.
    pub max_docs: u64,
    /// A pair must co-occur at least this often to be proposed.
    pub min_count: u32,
    pub out: PathBuf,
    /// Print the candidates without writing the file.
    pub dry_run: bool,
}

/// PMI floor for a candidate. ln-scale: 3.0 means the pair co-occurs ~20× more than independence
/// would predict — low enough to surface real translation pairs, high enough to drop the tokens
/// that merely share a news cycle.
const MIN_PMI: f64 = 3.0;

/// Tokens per title actually counted. Titles are short; anything longer is a feed artefact and
/// would quadratically inflate the pair counts.
const MAX_TOKENS_PER_TITLE: usize = 24;

/// Ceiling on proposed candidates. A reviewer reads a page, not a corpus.
const MAX_CANDIDATES: usize = 200;

#[derive(Default)]
struct Counts {
    titles: u64,
    /// Per-token title frequency.
    token: HashMap<String, u32>,
    /// Cross-script pair co-occurrence, keyed (arabic_token, latin_token).
    pair: HashMap<(String, String), PairEvidence>,
}

/// How many distinct domains a pair must appear on. One site's title template can repeat a pair
/// hundreds of times ("Sayings of the Prophet — sunnah.com" × every hadith page) — that is one
/// piece of evidence wearing a hundred hats, and the first dry run was dominated by exactly this.
const MIN_DOMAINS: usize = 2;

#[derive(Default)]
struct PairEvidence {
    count: u32,
    federated: u32,
    /// Hashes of the distinct domains this pair was seen on, capped — past the threshold the exact
    /// count stops mattering and an unbounded set per pair would swell the map for nothing.
    domains: Vec<u64>,
}

impl PairEvidence {
    fn note_domain(&mut self, domain_hash: u64) {
        if self.domains.len() < 8 && !self.domains.contains(&domain_hash) {
            self.domains.push(domain_hash);
        }
    }
}

struct CandidatePair {
    arabic: String,
    latin: String,
    count: u32,
    federated: u32,
    pmi: f64,
}

pub async fn run(client: &MeiliClient, config: &Config, opts: &MineOptions) -> Result<()> {
    let mut counts = Counts::default();

    // --- corpus titles ---------------------------------------------------------------------------
    let index = client.resolve(&config.search.documents_index).await?;
    let mut offset = 0u64;
    let page_size = 1000u64;
    while offset < opts.max_docs {
        let limit = page_size.min(opts.max_docs - offset);
        let page = client
            .documents_page_fields(&index, offset, limit, &["title", "domain"])
            .await
            .context("paging corpus titles")?;
        if page.is_empty() {
            break;
        }
        let n = page.len() as u64;
        for doc in &page {
            if let Some(title) = doc.get("title").and_then(Value::as_str) {
                let domain = doc.get("domain").and_then(Value::as_str).unwrap_or("");
                count_title(&mut counts, title, domain_hash(domain), false);
            }
        }
        offset += n;
        if n < limit {
            break;
        }
    }
    let corpus_titles = counts.titles;

    // --- federated titles (T07.3) ----------------------------------------------------------------
    if let Some(path) = &opts.reference {
        let rows = crate::calibrate::load_reference(path)?;
        for row in &rows {
            // The query and each external title form one co-occurrence context: what the user asked,
            // next to what the web calls the answer. Federated rows count as one shared pseudo-domain
            // — they are evidence of a different *kind*, not of a different site.
            for title in &row.titles {
                count_title(
                    &mut counts,
                    &format!("{} {}", row.query, title),
                    domain_hash("\u{0}federated"),
                    true,
                );
            }
        }
        println!(
            "mined {} corpus titles + {} federated rows ({})",
            corpus_titles,
            rows.len(),
            path.display()
        );
    } else {
        println!("mined {corpus_titles} corpus titles (no federated capture given — pass --reference to add one)");
    }
    if counts.titles == 0 {
        anyhow::bail!("nothing to mine — the index has no titles and no reference was given");
    }

    // --- score pairs -----------------------------------------------------------------------------
    let known = known_pairs();
    let n = counts.titles as f64;
    let mut candidates: Vec<CandidatePair> = counts
        .pair
        .iter()
        .filter(|(_, ev)| ev.count >= opts.min_count && ev.domains.len() >= MIN_DOMAINS)
        .filter_map(|((ar, la), ev)| {
            let cx = *counts.token.get(ar)? as f64;
            let cy = *counts.token.get(la)? as f64;
            let pmi = (ev.count as f64 * n / (cx * cy)).ln();
            (pmi >= MIN_PMI && !known.contains(&(ar.clone(), la.clone()))).then(|| CandidatePair {
                arabic: ar.clone(),
                latin: la.clone(),
                count: ev.count,
                federated: ev.federated,
                pmi,
            })
        })
        .collect();
    // Strongest evidence first: frequency × over-representation.
    candidates.sort_by(|a, b| {
        (b.count as f64 * b.pmi)
            .partial_cmp(&(a.count as f64 * a.pmi))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.truncate(MAX_CANDIDATES);

    if candidates.is_empty() {
        println!(
            "no candidates cleared the bar (min co-occurrence {}, min PMI {MIN_PMI}) — nothing written",
            opts.min_count
        );
        return Ok(());
    }

    println!("\n{} candidates:", candidates.len());
    for c in candidates.iter().take(20) {
        println!(
            "  {} ↔ {}   seen {}× (federated {}), pmi {:.1}",
            c.arabic, c.latin, c.count, c.federated, c.pmi
        );
    }
    if candidates.len() > 20 {
        println!("  … and {} more in the file", candidates.len() - 20);
    }

    if !opts.dry_run {
        let mut body = String::from(
            "# MINED SYNONYM CANDIDATES — NOT LOADED BY ANYTHING. REVIEW BEFORE PROMOTING.\n\
             #\n\
             # Generated by `xustive-cli mine-synonyms` from corpus-title and federated-title\n\
             # co-occurrence (M7-T01.2/T07.3). Every line is a *hypothesis*: two tokens that appear\n\
             # together far more than chance. A native speaker promotes a real pair by moving it\n\
             # into synonyms.tsv or entities.tsv (same format) and running `make migrate`; a wrong\n\
             # pair here costs nothing, a wrong pair there pollutes every search for the term.\n\
             #\n\
             # Format:  concept_id <TAB> variant|variant <TAB> weight <TAB> evidence\n\n",
        );
        for (i, c) in candidates.iter().enumerate() {
            body.push_str(&format!(
                "cand_{:03}\t{}|{}\t0.6\tseen {}x (federated {}), pmi {:.1}\n",
                i + 1,
                c.arabic,
                c.latin,
                c.count,
                c.federated,
                c.pmi
            ));
        }
        if let Some(dir) = opts.out.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(&opts.out, body)
            .with_context(|| format!("writing {}", opts.out.display()))?;
        println!("\nwrote {}", opts.out.display());
    }
    Ok(())
}

/// Count one title's tokens: bump each token's frequency and every cross-script pair once per title,
/// remembering which domain the evidence came from.
fn count_title(counts: &mut Counts, title: &str, domain_hash: u64, federated: bool) {
    let normalized = xustive_text::normalize(title);
    let mut arabic: Vec<String> = Vec::new();
    let mut latin: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for tok in normalized
        .split_whitespace()
        .take(MAX_TOKENS_PER_TITLE)
        .filter_map(clean_token)
    {
        if !seen.insert(tok.clone()) {
            continue;
        }
        match script::detect(&tok) {
            Script::Arabic => arabic.push(tok),
            Script::Latin => latin.push(tok),
            _ => {}
        }
    }
    if arabic.is_empty() && latin.is_empty() {
        return;
    }
    counts.titles += 1;
    for t in arabic.iter().chain(latin.iter()) {
        *counts.token.entry(t.clone()).or_insert(0) += 1;
    }
    for a in &arabic {
        for l in &latin {
            let ev = counts.pair.entry((a.clone(), l.clone())).or_default();
            ev.count += 1;
            ev.note_domain(domain_hash);
            if federated {
                ev.federated += 1;
            }
        }
    }
}

/// A token worth counting, stripped to the word itself: surrounding punctuation trimmed (the first
/// dry run proposed `(صلى` and `سلم)` as if the bracket were part of the word), long enough to mean
/// something, not a stop word, not a number, and not a hostname riding in the title
/// (`sunnah.com` is a domain, not a synonym for anything).
fn clean_token(tok: &str) -> Option<String> {
    let t = tok.trim_matches(|c: char| !c.is_alphanumeric());
    if t.chars().count() < 3
        || t.contains('.')
        || t.contains('/')
        || t.chars().all(|c| c.is_ascii_digit())
        || xustive_search::settings::STOP_WORDS.contains(&t)
    {
        return None;
    }
    Some(t.to_string())
}

/// A stable hash for domain identity — only equality matters, never the name back.
fn domain_hash(domain: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    domain.hash(&mut h);
    h.finish()
}

/// Every (arabic, latin) pair the curated lexicon already relates, so mining re-proposes nothing a
/// human has already judged.
fn known_pairs() -> HashSet<(String, String)> {
    let mut known = HashSet::new();
    for concept in xustive_lang::Expander::default().concepts() {
        let variants: Vec<String> = concept
            .variants
            .iter()
            .map(|v| xustive_text::normalize(v))
            .collect();
        for a in &variants {
            for b in &variants {
                if script::detect(a) == Script::Arabic && script::detect(b) == Script::Latin {
                    known.insert((a.clone(), b.clone()));
                }
            }
        }
    }
    known
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_script_pairs_are_counted_once_per_title() {
        let mut c = Counts::default();
        // The duplicate token must not double the pair count.
        count_title(
            &mut c,
            "سونلغاز sonelgaz سونلغاز coupure",
            domain_hash("a.dz"),
            false,
        );
        assert_eq!(c.titles, 1);
        assert_eq!(c.pair.len(), 2, "sonelgaz+coupure × the one arabic token");
        assert!(c.pair.values().all(|ev| ev.count == 1 && ev.federated == 0));
    }

    #[test]
    fn stop_words_short_tokens_numbers_and_hostnames_are_dropped_and_punctuation_trimmed() {
        assert_eq!(clean_token("de"), None);
        assert_eq!(clean_token("في"), None);
        assert_eq!(clean_token("2026"), None);
        // The first dry run's failure modes, verbatim.
        assert_eq!(clean_token("sunnah.com"), None);
        assert_eq!(clean_token("(صلى"), Some("صلى".to_string()));
        assert_eq!(clean_token("سلم)"), Some("سلم".to_string()));
        assert_eq!(clean_token("sonelgaz"), Some("sonelgaz".to_string()));
    }

    #[test]
    fn same_script_pairs_are_not_proposed() {
        let mut c = Counts::default();
        count_title(
            &mut c,
            "coupure electricite oran",
            domain_hash("a.dz"),
            false,
        );
        assert!(c.pair.is_empty(), "latin-only titles produce no pairs");
        count_title(&mut c, "انقطاع الكهرباء وهران", domain_hash("a.dz"), false);
        assert!(c.pair.is_empty(), "arabic-only titles produce no pairs");
    }

    #[test]
    fn one_domains_template_is_one_piece_of_evidence() {
        // A hundred titles from the same site sharing boilerplate must not clear the domain floor;
        // the same pair on a second domain must. This is the sunnah.com lesson from the first run.
        let mut c = Counts::default();
        for _ in 0..100 {
            count_title(
                &mut c,
                "سونلغاز sonelgaz",
                domain_hash("one-site.dz"),
                false,
            );
        }
        let key = ("سونلغاز".to_string(), "sonelgaz".to_string());
        assert_eq!(c.pair[&key].count, 100);
        assert_eq!(
            c.pair[&key].domains.len(),
            1,
            "one site is one domain, however loud"
        );
        count_title(&mut c, "سونلغاز sonelgaz", domain_hash("other.dz"), false);
        assert!(c.pair[&key].domains.len() >= MIN_DOMAINS);
    }

    #[test]
    fn curated_pairs_are_already_known() {
        // The lexicon relates الجزائر ↔ alger; mining must not re-propose it. (Both sides
        // normalised, as the miner normalises before lookup.)
        let known = known_pairs();
        assert!(!known.is_empty());
        let ar = xustive_text::normalize("الجزائر");
        let la = xustive_text::normalize("Alger");
        assert!(known.contains(&(ar, la)));
    }
}
