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
        "{field} is set to an unsafe value for the {environment} environment; \
         this configuration would crawl abusively"
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
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            url: "redis://127.0.0.1:6390".into(),
            index_stream: "q:index".into(),
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
    /// Rows per page in the admin document list.
    ///
    /// Paged rather than "all": a list that loads everything is fine at a thousand documents and
    /// unusable at a million, and that failure arrives exactly when the crawler starts working.
    pub documents_page_size: usize,
}

impl Default for CrawlConfig {
    fn default() -> Self {
        Self {
            respect_crawl_delay: true,
            per_host_concurrency: 1,
            // Off. The only safe default for a flag whose failure mode is being reported for abuse.
            ignore_politeness: false,
            documents_page_size: 50,
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
        }
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
