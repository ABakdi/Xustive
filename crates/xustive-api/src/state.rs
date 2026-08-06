//! Shared application state.

use std::sync::Arc;
use std::time::Duration;

use xustive_core::Config;
use xustive_search::{MeiliClient, SearchError};

use crate::metrics::Metrics;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub search: Arc<MeiliClient>,
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
            metrics: Metrics::new(),
        })
    }
}
