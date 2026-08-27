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

/// `GET /admin/config` — the effective configuration, secrets redacted (PROB-003).
///
/// Before this, "what is this system actually running with" required shell access: the console
/// echoed a scattering of values and hinted at config keys it could not show. This is the whole
/// effective `Config` — defaults, file, and environment overrides already merged — as one
/// document. Read-only on purpose: changing config stays a file edit that passes
/// `Config::validate()` on start, so the safety guards cannot be side-stepped from a browser.
///
/// Every secret is REDACTED, never echoed: keys, salts, and any URL that could embed credentials.
/// The redaction marker distinguishes "set" from "empty", which is what an operator actually
/// needs to know about a secret.
pub async fn config(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    authorise(&state, peer, &headers).map_err(|d| d.json())?;
    let mut cfg = (*state.config).clone();
    fn redact(v: &mut String) {
        if !v.trim().is_empty() {
            *v = "«set — redacted»".into();
        }
    }
    redact(&mut cfg.api.admin_key);
    redact(&mut cfg.search.meili_key);
    redact(&mut cfg.discovery.brave_api_key);
    // A proxy URL routinely embeds user:pass — treat the whole value as a secret.
    redact(&mut cfg.discovery.serp_proxy);
    redact(&mut cfg.vector.qdrant_key);
    redact(&mut cfg.interaction.salt);
    Ok(Json(json!({ "config": cfg })))
}

/// Resolve the current settings against the hardware actually present.
/// Image-AI status: which OCR engine is selected and whether it and the image-similarity stack are
/// up. Read-only — the operator's window onto the two optional sidecars and the vector index,
/// closing the loop on "make the OCR backend visible from the admin page".
///
/// Health probes hit the sidecars, so this is a little slower than the plain status call; it is a
/// deliberate operator action, not a hot path.
pub async fn media(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    authorise(&state, peer, &headers).map_err(|d| d.json())?;

    let ocr_backend = state.ocr.name();
    // Only probe when a sidecar is actually in the path — the in-process tesseract engine is always
    // ready, and a probe of it would just be `true` after a wasted network attempt.
    let ocr_healthy = if ocr_backend == "tesseract" {
        true
    } else {
        state.ocr.healthy().await
    };

    let vector = match &state.image_search {
        None => json!({ "enabled": false }),
        Some(engine) => {
            let embedder_healthy = engine.embedder.healthy().await;
            // A point count doubles as the Qdrant reachability probe.
            let (qdrant_ok, points) = match engine.store.count().await {
                Ok(n) => (true, Some(n)),
                Err(_) => (false, None),
            };
            json!({
                "enabled": true,
                "embedder_healthy": embedder_healthy,
                "qdrant_reachable": qdrant_ok,
                "image_vectors": points,
                "embedder_endpoint": state.config.vector.embedder_endpoint,
                "collection": state.config.vector.collection,
            })
        }
    };

    let stt = match &state.stt {
        None => json!({ "enabled": false }),
        Some(client) => json!({
            "enabled": true,
            "healthy": client.healthy().await,
            "breaker": client.breaker_state(),
            "endpoint": state.config.stt.endpoint,
        }),
    };

    Ok(Json(json!({
        "ocr": {
            "backend": ocr_backend,
            "healthy": ocr_healthy,
            "sidecar_endpoint": state.config.media.sidecar.endpoint,
        },
        "vector": vector,
        "stt": stt,
    })))
}

/// Interaction analytics for the operator console (M6-T07): top queries (with category), CTR
/// leaders, and hot re-crawl targets — every figure k-anonymous by construction (the store only
/// returns what clears the floor). Read-only; empty when interaction signals are off.
pub async fn interaction(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    authorise(&state, peer, &headers).map_err(|d| d.json())?;

    let Some(store) = state.interactions() else {
        return Ok(Json(json!({ "enabled": false })));
    };

    let top = store.top_queries(50).await;
    let hot = store
        .hot_docs(state.config.interaction.hot_floor(), 30)
        .await;
    let leaders = store.top_documents(30).await;
    let top_json: Vec<serde_json::Value> = top
        .iter()
        .map(|s| {
            json!({
                "query": s.query,
                "count": s.count,
                "category": s.category,
                // M7-T10 search history: how many results the query returned, and its total clicks.
                "result_count": s.result_count,
                "clicks": s.clicks,
            })
        })
        .collect();

    // Per-category volume rollup (M6-T05.2), from the same k-anonymous rows.
    let mut by_category: std::collections::BTreeMap<String, u32> =
        std::collections::BTreeMap::new();
    for s in &top {
        *by_category.entry(s.category.clone()).or_default() += s.count;
    }

    // Resolve the CTR-leader and hot-doc ids to titles/URLs so the console shows what people
    // actually opened, not opaque ids. One filtered query over the union of ids.
    let mut ids: Vec<String> = leaders.iter().map(|d| d.doc.clone()).collect();
    ids.extend(hot.iter().cloned());
    let titles = resolve_doc_titles(&state, &ids).await;

    let leaders_json: Vec<serde_json::Value> = leaders
        .iter()
        .map(|d| {
            let (title, url) = titles.get(&d.doc).cloned().unwrap_or_default();
            json!({ "doc": d.doc, "impressions": d.impressions, "clicks": d.clicks,
                    "ctr": d.ctr, "title": title, "url": url })
        })
        .collect();
    let hot_json: Vec<serde_json::Value> = hot
        .iter()
        .map(|d| {
            let (title, url) = titles.get(d).cloned().unwrap_or_default();
            json!({ "doc": d, "title": title, "url": url })
        })
        .collect();

    Ok(Json(json!({
        "enabled": true,
        "k_anonymity": state.config.interaction.k_anonymity,
        "window_days": state.config.interaction.window_days,
        // The click floor hot re-crawl acts on — used server-side above, now visible (PROB-003).
        "hot_floor": state.config.interaction.hot_floor(),
        "top_queries": top_json,
        "categories": by_category,
        "ctr_leaders": leaders_json,
        "hot_docs": hot_json,
    })))
}

/// Resolve document ids to `(title, url)` from the lexical index — best-effort, empty on failure.
pub(crate) async fn resolve_doc_titles(
    state: &AppState,
    ids: &[String],
) -> std::collections::HashMap<String, (String, String)> {
    use serde_json::Value;
    let mut out = std::collections::HashMap::new();
    let unique: Vec<&String> = {
        let mut seen = std::collections::HashSet::new();
        ids.iter().filter(|id| seen.insert((*id).clone())).collect()
    };
    if unique.is_empty() {
        return out;
    }
    let quoted: Vec<String> = unique.iter().map(|id| format!("\"{id}\"")).collect();
    let query = xustive_search::Query::new("")
        .filter(format!("id IN [{}]", quoted.join(", ")))
        .limit(unique.len());
    if let Ok(hits) = state
        .search
        .search::<Value>(&state.documents_index(), &query)
        .await
    {
        for h in hits.hits {
            if let Some(id) = h.get("id").and_then(Value::as_str) {
                let title = h
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let url = h
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                out.insert(id.to_string(), (title, url));
            }
        }
    }
    out
}

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

/// `GET /admin/integrations` — the external-tool control surface (M7-T09, [[ADR-0017]]).
///
/// Reports configuration and the runtime switch, not a live health probe: the serving plane cannot
/// reach SearXNG directly — it lives on the egress network behind the [[Federation Gateway]] — so a
/// live probe would violate the no-egress boundary this endpoint exists inside. Live health arrives
/// with the gateway (M7-T04). Nothing here reveals a query; there is none to reveal.
pub async fn integrations(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    authorise(&state, peer, &headers).map_err(|d| d.json())?;
    let f = &state.config.federation;
    // A live probe of the *gateway* is fine: it sits on `core`, which the API may reach — this is not
    // internet egress. The gateway, in turn, is the only thing that reaches SearXNG. If a client
    // exists, probe it and report its breaker state so the operator can see federation is wired.
    let (reachable, breaker) = match &state.federator {
        Some(c) => (c.healthy().await, c.breaker_state()),
        None => (false, "none"),
    };

    // Semantic (dense) text search (M7-T02): a whole retrieval path that otherwise had no console.
    // Probe the embedder sidecar (on `core`) and count the vectors in the text collection.
    let v = &state.config.vector;
    let semantic = match &state.text_search {
        Some(ts) => json!({
            "enabled": true,
            "configured": true,
            "embedder_endpoint": v.text_embedder_endpoint,
            "collection": v.text_collection,
            "dim": v.text_dim,
            "reachable": ts.healthy().await,
            "breaker": ts.embedder.breaker_state(),
            "documents_embedded": ts.store.count().await.ok(),
        }),
        None => json!({ "enabled": v.text_enabled, "configured": false }),
    };

    // Image similarity (M3): the CLIP path. Same shape, so the console shows both vector engines.
    let image = match &state.image_search {
        Some(is) => json!({
            "enabled": true,
            "configured": true,
            "embedder_endpoint": v.embedder_endpoint,
            "collection": v.collection,
            "reachable": is.embedder.healthy().await,
            "images_embedded": is.store.count().await.ok(),
        }),
        None => json!({ "enabled": v.enabled, "configured": false }),
    };

    Ok(Json(json!({
        "federation": {
            "enabled": state.federation_enabled.load(Ordering::Relaxed),
            // "Configured" from the API's side means a gateway client exists to call — which is built
            // whenever `federator_url` is set. `searxng_url` is the gateway's concern, shown for info.
            "configured": state.federator.is_some(),
            "searxng_url": f.searxng_url,
            "federator_url": f.federator_url,
            "budget_ms": f.budget_ms,
            "max_hits": f.max_hits,
            "eager_index": f.eager_index,
            // A live health probe of the gateway (on `core`), plus its circuit-breaker state. The API
            // never talks to SearXNG directly — the gateway does (ADR-0017).
            "reachable_from_api": reachable,
            "breaker": breaker,
        },
        "semantic": semantic,
        "image": image,
        // The external AI summariser (M7-T08): third-party SaaS behind the same gateway, flagged
        // distinctly so the console can say "this one sends data off-box when on". "Configured" from
        // the API's side is the same gateway client federation uses; whether the gateway itself holds
        // an EXTERNAL_LLM_URL is its own deployment concern.
        "external_summariser": {
            "enabled": state.external_summaries.load(Ordering::Relaxed),
            "configured": state.federator.is_some(),
            "third_party": true,
            "attempts_ok": state.metrics.counter_where(crate::metrics::SUMMARY_EXTERNAL, "outcome", "ok"),
            "attempts_failed": state.metrics.counter_total(crate::metrics::SUMMARY_EXTERNAL)
                .saturating_sub(state.metrics.counter_where(crate::metrics::SUMMARY_EXTERNAL, "outcome", "ok")),
        },
        // Effectiveness counters, read straight from the same registry Prometheus exports (M7), so the
        // console shows the payoff without a Grafana round-trip: how often federation contributed,
        // how many URLs it fed the index, and whether the dense leg added recall or just reinforced.
        "effectiveness": {
            "federation_searches_hits": state.metrics.counter_where(crate::metrics::FEDERATION_SEARCHES, "outcome", "hits"),
            "federation_searches_empty": state.metrics.counter_where(crate::metrics::FEDERATION_SEARCHES, "outcome", "empty"),
            "federation_urls_fed": state.metrics.counter_total(crate::metrics::FEDERATION_FED),
            // The convergence measure (T09.2): the web share of served cards falling over time is
            // the index catching up with what people search for.
            "blend_cards_web": state.metrics.counter_where(crate::metrics::FEDERATION_BLEND, "source", "web"),
            "blend_cards_local": state.metrics.counter_where(crate::metrics::FEDERATION_BLEND, "source", "local"),
            "semantic_fused_recall": state.metrics.counter_where(crate::metrics::SEMANTIC_FUSED, "kind", "recall"),
            "semantic_fused_reinforce": state.metrics.counter_where(crate::metrics::SEMANTIC_FUSED, "kind", "reinforce"),
        },
    })))
}

#[derive(Debug, serde::Deserialize)]
pub struct IntegrationUpdate {
    /// Which integration to change: `federation` or `external_summariser`.
    pub integration: String,
    pub enabled: bool,
}

/// `POST /admin/integrations` — turn an external integration on or off at runtime.
///
/// Toggling only flips the runtime switch the consumers read; it does not reach out. Enabling
/// federation without a configured endpoint is refused with a reason rather than silently doing
/// nothing, the same way an empty API key is a misconfiguration, not a mode.
pub async fn set_integrations(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
    Json(update): Json<IntegrationUpdate>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    authorise(&state, peer, &headers).map_err(|d| d.json())?;
    match update.integration.as_str() {
        "federation" => {
            // Refuse to arm federation with no gateway to call — the client is built only when a
            // `federator_url` is configured, so its absence is exactly "no endpoint".
            if update.enabled && state.federator.is_none() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": {
                        "code": "no_endpoint",
                        "message": "set federation.federator_url in config before enabling federation",
                    }})),
                ));
            }
            state
                .federation_enabled
                .store(update.enabled, Ordering::Relaxed);
            tracing::info!(peer = ?peer, enabled = update.enabled, "federation toggled via admin");
            Ok(Json(json!({
                "ok": true,
                "integration": "federation",
                "enabled": update.enabled,
            })))
        }
        "external_summariser" => {
            // Same refusal as federation: the external summariser reaches the world through the
            // gateway, so arming it with no gateway configured is a misconfiguration, not a mode.
            if update.enabled && state.federator.is_none() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": {
                        "code": "no_endpoint",
                        "message": "set federation.federator_url (and the gateway's EXTERNAL_LLM_* environment) before enabling the external summariser",
                    }})),
                ));
            }
            state
                .external_summaries
                .store(update.enabled, Ordering::Relaxed);
            tracing::info!(peer = ?peer, enabled = update.enabled, "external summariser toggled via admin");
            Ok(Json(json!({
                "ok": true,
                "integration": "external_summariser",
                "enabled": update.enabled,
            })))
        }
        other => Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {
                "code": "unknown_integration",
                "message": format!("unknown integration {other:?}"),
            }})),
        )),
    }
}
