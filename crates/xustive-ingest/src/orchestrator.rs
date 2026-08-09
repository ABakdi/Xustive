//! The crawl loop.
//!
//! Claim a URL, fetch it, parse it, hand the document on, queue the links it points to, repeat.
//! That is the whole thing; the difficulty is entirely in the edges.
//!
//! # Why the loop is dull on purpose
//!
//! A crawler is a program that runs for months without supervision against servers that change
//! without warning. Every clever scheduling decision is a decision somebody has to reconstruct at
//! two in the morning from a document count that stopped rising. So: shallow-first, one host at a
//! time, fixed budgets, and every skip counted under a name that says which rule fired.
//!
//! # What it does not do
//!
//! It does not index. It emits parsed documents and lets the caller decide — the CLI writes them
//! to Meilisearch directly, the daemon puts them on the queue. Keeping that out means the loop can
//! be tested end to end without an index.

use std::collections::HashMap;
use std::time::Duration;

use crate::crawl_stats::{CrawlStats, RecentUrl};
use crate::fetch::{FetchError, Fetcher};
use crate::frontier::{self, Frontier, Pending};
use crate::parse::{ParseConfig, ParseError, Parsed, Parser};

/// How long to wait when the frontier has nothing due.
///
/// Short enough that a newly-due host is picked up promptly, long enough that an idle crawler is
/// not a busy loop against Redis. An idle crawler is the *normal* state once a corpus is warm —
/// most hosts are waiting out a crawl-delay most of the time.
const IDLE_SLEEP: Duration = Duration::from_millis(500);

/// How often expired claims are swept.
const RECLAIM_EVERY: Duration = Duration::from_secs(30);

#[derive(Debug, Default, Clone)]
pub struct Stats {
    pub fetched: usize,
    pub parsed: usize,
    pub discovered: usize,
    pub failed: usize,
    /// Keyed by the rule that fired, so "the crawler is collecting nothing" resolves to *which*
    /// rule is eating everything. A single `skipped` total cannot answer that.
    pub skipped: HashMap<&'static str, usize>,
}

impl Stats {
    fn skip(&mut self, reason: &'static str) {
        *self.skipped.entry(reason).or_insert(0) += 1;
    }
}

#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    /// Links followed from a seed before a URL is dropped.
    pub max_depth: u32,
    /// Per-host crawl delay when robots does not name one. The fetcher enforces the real value;
    /// this is what the frontier uses to schedule the host's next slot.
    pub default_delay: Duration,
    /// Stop after this many documents. `None` runs until stopped, which is the daemon's case.
    pub max_documents: Option<usize>,
    /// Follow links to hosts we have not seen before.
    ///
    /// Off by default. Turning it on is the difference between crawling twenty known sources and
    /// crawling the web, and that is a decision worth making explicitly rather than inheriting.
    pub discover_new_hosts: bool,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            max_depth: 3,
            default_delay: Duration::from_millis(1500),
            max_documents: None,
            discover_new_hosts: false,
        }
    }
}

/// What the loop produces.
pub enum Outcome {
    /// A document worth keeping.
    Document(Box<Parsed>),
    /// Nothing due; the caller should back off.
    Idle,
    /// The document budget is spent.
    Finished,
}

pub struct Orchestrator {
    fetcher: Fetcher,
    parser: Parser,
    frontier: Frontier,
    config: OrchestratorConfig,
    /// Hosts we already know, so `discover_new_hosts = false` can tell an outward link from an
    /// internal one without a registry lookup per link.
    known_hosts: std::collections::HashSet<String>,
    stats: Stats,
    documents: usize,
    last_reclaim: std::time::Instant,
    /// Published so the console and Prometheus read the same numbers. Optional: a crawl without it
    /// is fully functional and merely unobservable, which is the right way round — observability
    /// must not be able to stop the crawl.
    shared: Option<CrawlStats>,
}

impl Orchestrator {
    pub fn new(fetcher: Fetcher, frontier: Frontier, config: OrchestratorConfig) -> Self {
        Self {
            fetcher,
            parser: Parser::new(ParseConfig::default()),
            frontier,
            config,
            known_hosts: std::collections::HashSet::new(),
            stats: Stats::default(),
            documents: 0,
            last_reclaim: std::time::Instant::now(),
            shared: None,
        }
    }

    /// Publish counters where the admin console can read them.
    pub fn with_shared_stats(mut self, shared: CrawlStats) -> Self {
        self.shared = Some(shared);
        self
    }

    async fn publish(&self, field: &str) {
        if let Some(s) = &self.shared {
            s.incr(field, 1).await;
        }
    }

    async fn publish_skip(&self, reason: &str) {
        if let Some(s) = &self.shared {
            s.incr_skip(reason).await;
        }
    }

    async fn publish_recent(&self, url: &str, host: &str, outcome: &str, words: usize) {
        if let Some(s) = &self.shared {
            s.record(&RecentUrl {
                url: url.to_string(),
                host: host.to_string(),
                outcome: outcome.to_string(),
                at: xustive_core::now_unix(),
                words,
            })
            .await;
        }
    }

    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    pub fn frontier(&self) -> &Frontier {
        &self.frontier
    }

    /// Seed the frontier. Idempotent — a URL already known is not queued twice.
    pub async fn seed(&mut self, url: &str, source_id: &str, trust: u8) -> bool {
        let Ok(parsed) = url::Url::parse(url) else {
            return false;
        };
        let host = parsed.host_str().unwrap_or_default().to_string();
        if host.is_empty() {
            return false;
        }
        self.known_hosts.insert(host.clone());

        let pending = Pending {
            url: frontier::canonical(&parsed),
            host,
            source_id: source_id.to_string(),
            depth: 0,
            priority: frontier::priority_for(0, trust, false),
        };
        matches!(self.frontier.add(&pending).await, Ok(true))
    }

    /// One turn of the loop.
    ///
    /// Returns a document, or says why there is nothing. Called in a loop by the daemon; called a
    /// bounded number of times by the CLI.
    pub async fn step(&mut self, now_ms: i64) -> Outcome {
        if self
            .config
            .max_documents
            .is_some_and(|max| self.documents >= max)
        {
            return Outcome::Finished;
        }

        // Sweep abandoned claims periodically rather than every turn. A worker that died is not
        // urgent, and doing it per step would put a scan of every in-flight claim on the hot path.
        if self.last_reclaim.elapsed() > RECLAIM_EVERY {
            let n = self.frontier.reclaim(now_ms).await;
            if n > 0 {
                tracing::info!(reclaimed = n, "returned claims from workers that went away");
            }
            self.last_reclaim = std::time::Instant::now();
        }

        let Some(claim) = self.frontier.claim(now_ms, self.config.default_delay).await else {
            return Outcome::Idle;
        };

        let outcome = self.process(&claim.url, &claim.host).await;
        // Completed whatever happened. A URL that failed is not retried by leaving it claimed —
        // that would block its host until the claim expired, which punishes the host for our
        // problem. Retries belong to the frontier, as a fresh entry.
        self.frontier.complete(&claim.url).await;
        outcome
    }

    async fn process(&mut self, url: &str, host: &str) -> Outcome {
        let fetched = match self.fetcher.get(url).await {
            Ok(f) => f,
            Err(FetchError::RobotsDisallowed) => {
                self.stats.skip("robots");
                self.publish_skip("robots").await;
                self.publish_recent(url, host, "robots", 0).await;
                return Outcome::Idle;
            }
            Err(e) => {
                self.stats.failed += 1;
                tracing::debug!(%url, error = %e, "fetch failed");
                self.publish("failed").await;
                self.publish_recent(url, host, "failed", 0).await;
                return Outcome::Idle;
            }
        };
        self.stats.fetched += 1;
        self.publish("fetched").await;

        // The header the site sent, which is the only way a document without a `<head>` can refuse
        // indexing. Checked before parsing so we do not spend the work.
        if fetched
            .exclusion
            .is_some_and(|e| e.blocks_indexing() && e.blocks_links())
        {
            self.stats.skip("x_robots_tag");
            return Outcome::Idle;
        }

        let parsed = match self.parser.parse(
            &fetched.body,
            &fetched.final_url,
            "crawl",
            xustive_core::SourceType::Web,
        ) {
            Ok(p) => p,
            Err(ParseError::TooLittleContent { .. }) => {
                self.stats.skip("thin");
                self.publish_skip("thin").await;
                self.publish_recent(url, host, "thin", 0).await;
                return Outcome::Idle;
            }
            Err(ParseError::NoIndex) => {
                self.stats.skip("noindex");
                self.publish_skip("noindex").await;
                self.publish_recent(url, host, "noindex", 0).await;
                return Outcome::Idle;
            }
            Err(e) => {
                // Not counted with thin pages. A crawl that starts refusing many of these is
                // telling us something a shared tally would hide.
                tracing::warn!(url = %fetched.final_url, error = %e, "pathological markup");
                self.stats.skip("malformed");
                return Outcome::Idle;
            }
        };
        self.stats.parsed += 1;
        self.publish("parsed").await;

        // Links are followed unless the page asked otherwise. A `noindex` page is still worth
        // crawling *through* — that combination is how sites let a crawler reach content behind a
        // section they do not want listed.
        if !fetched.exclusion.is_some_and(|e| e.blocks_links()) {
            self.enqueue_outlinks(&parsed, host).await;
        }

        if fetched.exclusion.is_some_and(|e| e.blocks_indexing()) {
            self.stats.skip("noindex_header");
            return Outcome::Idle;
        }

        self.documents += 1;
        let words = parsed.document.body.split_whitespace().count();
        self.publish_recent(&fetched.final_url, host, "indexed", words)
            .await;
        Outcome::Document(Box::new(parsed))
    }

    async fn enqueue_outlinks(&mut self, parsed: &Parsed, from_host: &str) {
        let depth = 1; // Depth tracking through the frontier arrives with revisit scheduling.
        if depth > self.config.max_depth {
            return;
        }

        for link in &parsed.outlinks {
            let Ok(u) = url::Url::parse(link) else {
                continue;
            };
            let host = u.host_str().unwrap_or_default().to_string();
            if host.is_empty() {
                continue;
            }

            // Off-site links are dropped unless discovery is on. Following them is the difference
            // between crawling twenty sources and crawling the web.
            if host != from_host
                && !self.config.discover_new_hosts
                && !self.known_hosts.contains(&host)
            {
                self.stats.skip("off_site");
                continue;
            }

            if let Some(rejected) = frontier::detect_trap(&u) {
                self.stats.skip(rejected.as_str());
                continue;
            }

            let pending = Pending {
                url: frontier::canonical(&u),
                host,
                source_id: parsed.document.source_id.clone(),
                depth,
                priority: frontier::priority_for(depth, 50, frontier::looks_like_article(u.path())),
            };
            match self.frontier.add(&pending).await {
                Ok(true) => {
                    self.stats.discovered += 1;
                    self.publish("discovered").await;
                }
                Ok(false) => self.stats.skip("seen"),
                Err(r) => self.stats.skip(r.as_str()),
            }
        }
    }

    /// How long to wait when there is nothing due.
    pub fn idle_sleep(&self) -> Duration {
        IDLE_SLEEP
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_are_counted_by_the_rule_that_fired() {
        // "The crawler is collecting nothing" has to resolve to *which* rule is eating everything.
        // A single total cannot answer that, and it is the question that gets asked first.
        let mut s = Stats::default();
        s.skip("robots");
        s.skip("thin");
        s.skip("thin");
        assert_eq!(s.skipped.get("thin"), Some(&2));
        assert_eq!(s.skipped.get("robots"), Some(&1));
        assert_eq!(s.skipped.get("noindex"), None);
    }

    #[test]
    fn discovery_of_new_hosts_is_off_by_default() {
        // The difference between crawling twenty known sources and crawling the web. That is a
        // decision to make explicitly, not one to inherit from a default.
        assert!(!OrchestratorConfig::default().discover_new_hosts);
    }

    #[test]
    fn an_idle_crawler_does_not_spin() {
        // Idle is the normal state once a corpus is warm — most hosts are waiting out a
        // crawl-delay most of the time, so this path runs far more often than the busy one.
        assert!(IDLE_SLEEP >= Duration::from_millis(100));
        assert!(IDLE_SLEEP <= Duration::from_secs(2));
    }

    #[test]
    fn claims_are_swept_far_less_often_than_the_loop_runs() {
        // A worker that died is not urgent, and scanning every in-flight claim per step would put
        // that scan on the hot path.
        assert!(RECLAIM_EVERY >= Duration::from_secs(10));
    }
}
