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
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use xustive_ml::{device, DeviceConfig, DevicePreference, Registry};

use crate::state::AppState;
use crate::web::escape_html;

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
fn authorise(
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

struct Denied {
    code: &'static str,
    message: &'static str,
}

impl Denied {
    fn json(&self) -> (StatusCode, Json<serde_json::Value>) {
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

/// `GET /admin/status` — what the system is currently doing.
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
        "models": Registry::new(&state.config.ml.model_dir).status(),
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

/// `GET /admin` — a small page for the settings above.
pub async fn page(State(state): State<AppState>, Peer(peer): Peer, headers: HeaderMap) -> Response {
    if let Err(d) = authorise(&state, peer, &headers) {
        return (
            StatusCode::FORBIDDEN,
            Html(crate::web::admin_shell(
                "Xustive admin",
                &format!(
                    r#"<main id="results"><h1>Not available</h1><p>{}</p></main>"#,
                    escape_html(d.message)
                ),
            )),
        )
            .into_response();
    }
    let r = current_resolution(&state);
    let models = Registry::new(&state.config.ml.model_dir).status();

    let gpu_row = match &r.gpu {
        Some(g) => format!(
            "{} · {} MiB total, {} MiB free · driver {}",
            escape_html(&g.name),
            g.total_mib,
            g.free_mib,
            escape_html(&g.driver)
        ),
        None => "none detected".to_string(),
    };

    let model_rows: String = models
        .iter()
        .map(|m| {
            format!(
                r#"<tr><td>{}</td><td>{}</td><td>{} MiB</td><td>{}</td><td class="{}">{}</td></tr>"#,
                escape_html(m.spec.id),
                escape_html(m.spec.licence),
                m.spec.size_mib,
                escape_html(m.spec.notes),
                if m.present { "state-ok" } else { "state-missing" },
                if m.present { "present" } else { "not downloaded" }
            )
        })
        .collect();

    let selected = |p: &str| {
        if r.preference.as_str() == p {
            " selected"
        } else {
            ""
        }
    };
    let layers = state.gpu_layers.load(Ordering::Relaxed);

    let body = format!(
        r#"<header class="site-header"><a class="wordmark" href="/">XUSTIVE</a>
  <span class="muted">admin</span></header>
<main id="results">
  <h1>Compute device</h1>

  <p class="lede state-{state_class}">Running on <strong>{active}</strong> — {reason}</p>

  <table class="admin">
    <tr><th>GPU</th><td>{gpu_row}</td></tr>
    <tr><th>GPU support compiled in</th><td>{compiled}</td></tr>
    <tr><th>Layers offloaded</th><td>{layers_display}</td></tr>
  </table>

  <form class="admin" id="device-form">
    <label>Device preference
      <select name="preference">
        <option value="auto"{sel_auto}>Auto — use the GPU when it is usable</option>
        <option value="gpu"{sel_gpu}>GPU — prefer the GPU, fall back to CPU</option>
        <option value="cpu"{sel_cpu}>CPU — force CPU even with a GPU present</option>
      </select>
    </label>
    <label>GPU layers
      <input type="number" name="gpu_layers" min="-1" max="999" value="{layers}">
      <span class="hint">-1 decides from free memory. 0 is CPU-only.</span>
    </label>
    <button type="submit">Apply</button>
    <span id="result" class="muted"></span>
  </form>

  <h2>Models</h2>
  <table class="admin">
    <tr><th>Model</th><th>Licence</th><th>Size</th><th>Notes</th><th>Status</th></tr>
    {model_rows}
  </table>
  <p class="muted">Models live in <code>{dir}</code>. A device change takes effect on the next
  model load.</p>
</main>"#,
        state_class = if r.fell_back { "warn" } else { "ok" },
        active = r.active.as_str(),
        reason = escape_html(&r.reason),
        gpu_row = gpu_row,
        compiled = if device::gpu_support_compiled() {
            "yes"
        } else {
            "no — rebuild with <code>--features cuda</code> to use the GPU"
        },
        layers_display = if r.gpu_layers == u32::MAX {
            "all".to_string()
        } else {
            r.gpu_layers.to_string()
        },
        sel_auto = selected("auto"),
        sel_gpu = selected("gpu"),
        sel_cpu = selected("cpu"),
        layers = layers,
        model_rows = model_rows,
        dir = escape_html(&state.config.ml.model_dir),
    );

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        Html(crate::web::admin_shell("Xustive admin", &body)),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_preference_maps_to_and_from_the_atomic_encoding() {
        // The state is an atomic u8 so it can be changed without a lock on the request path.
        for (n, p) in [
            (0u8, DevicePreference::Auto),
            (1, DevicePreference::Gpu),
            (2, DevicePreference::Cpu),
        ] {
            assert_eq!(p as u8, n);
        }
    }

    #[test]
    fn invalid_preferences_are_rejected() {
        assert!(DevicePreference::parse("turbo").is_none());
    }

    #[test]
    fn constant_time_compare_agrees_with_equality() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secrey"));
        assert!(!constant_time_eq(b"secret", b"secretx"), "length differs");
        assert!(!constant_time_eq(b"", b"secret"));
        assert!(
            constant_time_eq(b"", b""),
            "empty keys compare equal; the guard must never reach here with one"
        );
    }
}
