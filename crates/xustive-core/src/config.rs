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
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub api: ApiConfig,
    pub search: SearchConfig,
    pub telemetry: TelemetryConfig,
    pub ml: MlConfig,
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
    }

    fn validate(&self) -> Result<(), ConfigError> {
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
        Ok(())
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
