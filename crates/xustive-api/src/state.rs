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
    /// Ranking weights. Loaded once; hot-reload arrives with the config work.
    pub ranking: Arc<Weights>,
    /// Source id to trust tier, from the seed registry.
    pub trust_tiers: Arc<HashMap<String, TrustTier>>,
    /// Device preference, encoded as `DevicePreference as u8`.
    ///
    /// An atomic rather than a lock: it is read on every model load and written rarely from the
    /// admin page, and the read path should not be able to block on a writer.
    pub device_preference: Arc<AtomicU8>,
    /// GPU layers to offload. Negative means decide automatically.
    pub gpu_layers: Arc<AtomicI64>,
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
        // Falls back to the alias name itself, which is also what a pre-alias deployment uses.
        // `resolve` is async and this is not, so the real lookup happens in `resolve_index`
        // below, called from main once the runtime exists.
        let documents_index = config.search.documents_index.clone();
        let queue_url = config.queue.url.clone();
        let curated = crate::suggest::load_curated(&config.suggest.curated_path);
        let device = config.ml.device.clone();
        let gpu_layers = config.ml.gpu_layers;
        let search = MeiliClient::new(
            &config.search.meili_url,
            &config.search.meili_key,
            Duration::from_millis(config.search.timeout_ms),
        )?;
        Ok(Self {
            config: Arc::new(config),
            search: Arc::new(search),
            detector: Arc::new(Detector::default()),
            expander: Arc::new(Expander::new(ExpanderConfig::default())),
            ranking: Arc::new(Weights::default()),
            trust_tiers: Arc::new(load_trust_tiers()),
            device_preference: Arc::new(AtomicU8::new(
                xustive_ml::DevicePreference::parse(&device).unwrap_or_default() as u8,
            )),
            gpu_layers: Arc::new(AtomicI64::new(gpu_layers)),
            documents_index: Arc::new(std::sync::RwLock::new(documents_index)),
            suggest: Arc::new(std::sync::RwLock::new(Arc::new(
                crate::suggest::PrefixIndex::build(&curated, &[]),
            ))),
            // Connecting lazily and tolerating failure. The serving plane must start whether or
            // not the fetcher has ever run — a cold system has no cached weather by definition.
            tool_cache: xustive_toold::store::Store::connect(&queue_url).ok(),
            limiter: Arc::new(RateLimiter::new()),
            pending: Arc::new(PendingStore::default()),
            #[cfg(feature = "summariser")]
            engine: Arc::new(std::sync::RwLock::new(None)),
            metrics: Metrics::new(),
        })
    }
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
}
