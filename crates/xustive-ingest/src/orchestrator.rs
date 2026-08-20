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
use crate::revisit::{Observation, Visits};

/// How long to wait when the frontier has nothing due.
///
/// Short enough that a newly-due host is picked up promptly, long enough that an idle crawler is
/// not a busy loop against Redis. An idle crawler is the *normal* state once a corpus is warm —
/// most hosts are waiting out a crawl-delay most of the time.
const IDLE_SLEEP: Duration = Duration::from_millis(500);

/// How often expired claims are swept.
const RECLAIM_EVERY: Duration = Duration::from_secs(30);

/// How often due pages are moved back into their host queues.
const PROMOTE_EVERY: Duration = Duration::from_secs(30);

/// Most pages promoted in one sweep.
///
/// Bounded because a corpus seeded in one run comes due in one run: without a cap, the first sweep
/// after a day's quiet would move a million URLs in a single call and stall every worker behind it.
const PROMOTE_BATCH: usize = 500;

#[derive(Debug, Default, Clone)]
pub struct Stats {
    pub fetched: usize,
    /// Of `fetched`, how many were revisits of pages already held. `fetched - revisited` is fresh
    /// discovery. Split out so freshness and coverage are separately visible: a crawl whose fetches
    /// are almost all revisits is keeping its corpus current but not growing it, and one with almost
    /// none is the reverse. A single total hides which is happening (M2-T15.8).
    pub revisited: usize,
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
    /// Schedule each fetched page to be visited again ([[ADR-0011]]).
    ///
    /// Off by default so a bounded one-shot crawl stays bounded: `crawl --max 50` should fetch
    /// fifty pages and stop, not fifty and then whatever came due while it ran.
    pub revisit: bool,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            max_depth: 3,
            default_delay: Duration::from_millis(1500),
            max_documents: None,
            discover_new_hosts: false,
            revisit: false,
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
    /// Revisit state, when scheduling is on. Optional for the same reason as `shared`: a crawl
    /// that cannot reach it should still crawl.
    visits: Option<Visits>,
    raw_store: Option<crate::raw_store::RawStore>,
    /// The domain link graph, when capture is on. Optional like the rest: a crawl that cannot reach
    /// it still crawls, it just contributes nothing to the next PageRank.
    link_graph: Option<crate::link_graph::LinkGraphStore>,
    last_promote: std::time::Instant,
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
            visits: None,
            raw_store: None,
            link_graph: None,
            last_promote: std::time::Instant::now(),
        }
    }

    /// Capture the domain link graph as pages are crawled, for `xustive-cli pagerank`. Off unless set.
    pub fn with_link_graph(mut self, store: crate::link_graph::LinkGraphStore) -> Self {
        self.link_graph = Some(store);
        self
    }

    /// Publish counters where the admin console can read them.
    pub fn with_shared_stats(mut self, shared: CrawlStats) -> Self {
        self.shared = Some(shared);
        self
    }

    /// Give the loop somewhere to record what it learned about each page.
    ///
    /// Without this, `revisit` has nowhere to store an interval and scheduling silently does
    /// nothing — so the two are set together or not at all.
    pub fn with_visits(mut self, visits: Visits) -> Self {
        self.visits = Some(visits);
        self
    }

    /// Keep the raw fetched body for later reindexing (M2-T04.7). Off unless set.
    pub fn with_raw_store(mut self, store: crate::raw_store::RawStore) -> Self {
        self.raw_store = Some(store);
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

    async fn publish_source(&self, source_id: &str, metric: &str, by: u64) {
        if let Some(s) = &self.shared {
            s.incr_source(source_id, metric, by).await;
        }
    }

    async fn publish_channel(&self, channel: xustive_core::DiscoveryChannel, metric: &str) {
        if let Some(s) = &self.shared {
            s.incr_channel(channel.token(), metric, 1).await;
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
            trust,
            channel: xustive_core::DiscoveryChannel::Seed,
            priority: frontier::priority_for(0, trust, false),
        };
        let added = matches!(self.frontier.add(&pending).await, Ok(true));
        if added {
            self.publish_channel(xustive_core::DiscoveryChannel::Seed, "discovered")
                .await;
        }
        added
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

        // Pages that have come due rejoin their host queue. On the same cadence as reclamation and
        // for the same reason: a page due a second ago is not urgent, and a range query on the hot
        // path would cost more than the second it saves. Bounded per sweep so a corpus that all
        // comes due at once cannot stall the loop.
        if self.config.revisit && self.last_promote.elapsed() > PROMOTE_EVERY {
            let n = self.frontier.promote_due(now_ms, PROMOTE_BATCH).await;
            if n > 0 {
                tracing::info!(promoted = n, "pages came due for a revisit");
            }
            self.last_promote = std::time::Instant::now();
        }

        let Some(claim) = self.frontier.claim(now_ms, self.config.default_delay).await else {
            return Outcome::Idle;
        };

        let outcome = self.process(&claim).await;
        // Completed whatever happened. A URL that failed is not retried by leaving it claimed —
        // that would block its host until the claim expired, which punishes the host for our
        // problem. Retries belong to the frontier, as a fresh entry.
        self.frontier.complete(&claim.url).await;
        outcome
    }

    async fn process(&mut self, claim: &frontier::Claim) -> Outcome {
        let (url, host) = (claim.url.as_str(), claim.host.as_str());

        // A revisit carries the validators from last time, so an unchanged page answers 304 for a
        // few hundred bytes instead of a full body. Discovery has no history and sends none.
        let prior = match (&self.visits, self.config.revisit) {
            (Some(v), true) => v.get(url).await,
            _ => None,
        };
        let cond = crate::fetch::Conditional {
            etag: prior.as_ref().and_then(|v| v.etag.as_deref()),
            last_modified: prior.as_ref().and_then(|v| v.last_modified.as_deref()),
        };

        let fetched = match self.fetcher.get_conditional(url, cond).await {
            Ok(f) => f,
            Err(FetchError::RobotsDisallowed) => {
                self.stats.skip("robots");
                self.publish_skip("robots").await;
                self.publish_recent(url, host, "robots", 0).await;
                return Outcome::Idle;
            }
            Err(e) => {
                // Record the specific outcome, not a lone "failed": a rise in `gone` means sites
                // removing content, `throttled` means we are being rate-limited, `transient` means
                // the network — three different problems the operator needs told apart (M2-T04.5).
                self.stats.failed += 1;
                let outcome = e.outcome();
                self.stats.skip(outcome);
                tracing::debug!(%url, outcome, error = %e, "fetch failed");
                self.publish("failed").await;
                self.publish_skip(outcome).await;
                self.publish_source(&claim.source_id, "failed", 1).await;
                self.publish_recent(url, host, outcome, 0).await;
                return Outcome::Idle;
            }
        };
        self.stats.fetched += 1;
        self.publish("fetched").await;
        self.publish_source(&claim.source_id, "fetched", 1).await;
        self.publish_channel(claim.channel, "fetched").await;
        // A claim with prior visit state is a revisit; without, it is fresh discovery. This is the
        // same signal the conditional request used above, reused so the two can never disagree.
        if prior.is_some() {
            self.stats.revisited += 1;
            self.publish("revisited").await;
        }

        // Keep the raw body for later reindexing, when a store is attached (M2-T04.7). A 304 has no
        // body, so only a real 200 is stored. Best-effort — losing it costs a reindex convenience,
        // never a document.
        if fetched.status != 304 {
            if let Some(store) = &self.raw_store {
                store.put(&fetched.final_url, &fetched.body).await;
            }
        }

        if fetched.status == 304 {
            // The best possible outcome of a revisit: the page is exactly what we hold, learned
            // without transferring it. Not a document — the index already has it — so the loop
            // reports Idle, but the scheduler still gets its observation, or 304s would look like
            // silence and the interval would stop adapting.
            self.stats.skip("not_modified");
            self.publish_skip("not_modified").await;
            self.schedule_revisit(claim, None, None, None).await;
            return Outcome::Idle;
        }

        // The header the site sent, which is the only way a document without a `<head>` can refuse
        // indexing. Checked before parsing so we do not spend the work.
        if fetched
            .exclusion
            .is_some_and(|e| e.blocks_indexing() && e.blocks_links())
        {
            self.stats.skip("x_robots_tag");
            return Outcome::Idle;
        }

        let mut parsed = match self.parser.parse(
            &fetched.body,
            &fetched.final_url,
            "crawl",
            xustive_core::SourceType::Web,
        ) {
            Ok(p) => p,
            Err(ParseError::TooLittleContent { outlinks, .. }) => {
                // Not indexed, but still walked through. Listing and category pages are mostly
                // links and little prose, so they land here almost by definition — and they are
                // where most new URLs come from. Returning here without queuing them made the
                // crawler refuse to follow precisely the pages that exist to be followed.
                self.stats.skip("thin");
                self.publish_skip("thin").await;
                self.publish_source(&claim.source_id, "thin", 1).await;
                self.publish_recent(url, host, "thin", 0).await;
                if !fetched.exclusion.is_some_and(|e| e.blocks_links()) {
                    self.enqueue_outlinks(&outlinks, &claim.source_id, claim)
                        .await;
                }
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

        // Stamp the document with the channel that discovered its URL (M2-T16.7). The parser does
        // not know this — it is a property of how the URL reached the frontier, carried on the
        // claim — so it is set here, the one place both the claim and the document are in hand.
        parsed.document.discovery = claim.channel;

        // Links are followed unless the page asked otherwise. A `noindex` page is still worth
        // crawling *through* — that combination is how sites let a crawler reach content behind a
        // section they do not want listed.
        if !fetched.exclusion.is_some_and(|e| e.blocks_links()) {
            self.enqueue_outlinks(&parsed.outlinks, &parsed.document.source_id, claim)
                .await;
        }

        if fetched.exclusion.is_some_and(|e| e.blocks_indexing()) {
            self.stats.skip("noindex_header");
            return Outcome::Idle;
        }

        self.documents += 1;
        let words = parsed.document.body.split_whitespace().count();
        self.publish_recent(&fetched.final_url, host, "indexed", words)
            .await;
        self.schedule_revisit(
            claim,
            Some(&parsed.document.content_hash),
            fetched.etag.clone(),
            fetched.last_modified.clone(),
        )
        .await;
        Outcome::Document(Box::new(parsed))
    }

    /// Record what this fetch told us and book the next visit ([[ADR-0011]]).
    ///
    /// Keyed on `content_hash`, which is BLAKE3 over the *extracted* body — so a page whose
    /// sidebars and view counters changed while its article did not reads as unchanged and backs
    /// off, which is the entire point of scheduling on longevity rather than on byte difference.
    ///
    /// Silent when there is no store, and silent on a write failure. A crawl that cannot reach
    /// Redis should still crawl; the cost of losing this is that the page looks unvisited next time
    /// and is fetched at its floor, which is the safe direction.
    /// `content_hash` is `None` on a 304, which has no body to hash.
    async fn schedule_revisit(
        &mut self,
        claim: &frontier::Claim,
        content_hash: Option<&str>,
        etag: Option<String>,
        last_modified: Option<String>,
    ) {
        if !self.config.revisit {
            return;
        }
        let Some(visits) = &self.visits else {
            return;
        };

        let mut visit = visits.get(&claim.url).await.unwrap_or_default();
        let observation = match content_hash {
            Some(hash) => Observation::from_hashes(&visit.content_hash, hash),
            None => Observation::NotModified,
        };
        let now = xustive_core::now_unix();
        let decision = visit.record(observation, claim.trust, content_hash.unwrap_or(""), now);

        // Overwritten only when the server sent new ones. A 304 sends none, and clearing the old
        // validators on it would make the *next* request unconditional — paying full price
        // precisely because the last visit was free.
        if etag.is_some() {
            visit.etag = etag;
        }
        if last_modified.is_some() {
            visit.last_modified = last_modified;
        }

        visits.put(&claim.url, &visit).await;

        if decision.is_volatile() {
            self.stats.skip("volatile");
        }

        // Scored as a revisit, not as a discovery. The scheduler has measured how often this page
        // changes, and that measurement is better evidence than anything its URL suggests — so the
        // interval it converged on, and how overdue the page is, both feed the priority.
        //
        // Computed here because this is the only place both are known: the frontier sees a
        // `Pending` and has no history, and the discovery path has no interval at all.
        let due_at = visit.due_at();
        let overdue = now.saturating_sub(due_at).max(0);
        let pending = Pending {
            url: claim.url.clone(),
            host: claim.host.clone(),
            source_id: claim.source_id.clone(),
            depth: claim.depth,
            trust: claim.trust,
            // A revisit does not change how the URL was originally found.
            channel: claim.channel,
            priority: frontier::priority_for_revisit(
                claim.depth,
                claim.trust,
                visit.interval_secs,
                overdue,
            ),
        };
        self.frontier
            .defer(&pending, due_at.saturating_mul(1_000))
            .await;
    }

    async fn enqueue_outlinks(
        &mut self,
        links: &[String],
        source_id: &str,
        from: &frontier::Claim,
    ) {
        // One hop further than the page we just fetched. Previously this was the constant 1, which
        // made the check below compare 1 against a limit of 3 — always false, so `max_depth` never
        // fired and every URL in the frontier scored identically. The crawler had no breadth-first
        // ordering at all: it would follow a single site's archive as readily as a new homepage.
        // Record the domain link graph before any filtering — an off-site link we would never
        // enqueue is exactly the cross-domain vote PageRank is built on, and a page's links are a
        // vote whether or not it was deep enough to follow.
        if let Some(graph) = &self.link_graph {
            let targets: Vec<String> = links
                .iter()
                .filter_map(|l| url::Url::parse(l).ok())
                .filter_map(|u| u.host_str().map(str::to_string))
                .collect();
            graph.record(&from.host, &targets).await;
        }

        let depth = from.depth + 1;
        if depth > self.config.max_depth {
            self.stats.skip("too_deep");
            return;
        }

        for link in links {
            let Ok(u) = url::Url::parse(link) else {
                continue;
            };
            let host = u.host_str().unwrap_or_default().to_string();
            if host.is_empty() {
                continue;
            }

            // Off-site links are dropped unless discovery is on. Following them is the difference
            // between crawling twenty sources and crawling the web.
            if host != from.host
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

            // Trust is inherited from the page that linked here, rather than the flat 50 this used
            // before. A link off a tier-A ministry site is a better bet than one off a page we
            // reached by accident, and with depth now real the two compose the way priority
            // intends.
            let pending = Pending {
                url: frontier::canonical(&u),
                host,
                source_id: source_id.to_string(),
                depth,
                trust: from.trust,
                // Reached by following a link, whatever channel first found the parent.
                channel: xustive_core::DiscoveryChannel::Link,
                priority: frontier::priority_for(
                    depth,
                    from.trust,
                    frontier::looks_like_article(u.path()),
                ),
            };
            match self.frontier.add(&pending).await {
                Ok(true) => {
                    self.stats.discovered += 1;
                    self.publish("discovered").await;
                    self.publish_channel(xustive_core::DiscoveryChannel::Link, "discovered")
                        .await;
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
