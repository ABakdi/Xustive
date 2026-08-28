//! What the console may change while the process runs, and keep (M12-T02).
//!
//! The config is an `Arc<Config>` read once; that is the right shape for almost everything — a
//! port, a path, a model — and the wrong shape for the handful of numbers an operator tunes by
//! looking at a chart: the ranking weights, the federation budgets, and the switches for
//! collection, interaction and summaries. Those live here, behind atomics and one `RwLock`,
//! read per request; a `PATCH /admin/settings` validates a change with the same rules a restart
//! would apply, applies it at once, logs the old and new values with the operator's address, and
//! writes `runtime.toml` beside the config so the next start agrees with the console.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use xustive_core::config::{Config, RuntimeOverrides};
use xustive_search::rank::Weights;

use crate::state::AppState;

pub struct RuntimeSettings {
    ranking: RwLock<Arc<Weights>>,
    pub federation_budget_ms: AtomicU64,
    pub fetch_budget_ms: AtomicU64,
    pub max_hits: AtomicUsize,
    pub eager_index: AtomicBool,
    pub collection_enabled: AtomicBool,
    pub summaries_enabled: AtomicBool,
    pub interaction_enabled: AtomicBool,
    /// Which fields came from `runtime.toml` rather than the config, for the console to say so.
    overridden: RwLock<Vec<&'static str>>,
}

impl RuntimeSettings {
    pub fn from_config(config: &Config, ranking: Weights) -> Self {
        let o = RuntimeOverrides::load(config.config_path.as_deref()).unwrap_or_default();
        let mut overridden = Vec::new();
        for (name, set) in [
            ("federation.budget_ms", o.federation.budget_ms.is_some()),
            (
                "federation.fetch_budget_ms",
                o.federation.fetch_budget_ms.is_some(),
            ),
            ("federation.max_hits", o.federation.max_hits.is_some()),
            ("federation.eager_index", o.federation.eager_index.is_some()),
            ("collection.enabled", o.collection.enabled.is_some()),
            ("ml.summaries_enabled", o.ml.summaries_enabled.is_some()),
            ("interaction.enabled", o.interaction.enabled.is_some()),
            ("ranking", o.ranking.is_some()),
        ] {
            if set {
                overridden.push(name);
            }
        }
        // The ranking override wins over ranking.toml, if it passes the rule.
        let ranking = match o.ranking.and_then(|v| v.try_into::<Weights>().ok()) {
            Some(w) if w.check().is_ok() => w,
            _ => ranking,
        };
        Self {
            ranking: RwLock::new(Arc::new(ranking)),
            federation_budget_ms: AtomicU64::new(config.federation.budget_ms),
            fetch_budget_ms: AtomicU64::new(config.federation.fetch_budget_ms),
            max_hits: AtomicUsize::new(config.federation.max_hits),
            eager_index: AtomicBool::new(config.federation.eager_index),
            collection_enabled: AtomicBool::new(config.collection.enabled),
            summaries_enabled: AtomicBool::new(config.ml.summaries_enabled),
            interaction_enabled: AtomicBool::new(config.interaction.enabled),
            overridden: RwLock::new(overridden),
        }
    }

    pub fn ranking(&self) -> Arc<Weights> {
        self.ranking.read().map(|w| w.clone()).unwrap_or_default()
    }
    pub fn collection_enabled(&self) -> bool {
        self.collection_enabled.load(Ordering::Relaxed)
    }
    pub fn summaries_enabled(&self) -> bool {
        self.summaries_enabled.load(Ordering::Relaxed)
    }

    /// The effective settings, with what is overridden — the console's `GET`.
    pub fn snapshot(&self) -> Value {
        json!({
            "ranking": &*self.ranking(),
            "federation": {
                "budget_ms": self.federation_budget_ms.load(Ordering::Relaxed),
                "fetch_budget_ms": self.fetch_budget_ms.load(Ordering::Relaxed),
                "max_hits": self.max_hits.load(Ordering::Relaxed),
                "eager_index": self.eager_index.load(Ordering::Relaxed),
            },
            "collection": { "enabled": self.collection_enabled() },
            "ml": { "summaries_enabled": self.summaries_enabled() },
            "interaction": { "enabled": self.interaction_enabled.load(Ordering::Relaxed) },
            "overridden": self.overridden.read().map(|v| v.clone()).unwrap_or_default(),
        })
    }
}

/// A partial change: only the fields present are touched.
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Patch {
    pub ranking: Option<Weights>,
    pub federation: Option<FederationPatch>,
    pub collection: Option<SwitchPatch>,
    pub ml: Option<MlPatch>,
    pub interaction: Option<SwitchPatch>,
}
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct FederationPatch {
    pub budget_ms: Option<u64>,
    pub fetch_budget_ms: Option<u64>,
    pub max_hits: Option<usize>,
    pub eager_index: Option<bool>,
}
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct SwitchPatch {
    pub enabled: Option<bool>,
}
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct MlPatch {
    pub summaries_enabled: Option<bool>,
}

/// The bounds a change must respect — the same ones a config file would be held to.
fn check(p: &Patch) -> Result<(), String> {
    if let Some(w) = &p.ranking {
        w.check()?;
    }
    if let Some(f) = &p.federation {
        if let Some(b) = f.budget_ms {
            if !(100..=5_000).contains(&b) {
                return Err("federation.budget_ms must be between 100 and 5000".into());
            }
        }
        if let Some(b) = f.fetch_budget_ms {
            if !(1_000..=30_000).contains(&b) {
                return Err("federation.fetch_budget_ms must be between 1000 and 30000".into());
            }
        }
        if let Some(m) = f.max_hits {
            if !(1..=50).contains(&m) {
                return Err("federation.max_hits must be between 1 and 50".into());
            }
        }
    }
    Ok(())
}

/// `GET /api/v1/admin/settings`.
pub async fn get(
    State(state): State<AppState>,
    crate::admin::Peer(peer): crate::admin::Peer,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;
    Ok(Json(state.runtime.snapshot()))
}

/// `PATCH /api/v1/admin/settings` — validate, apply, persist, answer with the effective values.
pub async fn patch(
    State(state): State<AppState>,
    crate::admin::Peer(peer): crate::admin::Peer,
    headers: HeaderMap,
    Json(p): Json<Patch>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;
    if let Err(e) = check(&p) {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": e })),
        ));
    }
    let before = state.runtime.snapshot();
    let r = &state.runtime;
    let mut changed: Vec<&'static str> = Vec::new();

    if let Some(w) = p.ranking {
        if let Ok(mut slot) = r.ranking.write() {
            *slot = Arc::new(w);
        }
        changed.push("ranking");
    }
    if let Some(f) = &p.federation {
        if let Some(v) = f.budget_ms {
            r.federation_budget_ms.store(v, Ordering::Relaxed);
            changed.push("federation.budget_ms");
        }
        if let Some(v) = f.fetch_budget_ms {
            r.fetch_budget_ms.store(v, Ordering::Relaxed);
            changed.push("federation.fetch_budget_ms");
        }
        if let Some(v) = f.max_hits {
            r.max_hits.store(v, Ordering::Relaxed);
            changed.push("federation.max_hits");
        }
        if let Some(v) = f.eager_index {
            r.eager_index.store(v, Ordering::Relaxed);
            changed.push("federation.eager_index");
        }
    }
    if let Some(Some(v)) = p.collection.as_ref().map(|c| c.enabled) {
        r.collection_enabled.store(v, Ordering::Relaxed);
        changed.push("collection.enabled");
    }
    if let Some(Some(v)) = p.ml.as_ref().map(|m| m.summaries_enabled) {
        r.summaries_enabled.store(v, Ordering::Relaxed);
        changed.push("ml.summaries_enabled");
    }
    if let Some(Some(v)) = p.interaction.as_ref().map(|i| i.enabled) {
        r.interaction_enabled.store(v, Ordering::Relaxed);
        // The anonymous store connects or lets go at once; a failure to connect is reported
        // by the status, not here.
        if v {
            state.connect_interactions_forced().await;
        } else if let Ok(mut slot) = state.interactions.write() {
            *slot = None;
        }
        changed.push("interaction.enabled");
    }
    if changed.is_empty() {
        return Ok(Json(r.snapshot()));
    }

    // Persist: read the file's current overrides, merge, write. The file is the console's, so a
    // whole-file rewrite is right; the config file itself is never touched.
    let path_cfg = state.config.config_path.as_deref();
    let mut o = RuntimeOverrides::load(path_cfg).unwrap_or_default();
    if changed.contains(&"ranking") {
        o.ranking = toml::Value::try_from(&*r.ranking()).ok();
    }
    if changed.iter().any(|c| c.starts_with("federation.")) {
        o.federation.budget_ms = Some(r.federation_budget_ms.load(Ordering::Relaxed));
        o.federation.fetch_budget_ms = Some(r.fetch_budget_ms.load(Ordering::Relaxed));
        o.federation.max_hits = Some(r.max_hits.load(Ordering::Relaxed));
        o.federation.eager_index = Some(r.eager_index.load(Ordering::Relaxed));
    }
    if changed.contains(&"collection.enabled") {
        o.collection.enabled = Some(r.collection_enabled());
    }
    if changed.contains(&"ml.summaries_enabled") {
        o.ml.summaries_enabled = Some(r.summaries_enabled());
    }
    if changed.contains(&"interaction.enabled") {
        o.interaction.enabled = Some(r.interaction_enabled.load(Ordering::Relaxed));
    }
    let persisted = match o.save(path_cfg) {
        Ok(p) => {
            if let Ok(mut ov) = r.overridden.write() {
                for c in &changed {
                    let key = if c.starts_with("ranking") {
                        "ranking"
                    } else {
                        c
                    };
                    if !ov.contains(&key) {
                        ov.push(key);
                    }
                }
            }
            Some(p.display().to_string())
        }
        Err(e) => {
            tracing::warn!(error = %e, "runtime settings applied but not persisted");
            None
        }
    };
    let after = r.snapshot();
    tracing::warn!(
        changed = ?changed,
        peer = ?peer,
        before = %before,
        after = %after,
        "runtime settings changed from the console"
    );
    let mut out = after;
    out["changed"] = json!(changed);
    out["persisted_to"] = json!(persisted);
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_outside_the_bounds_is_refused_before_anything_is_touched() {
        let p = Patch {
            federation: Some(FederationPatch {
                budget_ms: Some(50),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(check(&p).is_err());
        let mut w = Weights::default();
        w.freshness = 0.9;
        let p = Patch {
            ranking: Some(w),
            ..Default::default()
        };
        assert!(check(&p).unwrap_err().contains("relevance"));
        assert!(check(&Patch::default()).is_ok());
    }
}
