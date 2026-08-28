//! Shared application state.

use std::sync::atomic::{AtomicI64, AtomicU8};
use std::sync::Arc;
use std::time::Duration;

use std::collections::HashMap;
use xustive_core::Config;
use xustive_lang::{Detector, Expander, ExpanderConfig};

use xustive_core::TrustTier;
use xustive_search::{MeiliClient, SearchError, Weights};

use crate::metrics::Metrics;
use crate::ratelimit::RateLimiter;
use crate::summary::PendingStore;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub search: Arc<MeiliClient>,
    /// Built once at startup: the lexicons are compiled in but the maps are not free to
    /// construct, and detection runs on every query.
    pub detector: Arc<Detector>,
    /// Query expansion: Arabizi to Arabic, Darija to MSA, synonyms.
    ///
    /// Built once — the lexicons are compiled in but the maps are not free to construct, and
    /// expansion runs on every query that needs a second retrieval leg.
    pub expander: Arc<Expander>,
    /// Ranking weights. Loaded once at startup from `config/ranking.toml` if present, else defaults.
    pub ranking: Arc<Weights>,
    /// Source id to trust tier, from the seed registry.
    pub trust_tiers: Arc<HashMap<String, TrustTier>>,
    /// Domain to authority (0–1), from `data/sources/authority.tsv` — the "famous websites" signal.
    pub authority: Arc<HashMap<String, f32>>,
    /// Device preference, encoded as `DevicePreference as u8`.
    ///
    /// An atomic rather than a lock: it is read on every model load and written rarely from the
    /// admin page, and the read path should not be able to block on a writer.
    pub device_preference: Arc<AtomicU8>,
    /// GPU layers to offload. Negative means decide automatically.
    pub gpu_layers: Arc<AtomicI64>,
    /// Runtime politeness bypass. **Testing only** — see `CrawlConfig::ignore_politeness`.
    ///
    /// Runtime as well as config, because the reason to turn it on is "I am about to crawl a
    /// fixture site" and the reason to turn it off is "I have finished", neither of which is worth
    /// a restart. The config value is the startup default; production refuses to start with it on
    /// at all, so this can only ever be flipped where it is already permitted.
    pub ignore_politeness: Arc<std::sync::atomic::AtomicBool>,
    /// Runtime federation switch, mirrored from `[federation] enabled` ([[ADR-0017]], M7-T04). Flipped
    /// from the admin Integrations page. The consumers — the query-time blend and the crawler
    /// crawl-feed, later increments — read it. The serving plane only *shows and toggles* it; it
    /// never reaches SearXNG itself, which lives behind the [[Federation Gateway]] on the egress
    /// network. Runtime as well as config so turning federation on or off is not a restart.
    pub federation_enabled: Arc<std::sync::atomic::AtomicBool>,
    /// Runtime switch for the external summariser (M7-T08), mirrored from `[ml] external_summaries`
    /// and flipped from the admin Integrations page. When on (and a gateway client exists), the
    /// summary endpoint tries the external LLM through the [[Federation Gateway]] before the local
    /// model; when off, summaries are exactly the local-only path. Default off — third-party SaaS.
    pub external_summaries: Arc<std::sync::atomic::AtomicBool>,
    /// Searches whose summary has not been requested yet.
    pub pending: Arc<PendingStore>,
    /// The loaded summariser, once it is ready.
    ///
    /// Behind a lock and an `Option` because loading takes seconds and must not delay the first
    /// search. Until it resolves, summaries are simply unavailable — which is a state the
    /// endpoint already handles, since it is also what a busy or missing model looks like.
    #[cfg(feature = "summariser")]
    pub engine: Arc<std::sync::RwLock<Option<Arc<xustive_ml::engine::Engine>>>>,
    /// The concrete index behind the `documents` alias, resolved once at startup.
    ///
    /// Resolved once rather than per request: it changes only when a reindex flips the alias,
    /// which restarts the process anyway, and a lookup on the search path would add a round
    /// trip to every query to answer a question whose answer never changes.
    pub documents_index: Arc<std::sync::RwLock<String>>,
    /// Per-route request budgets, keyed on a salted network hash rather than an address.
    pub limiter: Arc<RateLimiter>,
    /// Prefix index for autocomplete. Built once at startup from the curated list; corpus
    /// terms are added by [`Self::refresh_suggestions`] once the index is reachable.
    pub suggest: Arc<std::sync::RwLock<Arc<crate::suggest::PrefixIndex>>>,
    /// Cache the tool data plane fills. `None` when Redis is unreachable, which is not fatal:
    /// tools that need it render nothing and everything else is unaffected.
    pub tool_cache: Option<xustive_toold::store::Store>,
    /// The bundled IP-to-city database ([[ADR-0020]]). `None` when the file is not provisioned,
    /// which is the normal state of a fresh checkout: weather then answers only for a named place.
    /// Loaded once — it is memory-mapped, and reopening it per request would be the only expensive
    /// part of an otherwise microsecond lookup.
    pub geoip: Option<Arc<crate::geoip::GeoIp>>,
    /// The OCR engine the user-facing tools use, built once from `[media]`. Tesseract by default;
    /// a [`xustive_media::Fallback`] over the Unlimited-OCR sidecar when `ocr_backend = "unlimited"`,
    /// so a down sidecar degrades to tesseract instead of failing. See [`build_ocr_backend`].
    pub ocr: Arc<dyn xustive_media::OcrBackend>,
    /// Image-similarity search, or `None` when `[vector] enabled = false`. Its absence is a normal
    /// state — the `/search/image` endpoint returns a clean unavailable and text search is untouched.
    pub image_search: Option<crate::image_search::ImageSearch>,
    /// Semantic (dense) text retrieval for search (M7-T02). `Some` only when `[vector] text_enabled`.
    /// Every use is fail-open, so its absence or an outage only removes semantic recall, never
    /// results.
    pub text_search: Option<crate::text_search::TextSearch>,
    /// Voice transcription, or `None` when `[stt] enabled = false`. Same posture: absence is normal
    /// and the `/transcribe` endpoint returns a clean unavailable.
    pub stt: Option<crate::stt::SttClient>,
    /// Client for the [[Federation Gateway]] (M7-T05). `Some` only when the API is configured to
    /// federate; even then, gated at request time by the `federation_enabled` runtime switch. Every
    /// call is fail-open, so this being absent, disabled, or broken only removes the "from the web"
    /// strip — never the results.
    pub federator: Option<crate::federate::FederatorClient>,
    /// Anonymous interaction signals (M6): a k-anonymous CTR store in Redis. `None` when
    /// `[interaction] enabled = false` or Redis is unreachable — the search path treats absence as
    /// "no interaction data", so ranking and capture simply skip it. Never holds a per-person record.
    ///
    /// Behind a lock because the store connects async (`connect_in`), after the sync `new`, the same
    /// way the summariser engine is filled in — see [`AppState::connect_interactions`].
    pub interactions: Arc<std::sync::RwLock<Option<xustive_ingest::interaction::Interactions>>>,
    /// Opaque search→click tokens (M6-T03): `token → (qhash, minted_at)`, in memory, swept on write.
    /// The query text never leaves the process; the click endpoint resolves a token to a qhash here.
    pub interaction_tokens: Arc<std::sync::RwLock<HashMap<String, (String, std::time::Instant)>>>,
    /// What the token's search showed, kept only while first-party collection is on (M11): the
    /// query as typed and the ids in rank order, so a click or a report can be written with
    /// the query and the rank without the browser ever sending either.
    pub token_context: Arc<std::sync::RwLock<HashMap<String, (String, Vec<String>)>>>,
    /// The first-party events sink ([[ADR-0030]]); `None` when `[collection] enabled = false`.
    pub events: Option<crate::events::EventSink>,
    /// The console's short memory of the system's vitals (M12), started after the state exists.
    pub timeseries: Option<crate::timeseries::Ring>,
    /// Live crawl counters, connected **once** with a shared auto-reconnecting manager. Reused by
    /// every `/crawler/events` SSE frame and metrics sample rather than reconnecting per call — the
    /// per-frame reconnect was what surfaced a transient blip as "Redis unreachable" on the Live
    /// page. `None` until the async connect runs (or if Redis is down at startup).
    pub crawl_stats: Arc<std::sync::RwLock<Option<xustive_ingest::crawl_stats::CrawlStats>>>,
    pub metrics: Metrics,
}

impl AppState {
    /// The current suggestion index.
    ///
    /// Cloned out under a brief read lock rather than held: a rebuild swaps the whole thing, and
    /// a request that grabbed the old one finishes against a consistent snapshot instead of
    /// blocking or seeing a half-built index.
    pub fn suggestions(&self) -> Arc<crate::suggest::PrefixIndex> {
        self.suggest
            .read()
            .map(|s| Arc::clone(&s))
            .unwrap_or_else(|p| Arc::clone(&p.into_inner()))
    }

    /// Rebuild the suggestion index from the corpus and swap it in.
    ///
    /// Called at startup and could be called on a timer. The swap is atomic from a reader's
    /// point of view, so this never makes suggestions unavailable — the worst case is that they
    /// stay slightly stale, which nobody notices.
    pub async fn refresh_suggestions(&self) {
        let curated = crate::suggest::load_curated(&self.config.suggest.curated_path);
        let corpus = self.corpus_terms().await;
        let built = crate::suggest::PrefixIndex::build(&curated, &corpus);
        tracing::info!(
            terms = built.len(),
            curated = curated.len(),
            "suggestion index built"
        );
        if let Ok(mut slot) = self.suggest.write() {
            *slot = Arc::new(built);
        }
    }

    /// Titles from the index, as suggestion candidates.
    ///
    /// A failure here is not an error: the curated list still works, and an autocomplete that
    /// offers slightly less is better than a startup that fails over an optional feature.
    async fn corpus_terms(&self) -> Vec<String> {
        use serde_json::Value;
        let index = self.documents_index();
        let query = xustive_search::Query::new("").limit(5_000);
        match self.search.search::<Value>(&index, &query).await {
            Ok(hits) => hits
                .hits
                .iter()
                .filter_map(|h| h.get("title")?.as_str())
                .map(crate::suggest::title_term)
                .filter(|t| !t.is_empty())
                .collect(),
            Err(e) => {
                tracing::warn!(error = %e, "could not read corpus terms for suggestions");
                Vec::new()
            }
        }
    }

    /// The index searches actually run against.
    /// The query and the rank of `doc` for a token's search, when collection kept them.
    pub fn token_context(&self, token: &str, doc: &str) -> Option<(String, Option<u32>)> {
        let map = self.token_context.read().ok()?;
        let (query, shown) = map.get(token)?;
        let rank = shown.iter().position(|d| d == doc).map(|i| i as u32 + 1);
        Some((query.clone(), rank))
    }

    pub fn documents_index(&self) -> String {
        self.documents_index
            .read()
            .map(|s| s.clone())
            .unwrap_or_else(|_| self.config.search.documents_index.clone())
    }

    /// Resolve the `documents` alias to a concrete index.
    ///
    /// Called once at startup. A failure here is not fatal: the alias name is a valid index name
    /// and is what pre-alias deployments use, so falling back keeps a system with an unreachable
    /// Meilisearch starting up and reporting the real problem through `/readyz`.
    pub async fn resolve_index(&self) {
        match self
            .search
            .resolve(&self.config.search.documents_index)
            .await
        {
            Ok(index) => {
                if index != self.config.search.documents_index {
                    tracing::info!(alias = %self.config.search.documents_index, %index, "alias resolved");
                }
                if let Ok(mut slot) = self.documents_index.write() {
                    *slot = index;
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "could not resolve the documents alias; using it directly")
            }
        }
    }

    /// Connect the interaction store, once, at startup — if `[interaction] enabled`. Async (the
    /// store opens a Redis connection manager), so it runs after the sync `new`, like the model
    /// load. Failure is non-fatal: search runs without interaction signals.
    pub async fn connect_interactions(&self) {
        let cfg = &self.config.interaction;
        if !cfg.enabled {
            return;
        }
        let store = xustive_ingest::interaction::Interactions::connect_in(
            self.config.queue.signals_url(),
            "interaction",
            cfg.k_anonymity,
            Duration::from_secs(cfg.window_days * 86_400),
            &cfg.salt,
        )
        .await;
        match store {
            Some(s) => {
                if let Ok(mut slot) = self.interactions.write() {
                    *slot = Some(s);
                }
                tracing::info!(k = cfg.k_anonymity, "interaction signals enabled");
            }
            None => tracing::warn!("interaction enabled but Redis is unreachable; running without"),
        }
    }

    /// A cheap clone of the interaction store, if connected. `None` disables the whole path.
    pub fn interactions(&self) -> Option<xustive_ingest::interaction::Interactions> {
        self.interactions.read().ok().and_then(|s| s.clone())
    }

    /// Connect the shared crawl-stats manager once, at startup. Reused by the Live SSE stream and
    /// the metrics sampler so neither reconnects per call. Non-fatal: a `None` here just means the
    /// Live page shows "unreachable" until Redis is back and the next connect attempt succeeds.
    pub async fn connect_crawl_stats(&self) {
        match xustive_ingest::crawl_stats::CrawlStats::connect(&self.config.queue.url).await {
            Some(s) => {
                if let Ok(mut slot) = self.crawl_stats.write() {
                    *slot = Some(s);
                }
            }
            None => {
                tracing::warn!("crawl stats: Redis unreachable at startup; Live page will retry")
            }
        }
    }

    /// A cheap clone of the shared crawl-stats store, if connected.
    pub fn crawl_stats(&self) -> Option<xustive_ingest::crawl_stats::CrawlStats> {
        self.crawl_stats.read().ok().and_then(|s| s.clone())
    }

    /// The summariser, if it has finished loading.
    #[cfg(feature = "summariser")]
    pub fn summariser(&self) -> Option<Arc<xustive_ml::engine::Engine>> {
        self.engine.read().ok().and_then(|e| e.clone())
    }

    /// Resolve the current device setting against the hardware present.
    pub fn device_config(&self) -> xustive_ml::DeviceConfig {
        use std::sync::atomic::Ordering;
        let preference = match self.device_preference.load(Ordering::Relaxed) {
            1 => xustive_ml::DevicePreference::Gpu,
            2 => xustive_ml::DevicePreference::Cpu,
            _ => xustive_ml::DevicePreference::Auto,
        };
        let layers = self.gpu_layers.load(Ordering::Relaxed);
        xustive_ml::DeviceConfig {
            preference,
            gpu_layers: if layers < 0 {
                None
            } else {
                Some(layers as u32)
            },
            ..Default::default()
        }
    }

    pub fn new(config: Config) -> Result<Self, SearchError> {
        let ignore_politeness = config.crawl.ignore_politeness;
        let federation_enabled = config.federation.enabled;
        let external_summaries = config.ml.external_summaries;
        // Falls back to the alias name itself, which is also what a pre-alias deployment uses.
        // `resolve` is async and this is not, so the real lookup happens in `resolve_index`
        // below, called from main once the runtime exists.
        let documents_index = config.search.documents_index.clone();
        let queue_url = config.queue.url.clone();
        let curated = crate::suggest::load_curated(&config.suggest.curated_path);
        let device = config.ml.device.clone();
        let gpu_layers = config.ml.gpu_layers;
        // Cloned out before `config` is moved into the Arc — the OCR and image-search engines are
        // built from these.
        let media = config.media.clone();
        let vector = config.vector.clone();
        let stt = config.stt.clone();
        let federation = config.federation.clone();
        let search = Arc::new(MeiliClient::new(
            &config.search.meili_url,
            &config.search.meili_key,
            Duration::from_millis(config.search.timeout_ms),
        )?);
        // First-party events ([[ADR-0030]]): the writer task starts with the state when on.
        let events = config
            .collection
            .enabled
            .then(|| crate::events::EventSink::start(search.clone()));
        Ok(Self {
            config: Arc::new(config),
            search: search.clone(),
            detector: Arc::new(Detector::default()),
            expander: Arc::new(Expander::new(ExpanderConfig::default())),
            ranking: Arc::new(load_ranking_weights()),
            trust_tiers: Arc::new(load_trust_tiers()),
            authority: Arc::new(load_authority(&queue_url)),
            device_preference: Arc::new(AtomicU8::new(
                xustive_ml::DevicePreference::parse(&device).unwrap_or_default() as u8,
            )),
            gpu_layers: Arc::new(AtomicI64::new(gpu_layers)),
            ignore_politeness: Arc::new(std::sync::atomic::AtomicBool::new(ignore_politeness)),
            federation_enabled: Arc::new(std::sync::atomic::AtomicBool::new(federation_enabled)),
            external_summaries: Arc::new(std::sync::atomic::AtomicBool::new(external_summaries)),
            documents_index: Arc::new(std::sync::RwLock::new(documents_index)),
            suggest: Arc::new(std::sync::RwLock::new(Arc::new(
                crate::suggest::PrefixIndex::build(&curated, &[]),
            ))),
            // Connecting lazily and tolerating failure. The serving plane must start whether or
            // not the fetcher has ever run — a cold system has no cached weather by definition.
            tool_cache: xustive_toold::store::Store::connect(&queue_url).ok(),
            geoip: crate::geoip::GeoIp::load(crate::geoip::DEFAULT_PATH).map(Arc::new),
            ocr: build_ocr_backend(&media),
            image_search: crate::image_search::ImageSearch::from_config(&vector),
            text_search: crate::text_search::TextSearch::from_config(&vector),
            stt: crate::stt::SttClient::from_config(&stt),
            federator: crate::federate::FederatorClient::from_config(&federation),
            interactions: Arc::new(std::sync::RwLock::new(None)),
            interaction_tokens: Arc::new(std::sync::RwLock::new(HashMap::new())),
            token_context: Arc::new(std::sync::RwLock::new(HashMap::new())),
            events,
            timeseries: None,
            crawl_stats: Arc::new(std::sync::RwLock::new(None)),
            limiter: Arc::new(RateLimiter::new()),
            pending: Arc::new(PendingStore::default()),
            #[cfg(feature = "summariser")]
            engine: Arc::new(std::sync::RwLock::new(None)),
            metrics: Metrics::new(),
        })
    }
}

/// Build the user-facing OCR backend from `[media]`.
///
/// `"tesseract"` (the default, and the fallback for anything unrecognised) is the in-process CPU
/// engine. `"unlimited"` wraps the sidecar in a [`xustive_media::Fallback`] over tesseract, so the
/// GPU service is *preferred* but never *required*: if it is down or slow, the request quietly
/// degrades to tesseract rather than failing. Building the sidecar client can only fail on a
/// malformed endpoint, and there too we fall back to tesseract rather than refuse to start.
fn build_ocr_backend(
    media: &xustive_core::config::MediaConfig,
) -> Arc<dyn xustive_media::OcrBackend> {
    let tesseract = xustive_media::Tesseract::new(&media.tessdata_dir, &media.ocr_langs);
    if media.ocr_backend != "unlimited" {
        return Arc::new(tesseract);
    }
    match xustive_media::Sidecar::new(
        &media.sidecar.endpoint,
        Duration::from_millis(media.sidecar.timeout_ms),
    ) {
        Ok(sidecar) => Arc::new(xustive_media::Fallback::new(
            Box::new(sidecar),
            Box::new(tesseract),
        )),
        Err(e) => {
            tracing::warn!(error = %e, "Unlimited-OCR sidecar unavailable; using tesseract");
            Arc::new(tesseract)
        }
    }
}

/// Load ranking weights from `config/ranking.toml`, falling back to the built-in defaults.
///
/// Read at runtime (not compiled in) precisely so the offline yardstick can write tuned weights
/// there and a restart picks them up — the tuning loop is edit-file-then-restart, not recompile. A
/// missing or malformed file is not an error: it just means "use the defaults", logged so a typo in
/// the file does not silently revert a deliberate tune.
fn load_ranking_weights() -> Weights {
    match std::fs::read_to_string("config/ranking.toml") {
        Ok(body) => match toml::from_str::<Weights>(&body) {
            Ok(w) => {
                tracing::info!("loaded ranking weights from config/ranking.toml");
                w
            }
            Err(e) => {
                tracing::warn!(error = %e, "config/ranking.toml is malformed; using default weights");
                Weights::default()
            }
        },
        Err(_) => Weights::default(),
    }
}

/// Build the domain→authority map: the curated prior, with computed PageRank filling in every domain
/// the prior does not name.
///
/// The curated list wins on conflict (`or_insert`) — a human vouching for a domain outranks the link
/// graph — and PageRank's earned scores lift crawled-but-unlisted domains above the flat baseline.
/// The PageRank scores already carry the `.dz` home floor, so merging them preserves Algeria-first.
/// A missing or empty `pagerank:authority` (PageRank never run, or no Redis) just leaves the prior.
fn load_authority(queue_url: &str) -> HashMap<String, f32> {
    let mut map = xustive_search::authority::load();
    if let Some(store) = xustive_ingest::link_graph::LinkGraphStore::connect(queue_url) {
        let computed = store.load_authority_blocking();
        let earned = computed.len();
        for (domain, score) in computed {
            map.entry(domain).or_insert(score);
        }
        if earned > 0 {
            tracing::info!(earned, "merged PageRank authority into the curated prior");
        }
    }
    map
}

/// Read source trust tiers from the seed registry.
///
/// Compiled in rather than read at runtime: the registry is small, and a missing file would
/// silently flatten every source to the default tier — a ranking change nobody would notice.
fn load_trust_tiers() -> HashMap<String, TrustTier> {
    const SEEDS: &str = include_str!("../../../data/sources/seeds.tsv");
    let mut out = HashMap::new();
    for line in SEEDS.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').map(str::trim).collect();
        if cols.len() < 3 {
            continue;
        }
        let tier = match cols[2].to_ascii_uppercase().as_str() {
            "A" => TrustTier::A,
            "C" => TrustTier::C,
            _ => TrustTier::B,
        };
        out.insert(cols[0].to_string(), tier);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_tiers_load_from_the_seed_registry() {
        let t = load_trust_tiers();
        assert!(t.len() >= 15, "only {} sources loaded", t.len());
        assert_eq!(t.get("aps-dz"), Some(&TrustTier::A));
    }

    #[test]
    fn ocr_backend_defaults_to_tesseract() {
        let media = xustive_core::config::MediaConfig::default();
        assert_eq!(build_ocr_backend(&media).name(), "tesseract");
    }

    #[test]
    fn ocr_backend_unlimited_prefers_the_sidecar() {
        // The reported name is the sidecar's even though a tesseract fallback is wrapped behind it —
        // selecting "unlimited" puts the sidecar in the path, and the fallback stays invisible until
        // it is actually needed.
        let media = xustive_core::config::MediaConfig {
            ocr_backend: "unlimited".into(),
            ..Default::default()
        };
        assert_eq!(build_ocr_backend(&media).name(), "unlimited");
    }

    #[test]
    fn ocr_backend_falls_back_to_tesseract_for_unknown_value() {
        let media = xustive_core::config::MediaConfig {
            ocr_backend: "nonsense".into(),
            ..Default::default()
        };
        assert_eq!(build_ocr_backend(&media).name(), "tesseract");
    }
}
