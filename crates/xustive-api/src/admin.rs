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

/// The admin page's stylesheet and script, embedded in the binary.
///
/// Served from dedicated routes rather than inlined so the CSP stays `default-src 'self'` with no
/// hashes to keep in sync, and embedded rather than read from disk so this page cannot break by
/// someone moving a directory — which is precisely what happened when the old UI was deleted.
pub const ADMIN_CSS: &str = include_str!("../assets/admin.css");
pub const ADMIN_JS: &str = include_str!("../assets/admin.js");

pub async fn admin_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        ADMIN_CSS,
    )
}

pub async fn admin_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        ADMIN_JS,
    )
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

/// `GET /admin` — a small page for the settings above.
pub async fn page(State(state): State<AppState>, Peer(peer): Peer, headers: HeaderMap) -> Response {
    if let Err(d) = authorise(&state, peer, &headers) {
        return (
            StatusCode::FORBIDDEN,
            Html(admin_shell(
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

    let bypass_on = state.ignore_politeness.load(Ordering::Relaxed);
    let bypass_allowed = state.config.crawl.guard(&state.config.environment).is_ok();
    // Rendered at the top of the page and impossible to miss when on. A destructive mode that
    // looks like every other setting is one that gets left enabled.
    let bypass_banner = if bypass_on {
        r#"<p class="lede state-bad"><strong>POLITENESS BYPASS IS ON.</strong>
  robots.txt, crawl delays and host opt-outs are being ignored. This is for fixture sites.
  Turn it off before crawling anything you do not own.</p>"#
    } else {
        ""
    };
    let bypass_control = if bypass_allowed {
        format!(
            r#"<h1>Crawler politeness</h1>
  <p class="lede">Bypass is <strong>{}</strong>.</p>
  <form class="admin" id="politeness-form">
    <label><input type="checkbox" id="ignore-politeness" {}> Ignore robots.txt, crawl delays and host opt-outs</label>
    <button type="submit">Apply</button>
  </form>
  <p class="muted">Testing only — for crawling the local fixture site without a robots round trip
  per request. Takedown and global blocklists are never bypassed: those are a legal order and a
  safety block, not politeness. This environment is <code>{}</code>.</p>"#,
            if bypass_on { "ON" } else { "off" },
            if bypass_on { "checked" } else { "" },
            state.config.environment,
        )
    } else {
        format!(
            r#"<h1>Crawler politeness</h1>
  <p class="lede">Bypass is <strong>not available</strong> in <code>{}</code>.</p>
  <p class="muted">The politeness bypass is a testing facility. It is refused outside development
  because pointed at the open web it produces exactly the behaviour the politeness layer exists to
  prevent, and the damage lands on somebody else's server.</p>"#,
            state.config.environment,
        )
    };

    // No header and no <main> of its own: the console shell provides both, and the first version
    // of this page kept its pre-console wrapper — which nested a second <main> inside the shell's
    // and painted the wordmark twice, once in the banner and once at the top of the content.
    let body = format!(
        r#"{bypass_banner}
  {bypass_control}

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
"#,
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
        Html(console("/admin/compute", &body)),
    )
        .into_response()
}

// --- rendering helpers ---------------------------------------------------------------
//
// Moved here when the hand-written HTML renderer was deleted. The admin page is the one surface
// still served by this process — an operator tool, not part of the product, and putting it behind
// the frontend would mean device settings become unreachable exactly when the frontend is the
// thing that is broken.

/// Sections, in sidebar order.
///
/// Real URLs rather than tabs in one page: bookmarkable, linkable during an incident, and each
/// loads on its own — a single page that fetched everything up front would be slowest exactly when
/// one subsystem is unwell.
pub const SECTIONS: &[(&str, &str, &str)] = &[
    ("", "Overview", "/admin"),
    ("CRAWLER", "Live", "/admin/crawler"),
    ("CRAWLER", "Documents", "/admin/documents"),
    ("CRAWLER", "Sources", "/admin/sources"),
    ("CRAWLER", "Source health", "/admin/sources/health"),
    ("SYSTEM", "Compute", "/admin/compute"),
];

/// The shell: header, status bar, sidebar, content.
///
/// The status bar is on every section, not only the live one. "Is it still running" gets asked
/// while you are looking at something else, and making someone navigate away to answer it is how
/// they stop checking.
pub fn console(current: &str, body: &str) -> String {
    let mut nav = String::new();
    let mut group = "";
    for (g, label, href) in SECTIONS {
        if g != &group {
            if !g.is_empty() {
                nav.push_str(&format!("<div class=\"nav-group\">{g}</div>"));
            }
            group = g;
        }
        let active = if *href == current {
            " class=\"on\""
        } else {
            ""
        };
        nav.push_str(&format!("<a href=\"{href}\"{active}>{label}</a>"));
    }

    let page = format!(
        r#"<header class="site-header">
  <a class="wordmark" href="/admin">XUSTIVE</a>
  <span class="muted">admin</span>
  <span class="statusbar" id="statusbar">
    <span class="dot" id="sb-dot"></span>
    <span id="sb-state">…</span>
    <span class="muted" id="sb-rate"></span>
  </span>
</header>
<div class="console">
  <nav class="sidebar">{nav}</nav>
  <main id="results">{body}</main>
</div>"#
    );
    admin_shell("Xustive admin", &page)
}

pub fn admin_shell(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en" dir="ltr" class="admin-page">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="referrer" content="no-referrer">
<title>{title}</title>
<link rel="stylesheet" href="/admin.css">
<script src="/admin.js" defer></script>
</head>
<body>
{body}
</body>
</html>"#,
        title = escape_html(title),
    )
}

pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(ch),
        }
    }
    out
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
