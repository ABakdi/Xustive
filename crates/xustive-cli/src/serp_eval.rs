//! `xustive-cli eval-serp` — the offline "compare to Google" yardstick.
//!
//! For a set of queries it fetches a **reference engine's** ranked result domains and our own, and
//! measures how much our ordering agrees with the reference. It is a yardstick, not a target: the
//! reference is never consulted at query time and we are not trying to clone it — a persistent low
//! score just says "the famous answer for this query is not in our corpus or not ranked where a user
//! would expect it", which is exactly the gap the authority signal and query-driven discovery exist
//! to close.
//!
//! Google is the reference the user asked for, but it challenges this machine's IP, so the reference
//! defaults to DuckDuckGo-lite (which answers a direct connection and is Google-class in quality) and
//! switches to Google automatically once `discovery.serp_proxy` is set. The agreement maths is the
//! same whichever engine answers.
//!
//! Three agreement metrics, all over the top-`k` **domains** (a SERP hands back domains, not our
//! document ids, so the comparison is domain-level):
//! - **overlap@k** — fraction of the reference's top-k domains that appear anywhere in our top-k.
//! - **RBO** — rank-biased overlap (p=0.9): agreement weighted toward the very top, where users look.
//! - **NDCG@k** — our ordering scored against the reference's, treating a higher reference position as
//!   a higher gold grade.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;

use xustive_core::Config;
use xustive_ingest::serp::{Engine, SerpClient};
use xustive_search::{rank, MeiliClient, Query};

pub struct SerpEvalOptions {
    pub queries: PathBuf,
    pub out_dir: PathBuf,
    /// Reference engine override (`duckduckgo` | `bing` | `google`). Default: Google if a SERP proxy
    /// is configured, else DuckDuckGo.
    pub engine: Option<String>,
    /// How many top domains to compare.
    pub k: usize,
    /// Report only; never write a file.
    pub dry_run: bool,
    pub date: String,
}

/// Weight for rank-biased overlap. 0.9 concentrates ~86% of the weight in the top 10 — the right
/// shape for "did we agree where the user actually looks".
const RBO_P: f64 = 0.9;

/// Pause between reference-engine queries. A SERP tolerates one lookup but walls a fast burst of
/// them, so the yardstick paces itself like a person would; an offline eval can afford the minutes.
const QUERY_DELAY: std::time::Duration = std::time::Duration::from_millis(3500);

#[derive(Serialize)]
struct PerQuery {
    query: String,
    overlap: f64,
    rbo: f64,
    ndcg: f64,
    reference: Vec<String>,
    ours: Vec<String>,
}

#[derive(Serialize)]
struct SerpReport {
    generated_at: String,
    engine: String,
    k: usize,
    queries: usize,
    /// Queries the reference engine answered (the ones the means are computed over).
    answered: usize,
    mean_overlap: f64,
    mean_rbo: f64,
    mean_ndcg: f64,
    per_query: Vec<PerQuery>,
}

pub async fn run(client: &MeiliClient, config: &Config, opts: &SerpEvalOptions) -> Result<()> {
    let queries = load_queries(&opts.queries)?;
    if queries.is_empty() {
        anyhow::bail!("no queries in {}", opts.queries.display());
    }
    let k = opts.k.max(1);

    // Pick the reference engine. Google is the yardstick when we can reach it (proxy set); otherwise
    // DuckDuckGo-lite, which answers a direct connection.
    let proxy = config.discovery.serp_proxy.trim();
    let engine = match &opts.engine {
        Some(s) => Engine::parse(s).with_context(|| format!("unknown engine {s:?}"))?,
        None if !proxy.is_empty() => Engine::Google,
        None => Engine::DuckDuckGo,
    };
    let serp = SerpClient::new(vec![engine], Some(proxy).filter(|s| !s.is_empty()))
        .context("could not build the SERP client")?;

    let index = client.resolve(&config.search.documents_index).await?;
    let authority = xustive_search::authority::load();
    let weights = rank::Weights::default();
    let now = xustive_core::now_unix();
    let trust: HashMap<String, xustive_core::TrustTier> = HashMap::new();

    println!(
        "Comparing our top-{k} domains to {} for {} queries\n",
        engine.as_str(),
        queries.len()
    );

    let mut per_query = Vec::new();
    let (mut sum_overlap, mut sum_rbo, mut sum_ndcg) = (0.0, 0.0, 0.0);
    let mut answered = 0usize;

    for (i, q) in queries.iter().enumerate() {
        // Pace the reference lookups so a fast burst does not trip the engine's anomaly wall.
        if i > 0 {
            tokio::time::sleep(QUERY_DELAY).await;
        }
        // Reference ranking.
        let reference = domains_of(serp.search(q).await.iter().map(String::as_str), k);
        if reference.is_empty() {
            println!("  {q:<40}  reference returned nothing (skipped)");
            continue;
        }
        answered += 1;

        // Our ranking, through the same retrieval + re-rank the API uses.
        let ours = our_domains(client, &index, q, &authority, &weights, &trust, now, k).await?;

        let overlap = overlap_at_k(&ours, &reference);
        let rbo = rbo(&ours, &reference, RBO_P);
        let ndcg = ndcg_vs_reference(&ours, &reference);
        sum_overlap += overlap;
        sum_rbo += rbo;
        sum_ndcg += ndcg;

        println!("  {q:<40}  overlap {overlap:.2}  rbo {rbo:.2}  ndcg {ndcg:.2}");
        per_query.push(PerQuery {
            query: q.clone(),
            overlap,
            rbo,
            ndcg,
            reference,
            ours,
        });
    }

    if answered == 0 {
        anyhow::bail!(
            "the reference engine ({}) answered no queries — it is likely blocking this IP; set \
             discovery.serp_proxy or pass --engine duckduckgo",
            engine.as_str()
        );
    }

    let n = answered as f64;
    let report = SerpReport {
        generated_at: opts.date.clone(),
        engine: engine.as_str().to_string(),
        k,
        queries: queries.len(),
        answered,
        mean_overlap: sum_overlap / n,
        mean_rbo: sum_rbo / n,
        mean_ndcg: sum_ndcg / n,
        per_query,
    };

    println!(
        "\n{} queries answered by {} — mean overlap {:.3}  rbo {:.3}  ndcg {:.3}",
        report.answered, report.engine, report.mean_overlap, report.mean_rbo, report.mean_ndcg
    );

    if !opts.dry_run {
        std::fs::create_dir_all(&opts.out_dir)
            .with_context(|| format!("creating {}", opts.out_dir.display()))?;
        let path = opts
            .out_dir
            .join(format!("serp-{}-{}.json", report.engine, opts.date));
        std::fs::write(&path, serde_json::to_string_pretty(&report)?)
            .with_context(|| format!("writing {}", path.display()))?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

/// Run one query through our retrieval + re-rank and return the ordered, de-duplicated top-`k`
/// domains — the same path [`crate::eval`] uses, minus the golden-set machinery.
async fn our_domains(
    client: &MeiliClient,
    index: &str,
    query: &str,
    authority: &HashMap<String, f32>,
    weights: &rank::Weights,
    trust: &HashMap<String, xustive_core::TrustTier>,
    now: i64,
    k: usize,
) -> Result<Vec<String>> {
    let normalized = xustive_text::normalize(query);
    let pool = 200usize;
    let q = Query::new(&normalized).limit(pool);
    let hits = client
        .search::<Value>(index, &q)
        .await
        .with_context(|| format!("searching for {query:?}"))?;
    let ranked = rank::rerank(
        &hits.hits,
        &normalized,
        now,
        trust,
        authority,
        &std::collections::HashMap::new(),
        weights,
    );
    Ok(domains_of(
        ranked
            .iter()
            .filter_map(|r| r.hit.get("domain").and_then(Value::as_str)),
        k,
    ))
}

/// Ordered, de-duplicated top-`k` registrable-ish domains from a stream of URLs or hosts.
fn domains_of<'a>(items: impl Iterator<Item = &'a str>, k: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in items {
        let Some(d) = normalize_domain(item) else {
            continue;
        };
        if seen.insert(d.clone()) {
            out.push(d);
            if out.len() == k {
                break;
            }
        }
    }
    out
}

/// A URL or a bare host down to a comparable host string: scheme dropped, `www.` stripped, lowercased.
fn normalize_domain(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let host = if s.contains("://") {
        url::Url::parse(s).ok()?.host_str()?.to_string()
    } else {
        // Already a host (our `domain` field); take the authority part if a path slipped in.
        s.split('/').next()?.to_string()
    };
    let host = host.trim_start_matches("www.").to_ascii_lowercase();
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

/// Fraction of the reference's domains that also appear in ours.
fn overlap_at_k(ours: &[String], reference: &[String]) -> f64 {
    if reference.is_empty() {
        return 0.0;
    }
    let set: HashSet<&String> = ours.iter().collect();
    let hit = reference.iter().filter(|d| set.contains(d)).count();
    hit as f64 / reference.len() as f64
}

/// Rank-biased overlap: the expected agreement of the two prefixes under a geometric browsing model
/// with continuation probability `p`. 1.0 = identical order, 0.0 = disjoint.
///
/// The raw prefix form of two *identical* finite lists sums to `1 - p^depth`, not 1, so it is
/// normalised by that maximum — giving a clean [0, 1] where identical order is exactly 1.0. Because
/// the normaliser depends only on `depth`, it does not distort comparisons between two candidate
/// orderings measured against the same reference.
fn rbo(ours: &[String], reference: &[String], p: f64) -> f64 {
    let depth = ours.len().max(reference.len());
    if depth == 0 {
        return 0.0;
    }
    let mut seen_ours: HashSet<&String> = HashSet::new();
    let mut seen_ref: HashSet<&String> = HashSet::new();
    let mut overlap = 0usize;
    let mut sum = 0.0;
    for d in 1..=depth {
        if let Some(x) = ours.get(d - 1) {
            if seen_ref.contains(x) {
                overlap += 1;
            }
            seen_ours.insert(x);
        }
        if let Some(y) = reference.get(d - 1) {
            if seen_ours.contains(y) {
                overlap += 1;
            }
            seen_ref.insert(y);
        }
        // Agreement at depth d: shared items so far / d.
        let agreement = overlap as f64 / d as f64;
        sum += p.powi(d as i32 - 1) * agreement;
    }
    let max = (1.0 - p.powi(depth as i32)) / (1.0 - p);
    if max == 0.0 {
        0.0
    } else {
        (sum / max).clamp(0.0, 1.0)
    }
}

/// NDCG of our ordering, grading each domain by how high the reference ranked it (top reference
/// domain = highest grade), so agreeing with the reference near the top is what scores.
fn ndcg_vs_reference(ours: &[String], reference: &[String]) -> f64 {
    let n = reference.len();
    if n == 0 {
        return 0.0;
    }
    // grade(domain) = n - reference_rank (so rank 0 → n, rank n-1 → 1; absent → 0).
    let grade: HashMap<&String, f64> = reference
        .iter()
        .enumerate()
        .map(|(i, d)| (d, (n - i) as f64))
        .collect();

    let dcg = |ordered: &[f64]| -> f64 {
        ordered
            .iter()
            .enumerate()
            .map(|(i, g)| g / ((i + 2) as f64).log2())
            .sum()
    };
    let ours_grades: Vec<f64> = ours
        .iter()
        .map(|d| grade.get(d).copied().unwrap_or(0.0))
        .collect();
    let mut ideal: Vec<f64> = grade.values().copied().collect();
    ideal.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

    let idcg = dcg(&ideal);
    if idcg == 0.0 {
        0.0
    } else {
        (dcg(&ours_grades) / idcg).clamp(0.0, 1.0)
    }
}

fn load_queries(path: &PathBuf) -> Result<Vec<String>> {
    let body =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn identical_rankings_score_perfectly() {
        let a = v(&["a.com", "b.com", "c.com"]);
        assert_eq!(overlap_at_k(&a, &a), 1.0);
        assert!((rbo(&a, &a, 0.9) - 1.0).abs() < 1e-9);
        assert!((ndcg_vs_reference(&a, &a) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn disjoint_rankings_score_zero() {
        let a = v(&["a.com", "b.com"]);
        let b = v(&["x.com", "y.com"]);
        assert_eq!(overlap_at_k(&a, &b), 0.0);
        assert!(rbo(&a, &b, 0.9) < 1e-9);
        assert_eq!(ndcg_vs_reference(&a, &b), 0.0);
    }

    #[test]
    fn same_set_wrong_order_beats_disjoint_but_trails_exact() {
        let reference = v(&["a.com", "b.com", "c.com"]);
        let reversed = v(&["c.com", "b.com", "a.com"]);
        let exact = ndcg_vs_reference(&reference, &reference);
        let rev = ndcg_vs_reference(&reversed, &reference);
        assert_eq!(
            overlap_at_k(&reversed, &reference),
            1.0,
            "same set overlaps fully"
        );
        assert!(rev < exact, "wrong order must score below exact");
        assert!(rev > 0.0, "same set, wrong order still beats disjoint");
    }

    #[test]
    fn agreement_at_the_top_matters_more() {
        let reference = v(&["a.com", "b.com", "c.com", "d.com"]);
        let top_right = v(&["a.com", "z.com", "y.com", "x.com"]); // #1 correct
        let bottom_right = v(&["z.com", "y.com", "x.com", "d.com"]); // #4 correct
        assert!(
            rbo(&top_right, &reference, 0.9) > rbo(&bottom_right, &reference, 0.9),
            "matching the top result should beat matching the fourth"
        );
    }

    #[test]
    fn domains_are_normalized_and_deduped_in_order() {
        let got = domains_of(
            [
                "https://www.Wikipedia.org/wiki/X",
                "https://en.wikipedia.org/wiki/Y",
                "https://www.wikipedia.org/wiki/Z", // dup of the first after normalisation
                "bbc.com/news",
            ]
            .into_iter(),
            10,
        );
        assert_eq!(got, v(&["wikipedia.org", "en.wikipedia.org", "bbc.com"]));
    }
}
