//! Layered configuration: defaults → `config/{env}.toml` → environment variables.
//!
//! No component reads `std::env` directly. Each receives a typed struct at construction, so a
//! config mistake is a startup failure with a clear message rather than a surprise at runtime.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Classify, ErrorClass};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read config file {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid config in {path}: {source}")]
    Parse {
        path: String,
        source: toml::de::Error,
    },
    #[error("invalid value for {key}: {msg}")]
    Value { key: String, msg: String },
    #[error(
        "{field} is set to an unsafe value for the {environment} environment \
         (abusive crawling or a de-anonymising floor); refusing to start"
    )]
    Unsafe {
        field: &'static str,
        environment: String,
    },
}

impl Classify for ConfigError {
    fn class(&self) -> ErrorClass {
        // Bad config at startup is not something to retry around.
        ErrorClass::Fatal
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiConfig {
    pub bind_addr: String,
    /// Global in-flight cap. Requests beyond this are shed with 503.
    pub max_concurrent: usize,
    pub body_limit_default: usize,
    pub timeout_search_ms: u64,
    pub timeout_suggest_ms: u64,
    /// Empty means same-origin only.
    pub cors_origins: Vec<String>,
    /// Directory of built UI assets served at `/`.
    pub static_dir: String,
    /// Key required by `/admin`. Empty restricts the admin surface to loopback callers.
    pub admin_key: String,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:8080".into(),
            max_concurrent: 512,
            body_limit_default: 8 * 1024,
            timeout_search_ms: 1_500,
            timeout_suggest_ms: 150,
            cors_origins: Vec::new(),
            static_dir: "web/public".into(),
            admin_key: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    pub meili_url: String,
    pub meili_key: String,
    pub documents_index: String,
    pub comments_index: String,
    /// Candidates pulled from the engine before in-process re-ranking.
    pub candidate_pool: usize,
    pub default_hits_per_page: usize,
    pub max_hits_per_page: usize,
    pub timeout_ms: u64,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            meili_url: "http://localhost:7700".into(),
            meili_key: String::new(),
            documents_index: "documents".into(),
            comments_index: "comments".into(),
            candidate_pool: 200,
            default_hits_per_page: 20,
            max_hits_per_page: 50,
            timeout_ms: 800,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TelemetryConfig {
    /// `RUST_LOG`-style filter.
    pub log_filter: String,
    /// JSON lines in production; pretty in development.
    pub log_json: bool,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            log_filter: "info,xustive=debug".into(),
            log_json: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct QueueConfig {
    pub url: String,
    /// Stream carrying documents waiting to be indexed.
    pub index_stream: String,
    /// Redis holding the **behavioural signal stores** — interaction counters and weak-coverage
    /// terms (BUG-034). Separate from `url` on purpose: the queue Redis must persist (AOF — losing
    /// it loses the crawl frontier), but persistence turns the signal namespaces into an *ordered,
    /// durable* command log that chains qhash↔plaintext writes and outlives the sliding window in
    /// backups — the exact properties ADR-0018 forbids. Point this at an instance running with
    /// `--save '' --appendonly no` that no backup touches (`redis-signals` in compose). Empty falls
    /// back to `url`, which keeps a one-Redis dev box working but re-inherits its persistence.
    pub signals_url: String,
    /// Entry cap on the index stream (`XADD MAXLEN ~`). The cap that matters is **bytes** — each
    /// entry is a full document, so the old 100k-entry cap allowed ~900MB of stream on a 1GB Redis
    /// (PROB-001's actual OOM). 20k entries ≈ 100–200MB at typical document sizes; the crawler's
    /// backpressure holds the working depth far below this, so the cap is the runaway backstop.
    #[serde(default = "default_index_stream_max_len")]
    pub index_stream_max_len: usize,
}

fn default_index_stream_max_len() -> usize {
    20_000
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            url: "redis://127.0.0.1:6390".into(),
            index_stream: "q:index".into(),
            signals_url: String::new(),
            index_stream_max_len: default_index_stream_max_len(),
        }
    }
}

impl QueueConfig {
    /// The Redis the signal stores (interaction, weak-coverage) connect to: the dedicated
    /// ephemeral instance when configured, else the queue Redis (see [`QueueConfig::signals_url`]).
    pub fn signals_url(&self) -> &str {
        let s = self.signals_url.trim();
        if s.is_empty() {
            &self.url
        } else {
            s
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CrawlConfig {
    /// Honour `Crawl-delay` and the adaptive pacing derived from it.
    pub respect_crawl_delay: bool,
    /// Simultaneous requests to one host. One, always, in production.
    pub per_host_concurrency: u32,

    /// **Ignore politeness entirely.** Testing only.
    ///
    /// When on, the crawler does not fetch or consult `robots.txt`, does not wait between
    /// requests to a host, ignores adaptive slowdown from 429 and 503, and ignores the host
    /// opt-out list. It exists so a fixture site can be crawled at full speed without a robots
    /// round trip per test.
    ///
    /// **Not** bypassed: the global and takedown blocklists. Those are not politeness — one is a
    /// safety block and the other is a legal order, and a testing flag must not be able to lift a
    /// court order. Nothing about a local fixture site needs them lifted.
    ///
    /// Pointed at the open web this produces exactly the behaviour the politeness layer exists to
    /// prevent, so it is loud: a warning on every startup, a counter, and a banner on the admin
    /// page. `config_guard` refuses to let it ship on in production.
    pub ignore_politeness: bool,
    /// The seed list. Read and written by the admin console, read by the crawler.
    pub seeds_path: String,
    /// The data sources registry (JSON-Lines). Read by the crawler for its approved, active
    /// sources, and by the admin console's per-source quality dashboard (M2-T11.5).
    #[serde(default = "default_registry_path")]
    pub registry_path: String,
    /// Rows per page in the admin document list.
    ///
    /// Paged rather than "all": a list that loads everything is fine at a thousand documents and
    /// unusable at a million, and that failure arrives exactly when the crawler starts working.
    pub documents_page_size: usize,
    /// Keep the raw fetched body for this many days, so extraction can be re-run without a
    /// re-fetch (M2-T04.7). **Zero disables it** — the default, because blanket storage would
    /// overwhelm the small development Redis, and the real home is object storage.
    #[serde(default)]
    pub raw_ttl_days: u64,

    // ── Frontier growth bounds (PROB-001). Every knob here exists so the crawl can NEVER fill
    // Redis again: the frontier is a working set with a global ceiling, per-host lifetime budgets,
    // and a self-expiring seen-set — growth is linear and bounded by construction, not by hope.
    /// Global ceiling on URLs queued across all hosts. At the ceiling the frontier **evicts its
    /// worst-priority tail** to admit new discoveries — bounding by dropping the least promising,
    /// never by refusing the newest (the Heritrix/Nutch behaviour). ~200k URLs ≈ 120–150 MB.
    #[serde(default = "default_frontier_max_urls")]
    pub frontier_max_urls: usize,
    /// Lifetime page budget per host: once this many pages have been *crawled* from a host, new
    /// discoveries for it are dropped (revisits continue). Resets over roughly two seen-set
    /// rotations, so a large site is revisited across months rather than swallowing the crawl in
    /// one sitting. **0 = unlimited.**
    #[serde(default = "default_max_pages_per_host")]
    pub max_pages_per_host: u64,
    /// How many of a page's outlinks may enter the frontier — the **best-scoring K**, not the
    /// first K. This is the branching factor of the whole crawl: Nutch ships 100 as its default
    /// defence; 64 with priority selection keeps growth linear where 200-unfiltered was cubic.
    #[serde(default = "default_max_outlinks_per_page")]
    pub max_outlinks_per_page: usize,
    /// The seen-set rotation window, in days. URL dedup lives in generational sets that expire
    /// after two windows, so "every URL ever seen" stops being a forever-growing structure: memory
    /// is bounded by two windows of discovery, and a URL not re-encountered within them may be
    /// crawled afresh — which the revisit and content-dedup layers absorb.
    #[serde(default = "default_seen_rotate_days")]
    pub seen_rotate_days: u64,
}

fn default_frontier_max_urls() -> usize {
    200_000
}
fn default_max_pages_per_host() -> u64 {
    20_000
}
fn default_max_outlinks_per_page() -> usize {
    64
}
fn default_seen_rotate_days() -> u64 {
    45
}

fn default_registry_path() -> String {
    "data/sources/registry.jsonl".into()
}

/// Query-driven discovery (M2-T16.4). **Off by default**, and constrained by
/// [[ADR-0008 - No Query Logging]]: a search that comes up short is a signal of weak coverage, but
/// the query text is personal data. So this is opt-in, and what it records is k-anonymous — a term
/// is only ever surfaced once at least `k_anonymity` searches have hit it, at which point it is
/// demonstrably common rather than personal.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DiscoveryConfig {
    /// Master switch. Off means nothing about a search is recorded, anywhere.
    #[serde(default)]
    pub weak_coverage_enabled: bool,
    /// A search returning at most this many results is "weak coverage" worth noting.
    #[serde(default = "default_weak_floor")]
    pub weak_coverage_result_floor: usize,
    /// k-anonymity threshold: a term is never surfaced until it has been seen at least this often.
    /// The ADR mandates k ≥ 20 on any non-dev deployment, **enforced by `Config::validate()`** —
    /// the loader refuses to start rather than clamping silently (BUG-035; an earlier comment
    /// claimed a clamp that no code performed). k = 1 is single-operator dev only.
    #[serde(default = "default_k_anonymity")]
    pub k_anonymity: u32,
    /// Sliding window, in days. A term must reach `k_anonymity` within this window or it decays and
    /// is forgotten — which bounds retention and stops a rare query slowly accreting to the floor.
    #[serde(default = "default_weak_window_days")]
    pub weak_coverage_window_days: u64,

    /// The Brave Search API connector (M2-T16.6): resolve weak-coverage terms to URLs. **Off by
    /// default** and inert without a key — it is the one paid discovery route whose terms permit
    /// this, tried before direct SERP collection.
    #[serde(default)]
    pub brave_enabled: bool,
    /// Brave subscription token. Empty disables the connector even if `brave_enabled` is set.
    #[serde(default)]
    pub brave_api_key: String,
    /// Hard cap on Brave queries per resolver run — the budget. Brave is paid per query, so this is
    /// the spend ceiling, not a nicety.
    #[serde(default = "default_brave_max_queries")]
    pub brave_max_queries_per_run: usize,
    /// Results to request per query. Discovery wants a handful of good URLs, not a full page.
    #[serde(default = "default_brave_results")]
    pub brave_results_per_query: usize,

    /// Direct SERP scraping (M2-T16.9, [[ADR-0013]]): resolve weak terms by reading a general search
    /// engine's results page rather than an API. **Off by default.** When on, this is preferred over
    /// Brave. Google needs the residential-proxy/headless layer; Bing and DuckDuckGo's HTML endpoint
    /// often work without it.
    #[serde(default)]
    pub serp_enabled: bool,
    /// Engines to try, in order (`duckduckgo`, `bing`, `google`). Empty means the built-in ladder.
    #[serde(default)]
    pub serp_engines: Vec<String>,
    /// Hard cap on SERP queries per resolver run.
    #[serde(default = "default_brave_max_queries")]
    pub serp_max_queries_per_run: usize,
    /// Proxy the SERP fetches route through. Empty means a direct connection — which a datacentre IP
    /// gets challenge pages on, so results stay empty until this points at a (typically rotating,
    /// residential) proxy. Accepts `http://`, `https://`, or `socks5://` URLs, with optional
    /// credentials inline (`http://user:pass@host:port`). Put real credentials here, not in code.
    #[serde(default)]
    pub serp_proxy: String,
}

fn default_weak_floor() -> usize {
    3
}
fn default_k_anonymity() -> u32 {
    20
}
fn default_weak_window_days() -> u64 {
    30
}
fn default_brave_max_queries() -> usize {
    50
}
fn default_brave_results() -> usize {
    10
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            // On by default: recording which searches came up short is the signal the whole
            // discovery loop runs on, it is k-anonymous, and it stores no query text below the floor.
            weak_coverage_enabled: true,
            weak_coverage_result_floor: default_weak_floor(),
            k_anonymity: default_k_anonymity(),
            weak_coverage_window_days: default_weak_window_days(),
            brave_enabled: false,
            brave_api_key: String::new(),
            brave_max_queries_per_run: default_brave_max_queries(),
            brave_results_per_query: default_brave_results(),
            serp_enabled: false,
            serp_engines: Vec::new(),
            serp_max_queries_per_run: default_brave_max_queries(),
            serp_proxy: String::new(),
        }
    }
}

impl DiscoveryConfig {
    /// The k-anonymity floor: how many searches must hit a term before it is ever surfaced or
    /// acted on. The **default is 20** ([[ADR-0008]]), which is what makes weak-coverage counting
    /// safe on a public, multi-user engine — a term surfaced there is common, not personal.
    ///
    /// It can be lowered (down to 1) only by an explicit config value, because on a **single-user /
    /// personal deployment** there is no one to anonymise against: the operator is the only searcher,
    /// so a floor of 20 just means the feature never triggers. Lowering it below 20 is therefore
    /// appropriate *only* for such a deployment — on anything public it re-opens exactly what the ADR
    /// closed. The floor is 1, not 0, so a term must still have been searched at least once.
    pub fn effective_k(&self) -> u32 {
        self.k_anonymity.max(1)
    }

    /// Whether the Brave connector is actually usable: switched on *and* holding a key. Both are
    /// required — a flag with no key is a misconfiguration that should stay inert, not error.
    pub fn brave_usable(&self) -> bool {
        self.brave_enabled && !self.brave_api_key.trim().is_empty()
    }
}

/// Query-time federation with a self-hosted metasearch aggregator ([[ADR-0017]], [[Federation
/// Gateway]]).
///
/// Unlike [[DiscoveryConfig]]'s Brave/SERP routes — which resolve *weak* terms offline on the
/// ingestion plane — federation borrows recall for a **live** query: a self-hosted SearXNG returns a
/// ranked, multi-source URL+snippet list, blended into the answer and fed to the crawler so the
/// index converges to answering alone. **Off by default**, and inert without an endpoint. Reaching
/// the aggregator is the [[Federation Gateway]]'s job — the serving plane never talks to SearXNG
/// directly, so this config is read by the gateway/crawler, and by the API only to *show and toggle*
/// the feature.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct FederationConfig {
    /// Master switch. Off means no federation call is made, anywhere.
    pub enabled: bool,
    /// The self-hosted SearXNG JSON endpoint (e.g. `http://xustive-searxng:8080`). Empty leaves the
    /// feature inert even when `enabled` — like an empty API key, a flag with no endpoint stays off
    /// rather than erroring. Read by the gateway (which reaches SearXNG); the serving plane never
    /// uses this directly.
    pub searxng_url: String,
    /// The [[Federation Gateway]]'s `core`-side URL, which the serving API calls at query time (e.g.
    /// `http://xustive-federator:8095`). This is the *only* new outbound target the API gains, and it
    /// is an internal address — the API still has no route to the open internet ([[ADR-0017]]). Empty
    /// means the API does not federate even when `enabled`.
    pub federator_url: String,
    /// How long the search response waits for the "from the web" strip, in milliseconds. Federation
    /// runs concurrently with local retrieval; if the gateway has not answered within this, the
    /// response ships without the strip and the background fetch keeps going (indexing the results
    /// for the next search). This is the user-visible latency cap, not the fetch timeout.
    pub budget_ms: u64,
    /// How long the **background** federation fetch may run, in milliseconds — the real time a
    /// metasearch aggregation needs (seconds), independent of the response budget. The fetch runs
    /// detached, so this never delays a search; it only bounds how long we wait for SearXNG before
    /// giving up on indexing this query's results.
    pub fetch_budget_ms: u64,
    /// Hits to request per federated query. A handful of good URLs, not a full page.
    pub max_hits: usize,
    /// **Eager indexing** (M7): index each federated result *immediately* as a thin document (its
    /// SearXNG title + snippet) so it appears as a real result within seconds, instead of waiting for
    /// the crawler to fetch the full page. The full crawl still runs and overwrites the thin document
    /// with the real page (same URL-derived id). Off by default — it puts external, un-crawled text
    /// into the index, which is a deliberate trade of quality for immediacy.
    pub eager_index: bool,
}
// Note on gateway egress (BUG-004): there is no runtime allowlist. The gateway's reach is bounded
// *topologically* — it holds exactly two outbound clients, pointed at the endpoints its own
// environment names (`SEARXNG_URL`, `EXTERNAL_LLM_URL`), and nothing reads any other destination.
// An earlier `allowlist` field claimed deny-by-default enforcement that no code performed; a config
// knob that promises a control it does not exert is worse than stating the real boundary.

fn default_federation_budget_ms() -> u64 {
    // The response's strip wait. Long enough that a reasonably quick SearXNG shows the strip live,
    // short enough that a slow one does not stall the search — the background fetch indexes it either
    // way, so this is only about the *live* strip.
    1500
}
fn default_federation_fetch_budget_ms() -> u64 {
    // A real metasearch aggregation over several engines takes seconds. This runs detached from the
    // response, so it can be generous without ever slowing a search.
    6000
}
fn default_federation_max_hits() -> usize {
    10
}

impl Default for FederationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            searxng_url: String::new(),
            federator_url: String::new(),
            budget_ms: default_federation_budget_ms(),
            fetch_budget_ms: default_federation_fetch_budget_ms(),
            max_hits: default_federation_max_hits(),
            eager_index: false,
        }
    }
}

impl FederationConfig {
    /// Whether federation is actually usable by the gateway: switched on *and* holding a SearXNG
    /// endpoint. Both are required — a flag with no endpoint is a misconfiguration that stays inert,
    /// not an error.
    pub fn searxng_usable(&self) -> bool {
        self.enabled && !self.searxng_url.trim().is_empty()
    }

    /// Whether the *serving API* should federate: switched on *and* holding a gateway URL to call.
    /// Distinct from [`searxng_usable`] — the API talks to the gateway, never to SearXNG.
    pub fn api_federation_usable(&self) -> bool {
        self.enabled && !self.federator_url.trim().is_empty()
    }
}

/// Anonymous interaction signals ([[ADR-0015]], [[Interaction Signals]]). Impressions and clicks as
/// k-anonymous, windowed Redis counters — never tied to a person — feeding ranking and re-crawl.
///
/// **Off by default.** It amends the no-click-tracking promise of ADR-0008, so it must be an explicit
/// choice, never a silent one. When on, `k_anonymity` is the ADR-0008 floor of 20 on any multi-user
/// deployment; a single-user dev box may lower it, understanding that means "no anonymity, one
/// operator" rather than "anonymised".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InteractionConfig {
    pub enabled: bool,
    /// k-anonymity floor: a (query, doc) or query signal is used only above this many distinct
    /// searches. 20 is the ADR-0008 floor; 1 is single-user-dev only.
    pub k_anonymity: u32,
    /// Sliding retention. CTR reflects roughly the last quarter.
    pub window_days: u64,
    /// Clicks on a (query, doc) before the doc becomes a re-crawl freshness candidate. 0 = use `k`.
    pub hot_click_floor: u32,
    /// Deploy salt keying the query hash in `qd:`/`qk:` counter keys (BUG-036). With a salt, the
    /// hash is keyed blake3 — unguessable without it, so the stored keys resist dictionary
    /// reversal. Empty falls back to unsalted FNV, which only keeps plaintext out of the key bytes;
    /// **validation refuses an empty salt outside dev** when interaction is enabled. Also settable
    /// as `XUSTIVE_QHASH_SALT`. Rotating it orphans the windowed `qd:`/`qk:` counters — they decay
    /// out within the window, so rotation costs at most one window of click signal.
    pub salt: String,
}

// The `interaction` ranking *weight* deliberately lives in the ranker's `Weights` (config/ranking.toml),
// not here — it is a ranking concern, and keeping it out lets this config derive `Eq` alongside the
// rest of `Config`.

impl Default for InteractionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            k_anonymity: 20,
            window_days: 90,
            hot_click_floor: 0,
            salt: String::new(),
        }
    }
}

/// `[collection]` — first-party search events ([[ADR-0030]]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CollectionConfig {
    /// Keep search, click and report events with a first-party visitor id. No k floor: this
    /// is the operator's own data under the operator's own lawful basis.
    pub enabled: bool,
    /// Events older than this are deleted by `xustive events sweep`.
    pub retention_days: u64,
}

impl Default for CollectionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            retention_days: 365,
        }
    }
}

impl InteractionConfig {
    /// The effective k floor (never below 1).
    pub fn effective_k(&self) -> u32 {
        self.k_anonymity.max(1)
    }

    /// The click floor for re-crawl candidacy, defaulting to the k floor when unset.
    pub fn hot_floor(&self) -> u32 {
        if self.hot_click_floor == 0 {
            self.effective_k()
        } else {
            self.hot_click_floor
        }
    }

    /// Refuse to start with a sub-20 k-anonymity floor outside `dev` (M6-T01.4). This is a
    /// **structural** guarantee, not a convention: the ADR-0008 escape hatch is "k-anonymous, k ≥ 20",
    /// so a multi-user deployment cannot silently run with a floor that de-anonymises the counts. A
    /// single-user dev box may set k=1, which honestly means "one operator, no anonymity".
    pub fn guard(&self, environment: &str) -> Result<(), ConfigError> {
        if self.enabled && environment != "dev" && self.k_anonymity < 20 {
            return Err(ConfigError::Unsafe {
                field: "interaction.k_anonymity",
                environment: environment.to_string(),
            });
        }
        Ok(())
    }
}

/// Index-side image enrichment ([[Enrichment Pipeline]], M3-T07): fetch a crawled page's images and
/// OCR them so the text inside an image becomes searchable.
///
/// **Off by default** — it adds a network fetch and CPU-bound OCR per image, so it is opt-in and
/// heavily bounded. A failed image never fails its document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaConfig {
    /// Master switch for image OCR enrichment.
    pub image_ocr_enabled: bool,
    /// Directory holding the tesseract `*.traineddata` files.
    pub tessdata_dir: String,
    /// `+`-joined OCR languages, Arabic first for Algerian screenshots.
    pub ocr_langs: String,
    /// Most images OCR'd per document — the cost ceiling per page.
    pub max_images_per_doc: usize,
    /// Largest image fetched, in bytes.
    pub max_image_bytes: usize,
    /// Which engine the **user-facing** OCR tools use: `"tesseract"` (in-process, CPU, always
    /// available) or `"unlimited"` (the Unlimited-OCR sidecar, a GPU vision-language model). The
    /// crawl-time enrichment path always uses tesseract regardless — it runs over every image and
    /// must fit the CPU-only reference hardware. `"unlimited"` falls back to tesseract when the
    /// sidecar is unreachable, so selecting it never breaks the tools.
    pub ocr_backend: String,
    /// The optional Unlimited-OCR sidecar.
    pub sidecar: SidecarConfig,
}

/// The Unlimited-OCR sidecar: a Python/GPU service on the private network, reached over HTTP. This
/// is an *internal-service* call, not internet egress — the serving plane stays sealed ([[ADR-0001
/// Two-Plane Architecture]]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SidecarConfig {
    /// Full URL of the sidecar's OCR endpoint, e.g. `http://127.0.0.1:8091/ocr`.
    pub endpoint: String,
    /// Hard per-request timeout, milliseconds. A wedged 3 B VLM must fail over in bounded time.
    pub timeout_ms: u64,
}

impl Default for SidecarConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://127.0.0.1:8091/ocr".into(),
            timeout_ms: 30_000,
        }
    }
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            image_ocr_enabled: false,
            tessdata_dir: "data/tessdata".into(),
            ocr_langs: "ara+fra+eng".into(),
            max_images_per_doc: 3,
            max_image_bytes: 5 * 1024 * 1024,
            ocr_backend: "tesseract".into(),
            sidecar: SidecarConfig::default(),
        }
    }
}

/// Image-similarity vector search: Qdrant plus the CLIP embedder ([[Vector Index]] C07, M3-T05).
///
/// Off by default — it needs both Qdrant reachable and a CLIP embedder service. Unlike the OCR
/// sidecar, CLIP ViT-B/32 is small enough to run CPU-only, so the visual-similarity feature is not
/// GPU-gated; it is opt-in only because it needs its model and index provisioned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VectorConfig {
    /// Master switch for image-similarity search.
    pub enabled: bool,
    /// Qdrant base URL. On the internal network; the serving plane reaches it like Meilisearch.
    pub qdrant_url: String,
    /// Optional Qdrant API key.
    pub qdrant_key: String,
    /// Collection name.
    pub collection: String,
    /// CLIP embedder endpoint (the embed sidecar). An image posted here returns a 512-d vector.
    pub embedder_endpoint: String,
    /// Hard per-request timeout for both Qdrant and the embedder, milliseconds.
    pub timeout_ms: u64,
    /// Results requested from Qdrant before collapsing by document.
    pub search_limit: usize,
    /// HNSW `ef` at search time — recall vs latency (64 default).
    pub ef_search: usize,
    /// Cosine score below which a match is dropped: below this, "no similar images". Stored as
    /// per-mille (750 = 0.75) so the whole config can stay `Eq`; use [`VectorConfig::score_threshold`].
    pub score_threshold_milli: u32,
    /// TTL, in days, for the `phash → vector` reuse cache. 0 disables the cache (every image is
    /// embedded). Entries age out so a hash that stops recurring does not pin memory forever.
    pub embed_cache_ttl_days: u64,

    // --- semantic text search (M7-T02), a parallel path over the same Qdrant ---
    /// Master switch for semantic (dense) text retrieval. Independent of image search: it needs the
    /// **text**-embed sidecar and its own Qdrant collection. Off by default.
    pub text_enabled: bool,
    /// The text-embed sidecar endpoint. A batch of strings posted here returns one `text_dim` vector
    /// each. An internal-network call, not internet egress.
    pub text_embedder_endpoint: String,
    /// Qdrant collection for document text vectors — separate from the image collection.
    pub text_collection: String,
    /// Vector dimension of the text model. **Must match the sidecar's model** (bge-m3 = 1024); the
    /// collection is created with this size and a mismatched vector is rejected at upsert time.
    pub text_dim: usize,
    /// Nearest neighbours pulled from the text collection per query, before fusion with the lexical
    /// candidates. The dense leg's recall budget.
    pub text_search_limit: usize,
}

impl VectorConfig {
    /// The cosine score threshold as a fraction (0.0–1.0).
    pub fn score_threshold(&self) -> f32 {
        self.score_threshold_milli as f32 / 1000.0
    }
}

/// Speech-to-text for voice search ([[Speech to Text]], M3-T02). Off by default; needs the STT
/// sidecar and its model. Whisper `small` runs CPU-only, so voice is not GPU-gated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SttConfig {
    /// Master switch for voice search.
    pub enabled: bool,
    /// The STT sidecar's transcribe endpoint. An internal-network HTTP call, not internet egress.
    pub endpoint: String,
    /// Hard per-request timeout, milliseconds — a wedged model must fail in bounded time.
    pub timeout_ms: u64,
    /// Largest audio accepted, bytes. A 30 s Opus clip is well under 1 MB; this is the ceiling.
    pub max_audio_bytes: usize,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: "http://127.0.0.1:8093/transcribe".into(),
            timeout_ms: 30_000,
            max_audio_bytes: 8 * 1024 * 1024,
        }
    }
}

impl Default for VectorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            qdrant_url: "http://127.0.0.1:6333".into(),
            qdrant_key: String::new(),
            collection: "image_clip".into(),
            embedder_endpoint: "http://127.0.0.1:8092/embed".into(),
            timeout_ms: 10_000,
            search_limit: 40,
            ef_search: 64,
            score_threshold_milli: 750,
            embed_cache_ttl_days: 30,
            text_enabled: false,
            text_embedder_endpoint: "http://127.0.0.1:8094/embed".into(),
            text_collection: "text_bge".into(),
            text_dim: 1024,
            text_search_limit: 50,
        }
    }
}

impl Default for CrawlConfig {
    fn default() -> Self {
        Self {
            respect_crawl_delay: true,
            per_host_concurrency: 1,
            // Off. The only safe default for a flag whose failure mode is being reported for abuse.
            ignore_politeness: false,
            seeds_path: "data/sources/seeds.tsv".into(),
            registry_path: default_registry_path(),
            documents_page_size: 50,
            raw_ttl_days: 0,
            frontier_max_urls: default_frontier_max_urls(),
            max_pages_per_host: default_max_pages_per_host(),
            max_outlinks_per_page: default_max_outlinks_per_page(),
            seen_rotate_days: default_seen_rotate_days(),
        }
    }
}

impl CrawlConfig {
    /// Refuse a configuration that would be abusive if deployed.
    ///
    /// Called at startup rather than left to review. The failure mode here is not a crash — it is
    /// a crawler that behaves impeccably in testing and hammers real sites in production, and
    /// nothing in the process's own behaviour reveals it.
    pub fn guard(&self, environment: &str) -> Result<(), ConfigError> {
        let production = matches!(environment, "prod" | "production" | "staging");
        if !production {
            return Ok(());
        }
        for (field, bad) in [
            ("ignore_politeness", self.ignore_politeness),
            ("respect_crawl_delay", !self.respect_crawl_delay),
            ("per_host_concurrency", self.per_host_concurrency != 1),
        ] {
            if bad {
                return Err(ConfigError::Unsafe {
                    field,
                    environment: environment.to_string(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SuggestConfig {
    /// Hand-written high-value suggestions. Missing is fine — it improves on the corpus rather
    /// than being required by it.
    pub curated_path: String,
    pub limit: usize,
    pub min_prefix_len: usize,
}

impl Default for SuggestConfig {
    fn default() -> Self {
        Self {
            curated_path: "data/suggest/curated.tsv".into(),
            limit: 8,
            min_prefix_len: 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MlConfig {
    /// Directory holding model files. Not baked into images: they are large and their licences
    /// differ, so the operator manages them.
    pub model_dir: String,
    /// `auto`, `gpu` or `cpu`. Changeable at runtime from the admin page.
    pub device: String,
    /// Layers to offload to the GPU. `-1` decides from free memory, `0` is CPU-only.
    pub gpu_layers: i64,
    /// Model id for the summariser, or empty to take the first present one.
    pub summariser_model: String,
    /// Concurrent generation slots. Memory-bound, not compute-bound.
    pub slots: usize,
    /// Wall-clock budget for one summary. Past this, the partial text is validated and either
    /// shown or dropped — the results page never waits on it.
    pub deadline_ms: u64,
    /// Kill switch. With this off, `/v1/summary` always answers "no summary" and no model loads.
    pub summaries_enabled: bool,
    /// Route summaries through the external LLM behind the Federation Gateway (M7-T08) before
    /// falling back to the local model. **Default off** — it is third-party SaaS: when on, the
    /// query text and result excerpts leave the deployment for the configured provider
    /// ([[ADR-0005]] keeps the local model the default; the privacy page documents the egress).
    /// Needs `federation.federator_url` plus the gateway's `EXTERNAL_LLM_*` environment.
    pub external_summaries: bool,
    /// Whether the model may write a one-line description for an entity that has facts but no
    /// encyclopedic paragraph (M8-T04).
    ///
    /// **Default off.** The panel is fully useful without it, the output is cached against the
    /// entity so the cost is once per entity rather than once per search, and everything it
    /// produces is validated against the stored claims — but it is still a model writing prose a
    /// reader will take as fact, and that should be a deliberate choice.
    #[serde(default)]
    pub knowledge_assist: bool,
}

impl Default for MlConfig {
    fn default() -> Self {
        Self {
            model_dir: "models".into(),
            device: "auto".into(),
            gpu_layers: -1,
            summariser_model: String::new(),
            slots: 2,
            deadline_ms: 30_000,
            summaries_enabled: true,
            external_summaries: false,
            knowledge_assist: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Which deployment this is: `dev`, `ci`, `staging`, `prod`.
    ///
    /// Explicit rather than inferred from the config filename. The safety guards key off this, and
    /// a guard that decides how careful to be by parsing a path is a guard that stops working the
    /// day someone renames a file or passes the config on stdin.
    ///
    /// Defaults to `dev` — the *stricter* direction is the one that must be opted into, so a
    /// missing value never accidentally grants production permissions.
    pub environment: String,
    pub api: ApiConfig,
    pub search: SearchConfig,
    pub telemetry: TelemetryConfig,
    pub ml: MlConfig,
    pub suggest: SuggestConfig,
    pub queue: QueueConfig,
    pub crawl: CrawlConfig,
    #[serde(default)]
    pub discovery: DiscoveryConfig,
    #[serde(default)]
    pub federation: FederationConfig,
    #[serde(default)]
    pub interaction: InteractionConfig,
    /// First-party search data — searches, results shown, opens, reports — kept as events
    /// ([[ADR-0030]], M11). Off by default: turning it on makes the operator a data controller.
    #[serde(default)]
    pub collection: CollectionConfig,
    #[serde(default)]
    pub media: MediaConfig,
    #[serde(default)]
    pub vector: VectorConfig,
    #[serde(default)]
    pub stt: SttConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            environment: "dev".into(),
            api: ApiConfig::default(),
            search: SearchConfig::default(),
            telemetry: TelemetryConfig::default(),
            ml: MlConfig::default(),
            suggest: SuggestConfig::default(),
            queue: QueueConfig::default(),
            crawl: CrawlConfig::default(),
            discovery: DiscoveryConfig::default(),
            federation: FederationConfig::default(),
            interaction: InteractionConfig::default(),
            collection: CollectionConfig::default(),
            media: MediaConfig::default(),
            vector: VectorConfig::default(),
            stt: SttConfig::default(),
        }
    }
}

impl Config {
    /// Load defaults, overlay a TOML file if present, then apply environment overrides.
    ///
    /// A missing file is not an error — defaults plus env is a valid configuration, which is
    /// what containers usually want.
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let mut cfg = match path {
            Some(p) if p.exists() => {
                let text = std::fs::read_to_string(p).map_err(|source| ConfigError::Read {
                    path: p.display().to_string(),
                    source,
                })?;
                toml::from_str(&text).map_err(|source| ConfigError::Parse {
                    path: p.display().to_string(),
                    source,
                })?
            }
            _ => Self::default(),
        };
        cfg.apply_env();
        cfg.validate()?;
        Ok(cfg)
    }

    /// Environment overrides for the handful of values that differ per deployment.
    ///
    /// Deliberately an explicit list rather than generic reflection: a typo'd env var should be
    /// visibly ignored, not silently reinterpreted.
    fn apply_env(&mut self) {
        if let Ok(v) = std::env::var("XUSTIVE_BIND_ADDR") {
            self.api.bind_addr = v;
        }
        if let Ok(v) = std::env::var("MEILI_URL") {
            self.search.meili_url = v;
        }
        if let Ok(v) = std::env::var("MEILI_KEY") {
            self.search.meili_key = v;
        }
        if let Ok(v) = std::env::var("XUSTIVE_STATIC_DIR") {
            self.api.static_dir = v;
        }
        if let Ok(v) = std::env::var("REDIS_URL") {
            self.queue.url = v;
        }
        if let Ok(v) = std::env::var("XUSTIVE_ADMIN_KEY") {
            self.api.admin_key = v;
        }
        if let Ok(v) = std::env::var("XUSTIVE_DEVICE") {
            self.ml.device = v;
        }
        if let Ok(v) = std::env::var("XUSTIVE_MODEL_DIR") {
            self.ml.model_dir = v;
        }
        if let Ok(v) = std::env::var("RUST_LOG") {
            self.telemetry.log_filter = v;
        }
        if let Ok(v) = std::env::var("XUSTIVE_LOG_JSON") {
            self.telemetry.log_json = matches!(v.as_str(), "1" | "true" | "yes");
        }
        // Lets `make dev --federation` start with federation on without editing the config file. The
        // URLs still come from the config; this only flips the switch.
        if let Ok(v) = std::env::var("XUSTIVE_FEDERATION_ENABLED") {
            self.federation.enabled = matches!(v.as_str(), "1" | "true" | "yes");
        }
        // The qhash deploy salt (BUG-036) — a secret, so the environment is its natural home.
        if let Ok(v) = std::env::var("XUSTIVE_QHASH_SALT") {
            self.interaction.salt = v;
        }
    }

    fn validate(&self) -> Result<(), ConfigError> {
        // k-anonymity floors, enforced where EVERY binary passes (BUG-035): the API-only
        // `interaction.guard()` left crawld and the CLI loading the same config with no check, and
        // discovery — which stores plaintext user terms and defaults ON — had no enforcement at
        // all despite a doc comment claiming a clamp. ADR-0015/0018: k ≥ 20 outside dev, always.
        if self.environment != "dev" {
            if self.interaction.enabled && self.interaction.k_anonymity < 20 {
                return Err(ConfigError::Unsafe {
                    field: "interaction.k_anonymity",
                    environment: self.environment.clone(),
                });
            }
            // `hot_click_floor == 0` means "use k" and stays legal; an explicit sub-20 floor
            // would surface per-doc click behaviour below the anonymity line.
            if self.interaction.enabled
                && self.interaction.hot_click_floor != 0
                && self.interaction.hot_click_floor < 20
            {
                return Err(ConfigError::Unsafe {
                    field: "interaction.hot_click_floor",
                    environment: self.environment.clone(),
                });
            }
            if self.discovery.weak_coverage_enabled && self.discovery.k_anonymity < 20 {
                return Err(ConfigError::Unsafe {
                    field: "discovery.k_anonymity",
                    environment: self.environment.clone(),
                });
            }
            // Without a salt the query hash is dictionary-reversible (BUG-036) — dev-only.
            if self.interaction.enabled && self.interaction.salt.trim().is_empty() {
                return Err(ConfigError::Value {
                    key: "interaction.salt".into(),
                    msg: "required outside dev: set it (or XUSTIVE_QHASH_SALT) so query hashes \
                          are keyed, not dictionary-reversible FNV"
                        .into(),
                });
            }
        }
        // Frontier bounds (PROB-001): zero or absurd values would silently disable the growth
        // guarantees, so they are refused rather than "interpreted".
        if self.crawl.frontier_max_urls < 1_000 {
            return Err(ConfigError::Value {
                key: "crawl.frontier_max_urls".into(),
                msg: "must be at least 1000 — the global frontier ceiling is a load-bearing bound"
                    .into(),
            });
        }
        if self.crawl.max_outlinks_per_page == 0 || self.crawl.max_outlinks_per_page > 200 {
            return Err(ConfigError::Value {
                key: "crawl.max_outlinks_per_page".into(),
                msg: "must be between 1 and 200".into(),
            });
        }
        if self.crawl.seen_rotate_days < 7 {
            return Err(ConfigError::Value {
                key: "crawl.seen_rotate_days".into(),
                msg: "must be at least 7 days — shorter windows re-crawl everything constantly"
                    .into(),
            });
        }
        if self.queue.index_stream_max_len < 1_000 {
            return Err(ConfigError::Value {
                key: "queue.index_stream_max_len".into(),
                msg: "must be at least 1000".into(),
            });
        }
        if !matches!(self.ml.device.as_str(), "auto" | "gpu" | "cpu") {
            return Err(ConfigError::Value {
                key: "ml.device".into(),
                msg: format!("{:?} is not one of auto, gpu, cpu", self.ml.device),
            });
        }
        if self.api.bind_addr.parse::<std::net::SocketAddr>().is_err() {
            return Err(ConfigError::Value {
                key: "api.bind_addr".into(),
                msg: format!("{:?} is not a socket address", self.api.bind_addr),
            });
        }
        if self.search.meili_url.parse::<url::Url>().is_err() {
            return Err(ConfigError::Value {
                key: "search.meili_url".into(),
                msg: format!("{:?} is not a url", self.search.meili_url),
            });
        }
        if self.search.max_hits_per_page == 0 || self.search.max_hits_per_page > 200 {
            return Err(ConfigError::Value {
                key: "search.max_hits_per_page".into(),
                msg: "must be between 1 and 200".into(),
            });
        }
        if self.search.default_hits_per_page > self.search.max_hits_per_page {
            return Err(ConfigError::Value {
                key: "search.default_hits_per_page".into(),
                msg: "must not exceed max_hits_per_page".into(),
            });
        }
        if self.api.max_concurrent == 0 {
            return Err(ConfigError::Value {
                key: "api.max_concurrent".into(),
                msg: "must be greater than zero".into(),
            });
        }
        if self.federation.searxng_usable()
            && self.federation.searxng_url.parse::<url::Url>().is_err()
        {
            return Err(ConfigError::Value {
                key: "federation.searxng_url".into(),
                msg: format!("{:?} is not a url", self.federation.searxng_url),
            });
        }
        if !self.federation.federator_url.trim().is_empty()
            && self.federation.federator_url.parse::<url::Url>().is_err()
        {
            return Err(ConfigError::Value {
                key: "federation.federator_url".into(),
                msg: format!("{:?} is not a url", self.federation.federator_url),
            });
        }
        if self.federation.enabled
            && (self.federation.budget_ms == 0 || self.federation.budget_ms > 5000)
        {
            return Err(ConfigError::Value {
                key: "federation.budget_ms".into(),
                msg: "must be between 1 and 5000 — the strip wait may never make the local answer wait long".into(),
            });
        }
        // The strip wait has to fit inside the search timeout with room to shape the page, or a
        // slow tool turns every search into a 504. Refused rather than clamped, like every other
        // guard here — and checked whenever a gateway is configured, not only when the file says
        // enabled, because the admin console can switch federation on at runtime.
        if self.federation.api_federation_usable()
            && self.federation.budget_ms + 200 > self.api.timeout_search_ms
        {
            return Err(ConfigError::Value {
                key: "federation.budget_ms".into(),
                msg: format!(
                    "must be at least 200ms below api.timeout_search_ms ({}), or the strip wait \
                     leaves no time to answer",
                    self.api.timeout_search_ms
                ),
            });
        }
        if self.federation.enabled
            && (self.federation.fetch_budget_ms == 0 || self.federation.fetch_budget_ms > 30_000)
        {
            return Err(ConfigError::Value {
                key: "federation.fetch_budget_ms".into(),
                msg: "must be between 1 and 30000".into(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod crawl_guard_tests {
    use super::*;

    #[test]
    fn the_bypass_is_off_by_default() {
        // The only safe default for a flag whose failure mode is being reported for abuse.
        let c = CrawlConfig::default();
        assert!(!c.ignore_politeness);
        assert!(c.respect_crawl_delay);
        assert_eq!(c.per_host_concurrency, 1);
    }

    #[test]
    fn federation_is_off_and_inert_by_default() {
        // Default off, and a flag with no endpoint stays inert rather than erroring — the same
        // "both required" rule Brave uses. Query-time egress is opt-in ([[ADR-0017]]).
        let f = FederationConfig::default();
        assert!(!f.enabled);
        assert!(!f.searxng_usable());
        assert_eq!(f.budget_ms, 1500);
        assert_eq!(f.fetch_budget_ms, 6000);
        // Enabled but endpointless is still inert, not an error.
        let f = FederationConfig {
            enabled: true,
            ..FederationConfig::default()
        };
        assert!(!f.searxng_usable());
    }

    #[test]
    fn federation_rejects_a_non_url_endpoint_when_usable() {
        let mut c = Config::default();
        c.federation.enabled = true;
        c.federation.searxng_url = "not a url".into();
        assert!(c.validate().is_err());
        c.federation.searxng_url = "http://xustive-searxng:8080".into();
        assert!(c.validate().is_ok());
    }

    #[test]
    fn non_dev_refuses_sub_20_k_anonymity_everywhere_it_matters() {
        // BUG-035: enforced in validate() so EVERY binary that loads the config gets it, not only
        // the API. Interaction k, the explicit hot-click floor, and discovery k are all floored.
        let mut c = Config::default();
        c.environment = "prod".into();
        c.interaction.enabled = true;
        c.interaction.salt = "deploy-secret".into(); // required outside dev; tested below
        c.interaction.k_anonymity = 1;
        assert!(c.validate().is_err(), "prod + interaction k=1 must refuse");

        c.interaction.k_anonymity = 20;
        c.interaction.hot_click_floor = 5;
        assert!(
            c.validate().is_err(),
            "prod + hot_click_floor=5 must refuse"
        );
        c.interaction.hot_click_floor = 0; // "use k" stays legal
        assert!(c.validate().is_ok());

        c.discovery.weak_coverage_enabled = true;
        c.discovery.k_anonymity = 1;
        assert!(c.validate().is_err(), "prod + discovery k=1 must refuse");
        c.discovery.k_anonymity = 20;
        assert!(c.validate().is_ok());

        // Outside dev an empty qhash salt is refused too (BUG-036) — unsalted FNV is
        // dictionary-reversible, the exact "false comfort" ADR-0008 rejects.
        c.interaction.salt = String::new();
        assert!(c.validate().is_err(), "prod + empty salt must refuse");
        c.interaction.salt = "deploy-secret".into();
        assert!(c.validate().is_ok());

        // Dev keeps the single-operator escape hatch: k=1 honestly means "no anonymity, my box".
        c.environment = "dev".into();
        c.interaction.k_anonymity = 1;
        c.discovery.k_anonymity = 1;
        assert!(c.validate().is_ok(), "dev may run k=1");
    }

    #[test]
    fn production_refuses_to_start_with_the_bypass_on() {
        // Not a warning. A warning in startup output is a warning nobody reads, and the symptom of
        // getting this wrong appears on someone else's server rather than in our logs.
        let c = CrawlConfig {
            ignore_politeness: true,
            ..CrawlConfig::default()
        };
        for env in ["prod", "production", "staging"] {
            assert!(c.guard(env).is_err(), "{env} should refuse");
        }
    }

    #[test]
    fn development_may_use_the_bypass() {
        // The whole point: crawling a local fixture site at full speed without a robots round trip
        // per request.
        let c = CrawlConfig {
            ignore_politeness: true,
            ..CrawlConfig::default()
        };
        for env in ["dev", "development", "ci", "test", ""] {
            assert!(c.guard(env).is_ok(), "{env} should allow");
        }
    }

    #[test]
    fn production_also_refuses_the_quieter_ways_to_be_rude() {
        // Turning off crawl-delay or raising per-host concurrency is the same abuse arrived at by
        // a less obvious route, and neither looks alarming in a diff.
        for c in [
            CrawlConfig {
                respect_crawl_delay: false,
                ..CrawlConfig::default()
            },
            CrawlConfig {
                per_host_concurrency: 8,
                ..CrawlConfig::default()
            },
        ] {
            assert!(c.guard("prod").is_err());
        }
    }

    #[test]
    fn the_shipped_production_config_is_safe() {
        // Reads the file that actually deploys. A guard the deployed config was never run through
        // proves only that the guard compiles.
        for env in ["prod", "staging"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../../config/{env}.toml"));
            let cfg = Config::load(Some(&path)).expect("config should load");
            assert_eq!(
                cfg.environment, env,
                "config/{env}.toml must declare its environment"
            );
            assert!(
                cfg.crawl.guard(&cfg.environment).is_ok(),
                "config/{env}.toml would crawl abusively: {:?}",
                cfg.crawl
            );
            assert!(
                cfg.interaction.guard(&cfg.environment).is_ok(),
                "config/{env}.toml has an unsafe interaction k-floor: {:?}",
                cfg.interaction
            );
        }
    }

    #[test]
    fn a_sub_floor_k_is_refused_outside_dev() {
        // The structural k-anonymity guarantee (M6-T01.4): enabled interaction with k < 20 must not
        // start anywhere but dev — otherwise the "k-anonymous" claim is false.
        let unsafe_cfg = InteractionConfig {
            enabled: true,
            k_anonymity: 1,
            ..InteractionConfig::default()
        };
        for env in ["prod", "production", "staging", "ci"] {
            assert!(unsafe_cfg.guard(env).is_err(), "{env} should refuse k=1");
        }
        assert!(
            unsafe_cfg.guard("dev").is_ok(),
            "dev may run single-user with k=1"
        );

        // Disabled is always fine — no counters exist to de-anonymise.
        let off = InteractionConfig {
            enabled: false,
            k_anonymity: 1,
            ..InteractionConfig::default()
        };
        assert!(off.guard("prod").is_ok());

        // At or above the floor is fine everywhere.
        let safe = InteractionConfig {
            enabled: true,
            k_anonymity: 20,
            ..InteractionConfig::default()
        };
        assert!(safe.guard("prod").is_ok());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn missing_file_falls_back_to_defaults() {
        let cfg = Config::load(Some(Path::new("/nonexistent/xustive.toml"))).unwrap();
        assert_eq!(cfg.api.bind_addr, ApiConfig::default().bind_addr);
    }

    #[test]
    fn partial_toml_overlays_defaults() {
        let dir = std::env::temp_dir().join("xustive-cfg-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("partial.toml");
        std::fs::write(&path, "[search]\ncandidate_pool = 42\n").unwrap();

        let cfg = Config::load(Some(&path)).unwrap();
        assert_eq!(cfg.search.candidate_pool, 42);
        // Untouched keys keep their defaults.
        assert_eq!(cfg.search.documents_index, "documents");
        assert_eq!(cfg.api.max_concurrent, 512);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn invalid_bind_addr_is_rejected() {
        let mut cfg = Config::default();
        cfg.api.bind_addr = "not-an-addr".into();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, ConfigError::Value { .. }));
        assert!(err.class().is_fatal());
    }

    #[test]
    fn inconsistent_page_sizes_are_rejected() {
        let mut cfg = Config::default();
        cfg.search.default_hits_per_page = 100;
        cfg.search.max_hits_per_page = 50;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn malformed_toml_reports_the_path() {
        let dir = std::env::temp_dir().join("xustive-cfg-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad.toml");
        std::fs::write(&path, "this is not = = toml").unwrap();
        let err = Config::load(Some(&path)).unwrap_err();
        assert!(matches!(err, ConfigError::Parse { .. }));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn config_round_trips_through_toml() {
        let cfg = Config::default();
        let text = toml::to_string(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(cfg, back);
    }
}
