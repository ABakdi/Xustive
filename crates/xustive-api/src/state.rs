//! Shared application state.

use std::sync::atomic::{AtomicI64, AtomicU8};
use std::sync::Arc;
use std::time::Duration;

use std::collections::HashMap;
use xustive_core::Config;
use xustive_lang::Detector;

use xustive_core::TrustTier;
use xustive_search::{MeiliClient, SearchError, Weights};

use crate::metrics::Metrics;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub search: Arc<MeiliClient>,
    /// Built once at startup: the lexicons are compiled in but the maps are not free to
    /// construct, and detection runs on every query.
    pub detector: Arc<Detector>,
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
    pub metrics: Metrics,
}

impl AppState {
    pub fn new(config: Config) -> Result<Self, SearchError> {
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
            ranking: Arc::new(Weights::default()),
            trust_tiers: Arc::new(load_trust_tiers()),
            device_preference: Arc::new(AtomicU8::new(
                xustive_ml::DevicePreference::parse(&device).unwrap_or_default() as u8,
            )),
            gpu_layers: Arc::new(AtomicI64::new(gpu_layers)),
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
