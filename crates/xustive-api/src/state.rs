//! Shared application state.

use std::sync::Arc;
use std::time::Duration;

use xustive_core::Config;
use xustive_lang::Detector;
use xustive_search::{MeiliClient, SearchError};

use crate::metrics::Metrics;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub search: Arc<MeiliClient>,
    /// Built once at startup: the lexicons are compiled in but the maps are not free to
    /// construct, and detection runs on every query.
    pub detector: Arc<Detector>,
    pub metrics: Metrics,
}

impl AppState {
    pub fn new(config: Config) -> Result<Self, SearchError> {
        let search = MeiliClient::new(
            &config.search.meili_url,
            &config.search.meili_key,
            Duration::from_millis(config.search.timeout_ms),
        )?;
        Ok(Self {
            config: Arc::new(config),
            search: Arc::new(search),
            detector: Arc::new(Detector::default()),
            metrics: Metrics::new(),
        })
    }
}
