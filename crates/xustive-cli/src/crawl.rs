//! The `crawl` command: seed URLs to indexed documents.
//!
//! This is the minimum honest path from the live web to a searchable index. It fetches politely,
//! parses, deduplicates within the run, and indexes in batches.
//!
//! What it is **not** is the full crawler. There is no persistent frontier, no adaptive revisit
//! scheduling, no cross-process politeness state and no social connectors — those are a later
//! milestone. This runs once, over a seed list, and stops.

use std::collections::HashSet;
use std::time::Instant;

use anyhow::{Context, Result};
use serde_json::Value;

use xustive_core::{Classify, Config, SourceType, TrustTier};
use xustive_ingest::{FetchConfig, FetchError, Fetcher, ParseError, Parser};
use xustive_lang::Scorer;
use xustive_search::MeiliClient;

/// One line of `data/sources/seeds.tsv`.
#[derive(Debug, Clone)]
pub struct Seed {
    pub source_id: String,
    pub url: String,
    pub trust: TrustTier,
}

pub fn parse_seeds(tsv: &str) -> Vec<Seed> {
    let mut out = Vec::new();
    for line in tsv.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').map(str::trim).collect();
        if cols.len() < 2 || cols[1].is_empty() {
            continue;
        }
        let trust = match cols.get(2).map(|t| t.to_ascii_uppercase()).as_deref() {
            Some("A") => TrustTier::A,
            Some("C") => TrustTier::C,
            _ => TrustTier::B,
        };
        out.push(Seed {
            source_id: cols[0].to_string(),
            url: cols[1].to_string(),
            trust,
        });
    }
    out
}

#[derive(Debug, Default)]
pub struct Stats {
    pub fetched: usize,
    pub indexed: usize,
    pub skipped_robots: usize,
    pub skipped_duplicate: usize,
    pub skipped_thin: usize,
    pub failed: usize,
}

pub struct CrawlOptions {
    pub max_pages_per_source: usize,
    pub max_total: usize,
    pub batch_size: usize,
    pub discover_links: bool,
}

impl Default for CrawlOptions {
    fn default() -> Self {
        Self {
            max_pages_per_source: 60,
            max_total: 500,
            batch_size: 100,
            discover_links: true,
        }
    }
}

pub async fn run(
    client: &MeiliClient,
    config: &Config,
    seeds: &[Seed],
    opts: &CrawlOptions,
) -> Result<Stats> {
    let fetcher = Fetcher::new(FetchConfig::default()).context("building the fetcher")?;
    // Per-domain rules, loaded once for the whole crawl. Without them, publishers that emit no
    // machine-readable metadata — which is most of the Algerian press — yield no date at all.
    let rules = xustive_ingest::rules::Rules::load("data/parsers/domains.toml");
    let parser = Parser::default().with_rules(rules);
    let sentiment = Scorer::default();
    let started = Instant::now();

    let mut stats = Stats::default();
    let mut batch: Vec<Value> = Vec::new();
    // Content hashes seen this run. Cross-run deduplication needs the persistent store that
    // arrives with the real pipeline; within a run this is enough to stop the same article
    // being indexed from both a sitemap and a homepage link.
    let mut seen_hashes: HashSet<String> = HashSet::new();
    let mut seen_urls: HashSet<String> = HashSet::new();

    for seed in seeds {
        if stats.indexed >= opts.max_total {
            break;
        }
        println!("\n▸ {} — {}", seed.source_id, seed.url);

        let mut queue = discover(&fetcher, &seed.url, opts).await;
        queue.retain(|u| seen_urls.insert(u.clone()));
        queue.truncate(opts.max_pages_per_source);
        println!("  {} candidate pages", queue.len());

        let mut from_source = 0usize;
        for url in queue {
            if stats.indexed >= opts.max_total {
                break;
            }

            let fetched = match fetcher.get(&url).await {
                Ok(f) => f,
                Err(FetchError::RobotsDisallowed) => {
                    stats.skipped_robots += 1;
                    continue;
                }
                Err(e) => {
                    // A permanent error is the page's property, not a transient fault; there is
                    // nothing to report beyond a count.
                    if !e.class().is_retryable() {
                        tracing::debug!(%url, error = %e, "skipped");
                    } else {
                        tracing::warn!(%url, error = %e, "fetch failed");
                    }
                    stats.failed += 1;
                    continue;
                }
            };
            stats.fetched += 1;

            let parsed = match parser.parse(
                &fetched.body,
                &fetched.final_url,
                &seed.source_id,
                SourceType::Web,
            ) {
                Ok(p) => p,
                Err(ParseError::TooLittleContent { .. }) | Err(ParseError::NoIndex) => {
                    stats.skipped_thin += 1;
                    continue;
                }
                // Logged rather than counted with thin pages: a page this shape is broken or
                // hostile, and a crawl that starts refusing many of them is telling us something
                // a "skipped" tally would hide.
                Err(e @ ParseError::TooComplex { .. }) => {
                    tracing::warn!(url = %fetched.final_url, error = %e, "skipping pathological markup");
                    stats.skipped_thin += 1;
                    continue;
                }
            };

            let mut doc = parsed.document;
            if !seen_hashes.insert(doc.content_hash.clone()) {
                stats.skipped_duplicate += 1;
                continue;
            }

            // Source trust is a property of the registry, not of the page.
            doc.quality_score = (doc.quality_score * 0.7) + (seed.trust.weight() * 0.3);

            // Sentiment is scored from the title and the opening of the body, where it is
            // usually established. Scoring the whole document is slower and more diluted.
            doc.sentiment = sentiment.score(
                &format!("{} {}", doc.title, head(&doc.body, 800)),
                doc.language,
            );

            println!(
                "  · {:<58} {}",
                truncate(&doc.title, 58),
                doc.language.as_str()
            );

            batch.push(serde_json::to_value(&doc)?);
            from_source += 1;
            stats.indexed += 1;

            if batch.len() >= opts.batch_size {
                flush(client, config, &mut batch).await?;
            }
        }
        println!("  indexed {from_source}");
    }

    flush(client, config, &mut batch).await?;

    println!(
        "\n{} indexed, {} fetched, {} duplicates, {} thin, {} robots-blocked, {} failed in {:.1}s",
        stats.indexed,
        stats.fetched,
        stats.skipped_duplicate,
        stats.skipped_thin,
        stats.skipped_robots,
        stats.failed,
        started.elapsed().as_secs_f32()
    );
    Ok(stats)
}

/// Find pages to fetch for a seed.
///
/// A sitemap or feed is always preferred: it is a list the publisher maintains, so it beats
/// guessing which links on a homepage are articles.
async fn discover(fetcher: &Fetcher, seed_url: &str, opts: &CrawlOptions) -> Vec<String> {
    use xustive_ingest::sitemap;

    let looks_like_feed = seed_url.ends_with(".xml") || seed_url.contains("sitemap");

    if looks_like_feed {
        if let Ok(f) = fetcher.get(seed_url).await {
            let urls = sitemap::extract_urls(&f.body, opts.max_pages_per_source * 4);
            if sitemap::is_index(&f.body) {
                // A sitemap index points at more sitemaps. One level is enough here.
                let mut pages = Vec::new();
                for child in urls.iter().take(3) {
                    if let Ok(cf) = fetcher.get(child).await {
                        pages.extend(sitemap::extract_urls(&cf.body, opts.max_pages_per_source));
                    }
                    if pages.len() >= opts.max_pages_per_source {
                        break;
                    }
                }
                return pages;
            }
            // Filter sitemap URLs the same way as discovered links.
            //
            // Not every sitemap lists articles. aps.dz publishes a *navigation* sitemap —
            // "/dossier", "/infographie", the homepage — so taking it at face value indexed
            // five copies of the site's landing pages under one title. A sitemap is a hint
            // about what exists, not a promise that it is content.
            let articles: Vec<String> = urls
                .iter()
                .filter(|u| {
                    url::Url::parse(u)
                        .map(|p| looks_like_article(p.path()))
                        .unwrap_or(false)
                })
                .cloned()
                .collect();
            if !articles.is_empty() {
                return articles;
            }
            tracing::info!(
                seed = seed_url,
                listed = urls.len(),
                "sitemap lists no article-shaped urls, falling back to link discovery"
            );
        }
        // Fall through to homepage discovery rather than giving up: a section-only sitemap
        // says nothing about whether the site has articles.
        let root = url::Url::parse(seed_url)
            .ok()
            .and_then(|u| u.join("/").ok())
            .map(|u| u.to_string())
            .unwrap_or_else(|| seed_url.to_string());
        return Box::pin(discover_from_page(fetcher, &root, opts)).await;
    }

    discover_from_page(fetcher, seed_url, opts).await
}

/// Take a page and the same-host article links on it.
async fn discover_from_page(fetcher: &Fetcher, seed_url: &str, opts: &CrawlOptions) -> Vec<String> {
    let mut out = vec![seed_url.to_string()];
    if !opts.discover_links {
        return out;
    }

    let Ok(f) = fetcher.get(seed_url).await else {
        return out;
    };
    let Ok(base) = url::Url::parse(seed_url) else {
        return out;
    };
    let host = base.host_str().unwrap_or_default().to_string();

    if let Ok(p) = Parser::default().parse(&f.body, &f.final_url, "discovery", SourceType::Web) {
        for link in p.outlinks {
            if out.len() >= opts.max_pages_per_source {
                break;
            }
            let Ok(u) = url::Url::parse(&link) else {
                continue;
            };
            // Same host only. Following outward turns a seeded crawl into an open one, which is
            // a much larger decision than this command should be making.
            if u.host_str().unwrap_or_default() != host {
                continue;
            }
            if looks_like_article(u.path()) && !out.contains(&link) {
                out.push(link);
            }
        }
    }
    out
}

/// Heuristic for "this path is probably an article, not a section index".
///
/// Deliberately loose. Missing an article costs one page; fetching a section index costs one
/// wasted request and a thin-content skip.
fn looks_like_article(path: &str) -> bool {
    let segments = path.trim_matches('/').split('/').count();
    let has_digits = path.chars().any(|c| c.is_ascii_digit());
    let is_asset = [".jpg", ".png", ".pdf", ".css", ".js", ".zip", ".mp4"]
        .iter()
        .any(|e| path.ends_with(e));
    !is_asset && segments >= 2 && (has_digits || segments >= 3)
}

async fn flush(client: &MeiliClient, config: &Config, batch: &mut Vec<Value>) -> Result<()> {
    if batch.is_empty() {
        return Ok(());
    }
    let uid = client
        .add_documents(&config.search.documents_index, batch)
        .await
        .context("submitting batch")?;
    // Wait for the task: reporting success for a batch that later fails is worse than slow.
    client.wait_task(uid).await.context("indexing batch")?;
    batch.clear();
    Ok(())
}

fn head(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn truncate(s: &str, n: usize) -> String {
    let t: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        format!("{t}…")
    } else {
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_seed_lines() {
        let tsv = "# comment\n\naps-dz\thttps://a.dz/sitemap.xml\tA\tnote\nx\thttps://b.dz/\n";
        let seeds = parse_seeds(tsv);
        assert_eq!(seeds.len(), 2);
        assert_eq!(seeds[0].source_id, "aps-dz");
        assert_eq!(seeds[0].trust, TrustTier::A);
        // An omitted tier defaults to B rather than to the most trusting option.
        assert_eq!(seeds[1].trust, TrustTier::B);
    }

    #[test]
    fn malformed_seed_lines_are_skipped() {
        assert!(parse_seeds("just-an-id\n\t\t\n").is_empty());
    }

    #[test]
    fn shipped_seed_file_parses_and_is_substantial() {
        let seeds = parse_seeds(include_str!("../../../data/sources/seeds.tsv"));
        assert!(seeds.len() >= 15, "only {} seeds", seeds.len());
        for s in &seeds {
            assert!(
                s.url.starts_with("https://") || s.url.starts_with("http://"),
                "{} has a non-http url: {}",
                s.source_id,
                s.url
            );
        }
    }

    #[test]
    fn article_paths_are_told_from_section_indexes() {
        assert!(looks_like_article("/article/123456"));
        assert!(looks_like_article("/economie/2026/08/04/titre"));
        assert!(looks_like_article("/ar/news/12345"));

        assert!(!looks_like_article("/"));
        assert!(!looks_like_article("/economie"));
        assert!(!looks_like_article("/images/photo.jpg"));
        assert!(!looks_like_article("/assets/app.js"));
    }

    #[test]
    fn truncate_is_char_safe() {
        assert_eq!(truncate("الجزائر", 3), "الج…");
        assert_eq!(truncate("ab", 5), "ab");
    }
}
