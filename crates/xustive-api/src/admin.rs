//! The admin surface.
//!
//! Small on purpose. It exposes the settings an operator genuinely needs to change while the
//! system is running — chiefly which compute device the models use, since testing CPU behaviour
//! on a GPU machine is a routine thing to want and rebuilding for it is not acceptable.
//!
//! Everything here is read-mostly and changes take effect on the next model load. Nothing on
//! this page can make the process fail to start.

use std::sync::atomic::Ordering;

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use xustive_ml::{device, DeviceConfig, DevicePreference, Registry};

use crate::state::AppState;

/// The peer address, when the server was started with connection info attached.
///
/// `ConnectInfo` itself cannot be extracted optionally, and a handler that hard-fails without it
/// would take the whole admin surface down in any harness that does not provide it. This wraps
/// the lookup so a missing address is an ordinary `None` that the guard can treat as remote.
pub struct Peer(pub Option<SocketAddr>);

impl<S: Send + Sync> FromRequestParts<S> for Peer {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Peer(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ConnectInfo(addr)| *addr),
        ))
    }
}

/// Who is allowed to touch the operator surface.
///
/// Two modes, and the default is the safe one:
///
/// - With `api.admin_key` set, callers must present it in `X-Admin-Key`. This is how a deployment
///   reachable from a network is meant to run.
/// - With no key configured, only loopback callers are admitted. That keeps `make web` usable in
///   a browser with no setup, without silently exposing device settings on a box that binds
///   `0.0.0.0` — which is the default bind address.
///
/// An unknown peer address is treated as remote. The guard errs towards refusing.
pub(crate) fn authorise(
    state: &AppState,
    peer: Option<SocketAddr>,
    headers: &HeaderMap,
) -> Result<(), Denied> {
    let configured = state.config.api.admin_key.as_bytes();
    if !configured.is_empty() {
        let presented = headers
            .get("x-admin-key")
            .map(|v| v.as_bytes())
            .unwrap_or_default();
        return if constant_time_eq(presented, configured) {
            Ok(())
        } else {
            Err(Denied {
                code: "admin_key_required",
                message: "this endpoint requires a valid X-Admin-Key header",
            })
        };
    }

    match peer {
        Some(addr) if addr.ip().is_loopback() => Ok(()),
        _ => Err(Denied {
            code: "admin_local_only",
            message: "the admin surface is restricted to loopback callers; \
                      set XUSTIVE_ADMIN_KEY to allow remote access",
        }),
    }
}

pub(crate) struct Denied {
    code: &'static str,
    message: &'static str,
}

impl Denied {
    pub(crate) fn json(&self) -> (StatusCode, Json<serde_json::Value>) {
        (
            StatusCode::FORBIDDEN,
            Json(json!({"error": {"code": self.code, "message": self.message}})),
        )
    }
}

/// Compare without leaking the length of the matching prefix through timing.
///
/// Overkill for a self-hosted admin key, but the alternative is an equality check that a patient
/// attacker can walk one byte at a time, and the cost here is nothing.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[derive(Debug, Deserialize)]
pub struct DeviceUpdate {
    pub preference: Option<String>,
    /// `null` means "decide automatically from available memory".
    pub gpu_layers: Option<i64>,
}

pub async fn status(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    authorise(&state, peer, &headers).map_err(|d| d.json())?;
    Ok(Json(json!({
        "device": current_resolution(&state),
        "gpu_support_compiled": device::gpu_support_compiled(),
        "gpu_detected": device::detect_gpu(),
        "ignore_politeness": state.ignore_politeness.load(Ordering::Relaxed),
        "models": Registry::new(&state.config.ml.model_dir).status(),
        "logging": {
            "filter": crate::telemetry::level_status().0,
            "baseline": crate::telemetry::level_status().1,
            "override_expires_in": crate::telemetry::level_status().2,
        },
        "index": {
            "alias": state.config.search.documents_index,
            "documents": state.documents_index(),
            "meili_url": state.config.search.meili_url,
        },
        "ranking": &*state.ranking,
    })))
}

/// `POST /admin/device` — change the compute device.
///
/// Takes effect on the next model load rather than immediately: tearing down a model mid-request
/// would fail whatever generation is in flight for no benefit.
pub async fn set_device(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
    Json(update): Json<DeviceUpdate>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    authorise(&state, peer, &headers).map_err(|d| d.json())?;
    if let Some(p) = &update.preference {
        let Some(pref) = DevicePreference::parse(p) else {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": {
                    "code": "invalid_device",
                    "message": "preference must be auto, gpu or cpu",
                }})),
            ));
        };
        state.device_preference.store(pref as u8, Ordering::Relaxed);
    }

    if let Some(layers) = update.gpu_layers {
        if !(0..=999).contains(&layers) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": {
                    "code": "invalid_gpu_layers",
                    "message": "gpu_layers must be between 0 and 999",
                }})),
            ));
        }
        state.gpu_layers.store(layers, Ordering::Relaxed);
    }

    tracing::info!(
        preference = ?update.preference,
        gpu_layers = ?update.gpu_layers,
        "device settings changed"
    );

    Ok(Json(json!({
        "ok": true,
        "device": current_resolution(&state),
        "note": "takes effect on the next model load",
    })))
}

/// Resolve the current settings against the hardware actually present.
fn current_resolution(state: &AppState) -> device::Resolved {
    // Prefer what the engine actually resolved to at load time over a fresh probe.
    //
    // A fresh `resolve` re-measures free VRAM, and once our own model is loaded that measurement
    // includes the memory the model is using — so a summariser running happily on the GPU makes
    // the page report "cpu, not enough free memory". The engine already knows where it loaded;
    // ask it. The live probe is only the answer before the first model load, or after a device
    // change that has not taken effect yet.
    if let Ok(guard) = state.engine.read() {
        if let Some(engine) = guard.as_ref() {
            return engine.resolved().clone();
        }
    }

    let pref = match state.device_preference.load(Ordering::Relaxed) {
        1 => DevicePreference::Gpu,
        2 => DevicePreference::Cpu,
        _ => DevicePreference::Auto,
    };
    let layers = state.gpu_layers.load(Ordering::Relaxed);
    let registry = Registry::new(&state.config.ml.model_dir);
    let size = registry
        .resolve(xustive_ml::registry::Role::Summariser, None)
        .map(|s| s.spec.size_mib)
        .unwrap_or(2000);

    device::resolve(
        &DeviceConfig {
            preference: pref,
            gpu_layers: if layers < 0 {
                None
            } else {
                Some(layers as u32)
            },
            ..Default::default()
        },
        size,
    )
}

#[derive(Debug, Deserialize)]
pub struct LevelUpdate {
    /// A `tracing` filter, e.g. `info,xustive=debug`. Omit to revert immediately.
    pub filter: Option<String>,
}

/// `POST /admin/log-level` — raise or lower logging without a restart.
///
/// Every override expires on its own after fifteen minutes. Debug logging is expensive on a busy
/// search engine and is the state in which the most sensitive data comes closest to being written
/// down; relying on someone to turn it off again is relying on the step that never happens.
pub async fn set_log_level(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
    Json(update): Json<LevelUpdate>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    authorise(&state, peer, &headers).map_err(|d| d.json())?;

    let Some(filter) = update.filter else {
        let baseline =
            crate::telemetry::revert_level().map_err(|e| bad_request("invalid_filter", e))?;
        tracing::info!(%baseline, "log level reverted by operator");
        return Ok(Json(json!({ "filter": baseline, "expires_in": null })));
    };

    let expires_in =
        crate::telemetry::set_level(&filter).map_err(|e| bad_request("invalid_filter", e))?;
    // Logged at the level being left, so the record of the change survives even when the new
    // filter would have hidden it.
    tracing::warn!(%filter, expires_in, "log level raised by operator");

    let (current, baseline, remaining) = crate::telemetry::level_status();
    Ok(Json(json!({
        "filter": current,
        "baseline": baseline,
        "expires_in": remaining.unwrap_or(expires_in),
    })))
}

fn bad_request(code: &'static str, message: String) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error": {"code": code, "message": message}})),
    )
}

#[derive(Debug, serde::Deserialize)]
pub struct PolitenessUpdate {
    pub ignore_politeness: bool,
}

/// `POST /admin/politeness` — turn the crawler's politeness bypass on or off.
///
/// **Testing only.** With this on, the crawler does not fetch or consult `robots.txt`, does not
/// wait between requests to a host, ignores adaptive slowdown from 429 and 503, and ignores the
/// host opt-out list. It exists so a fixture site can be crawled at full speed without a robots
/// round trip per request.
///
/// The global and takedown blocklists are **not** bypassed. Those are not politeness — one is a
/// safety block and the other is a legal order, and a testing flag must not be able to lift a
/// court order. Nothing about crawling a local fixture site needs them lifted.
///
/// Pointed at the open web this produces exactly the behaviour the politeness layer exists to
/// prevent, and the damage lands on somebody else's server where we would never see it. So
/// turning it on is logged at `warn` with the peer that did it, and production refuses to start
/// with it enabled at all — meaning this endpoint can only flip it where it is already permitted.
pub async fn set_politeness(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
    Json(update): Json<PolitenessUpdate>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    authorise(&state, peer, &headers).map_err(|d| d.json())?;

    // Belt as well as braces. The startup guard already refuses this configuration in production,
    // but an endpoint that can enable abusive crawling should not depend on a check that ran once,
    // hours ago, in a different function.
    if update.ignore_politeness {
        if let Err(e) = state.config.crawl.guard(&state.config.environment) {
            return Err((
                StatusCode::FORBIDDEN,
                Json(json!({"error": {
                    "code": "not_permitted_here",
                    "message": e.to_string(),
                }})),
            ));
        }
        tracing::warn!(
            peer = ?peer,
            "POLITENESS BYPASS ENABLED via admin — robots.txt, crawl delays and host opt-outs \
             are now ignored. This is for fixture sites only."
        );
    } else {
        tracing::info!(peer = ?peer, "politeness bypass disabled");
    }

    state
        .ignore_politeness
        .store(update.ignore_politeness, Ordering::Relaxed);

    Ok(Json(json!({
        "ok": true,
        "ignore_politeness": update.ignore_politeness,
        "note": "takedown and global blocklists are never bypassed",
    })))
}
