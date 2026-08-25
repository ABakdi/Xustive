//! The summary endpoint.
//!
//! Summaries are fetched **after** the results page has rendered, not with it. On CPU a 3B model
//! takes tens of seconds; blocking search on that would trade the whole product for one feature.
//! So `/v1/search` returns a token, the page paints, and the browser asks for the summary
//! separately — arriving late, or not at all, costs nothing that is already on screen.
//!
//! The token also binds a summary request to a search we actually performed. Letting a caller
//! post arbitrary passages would turn the endpoint into a free text generator pointed at our
//! hardware.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use xustive_ml::prompt::{self, OutputLang, Passage};
use xustive_ml::validate::{self, Citation};

use crate::error::ApiError;
use crate::state::AppState;

/// How long a token stays usable. Long enough for a page to load and ask, short enough that the
/// query text does not linger in memory — the passages are user-visible content, but the query
/// is the sensitive part ([[Security and Privacy]] P1).
const TOKEN_TTL: Duration = Duration::from_secs(120);
/// Ceiling on outstanding tokens, so a flood of searches cannot grow the map without bound.
const MAX_PENDING: usize = 4096;

/// A search whose summary has not been requested yet.
#[derive(Clone)]
pub struct Pending {
    pub query: String,
    pub lang: OutputLang,
    pub passages: Vec<Passage>,
    created: Instant,
}

/// Tokens awaiting a summary request.
///
/// In-process and deliberately not Redis: this is per-request scratch that must not outlive the
/// process, and putting queries in a shared store is exactly the durability we do not want.
#[derive(Default)]
pub struct PendingStore {
    inner: std::sync::Mutex<HashMap<String, Pending>>,
}

impl PendingStore {
    /// Register a search and return its token.
    pub fn insert(&self, query: String, lang: OutputLang, passages: Vec<Passage>) -> String {
        let token = xustive_core::new_id();
        let mut map = match self.inner.lock() {
            Ok(m) => m,
            Err(poisoned) => poisoned.into_inner(),
        };

        // Sweep on write. A background timer would be tidier, but this map is only ever touched
        // by requests, so a request is the only moment its size can have changed.
        let now = Instant::now();
        map.retain(|_, p| now.duration_since(p.created) < TOKEN_TTL);
        if map.len() >= MAX_PENDING {
            // Under genuine pressure, drop the oldest rather than refusing to record new ones:
            // a missing summary on an old page beats no summaries at all on new ones.
            if let Some(oldest) = map
                .iter()
                .min_by_key(|(_, p)| p.created)
                .map(|(k, _)| k.clone())
            {
                map.remove(&oldest);
            }
        }

        map.insert(
            token.clone(),
            Pending {
                query,
                lang,
                passages,
                created: now,
            },
        );
        token
    }

    /// Take a token, consuming it. One summary per search; a repeated request re-runs nothing.
    fn take(&self, token: &str) -> Option<Pending> {
        let mut map = match self.inner.lock() {
            Ok(m) => m,
            Err(poisoned) => poisoned.into_inner(),
        };
        let pending = map.remove(token)?;
        (Instant::now().duration_since(pending.created) < TOKEN_TTL).then_some(pending)
    }

    pub fn len(&self) -> usize {
        self.inner.lock().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Deserialize)]
pub struct SummaryRequest {
    pub token: String,
}

/// Always 200, even when there is no summary.
///
/// A missing summary is a normal outcome — the passages did not support an answer, the model was
/// busy, the machine is slow. None of those are errors the user can act on, and the results are
/// on the page regardless ([[Error Handling and Resilience]] §6).
#[derive(Debug, Serialize)]
pub struct SummaryResponse {
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub citations: Vec<Citation>,
    /// Why there is no summary. Diagnostic, not user-facing copy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    pub took_ms: u64,
}

impl SummaryResponse {
    fn none(reason: &'static str, started: Instant) -> Self {
        Self {
            summary: None,
            citations: Vec::new(),
            reason: Some(reason),
            took_ms: started.elapsed().as_millis() as u64,
        }
    }
}

pub async fn handler(
    State(state): State<AppState>,
    Json(req): Json<SummaryRequest>,
) -> Result<Json<SummaryResponse>, ApiError> {
    let started = Instant::now();

    let Some(pending) = state.pending.take(&req.token) else {
        // Expired, already used, or never issued. Indistinguishable on purpose — a caller
        // probing for valid tokens learns nothing from the response.
        return Ok(Json(SummaryResponse::none("unknown_token", started)));
    };

    let Some(built) = prompt::build(&pending.query, pending.lang, &pending.passages) else {
        return Ok(Json(SummaryResponse::none("no_passages", started)));
    };
    // Cloned for the validator, which needs the shown passages to resolve `[n]` markers. The prompt
    // itself is rebuilt per attempt inside `generate` (it is cheap and each attempt consumes one).
    let cited = built.cited.clone();

    // One deadline for the whole request, shared between the external attempt and the local
    // fallback (BUG-005): two full budgets in sequence meant enabling the external summariser could
    // *double* the worst-case latency. The external leg gets at most half, so a hung provider can
    // never starve the local model of its shot; the local leg gets whatever actually remains.
    let overall = Duration::from_millis(state.config.ml.deadline_ms);

    // The external summariser first, when the operator has switched it on (M7-T08): the same
    // prompt, carried out by the LLM behind the Federation Gateway, held to the same validator —
    // an external model does not get to skip citations or switch language. Every failure falls
    // through to the local model, so turning this on can only ever change who writes the summary,
    // never whether one is possible.
    let external = if state
        .external_summaries
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        generate_external(&state, &built, &cited, pending.lang, overall / 2).await
    } else {
        None
    };
    drop(built);

    let outcome = match external {
        Some(summary) => Ok(summary),
        None => {
            let remaining = overall.saturating_sub(started.elapsed());
            generate(&state, &pending, &cited, remaining).await
        }
    };
    let took_ms = started.elapsed().as_millis() as u64;

    Ok(Json(match outcome {
        Ok(summary) => {
            state.metrics.observe(
                crate::metrics::SUMMARY_DURATION,
                crate::metrics::SUMMARY_DURATION_HELP,
                &[("outcome", "ok")],
                started.elapsed().as_secs_f64(),
            );
            SummaryResponse {
                summary: Some(summary.text),
                citations: summary.citations,
                reason: None,
                took_ms,
            }
        }
        Err(reason) => {
            state.metrics.incr(
                crate::metrics::SUMMARY_WITHHELD,
                crate::metrics::SUMMARY_WITHHELD_HELP,
                &[("reason", reason)],
            );
            SummaryResponse {
                summary: None,
                citations: Vec::new(),
                reason: Some(reason),
                took_ms,
            }
        }
    }))
}

/// One attempt against the external summariser via the Federation Gateway (M7-T08). `None` on any
/// failure — gateway absent, over budget, provider down, or the text failing the same validation
/// the local model faces — so the caller falls back to the local path. Deliberately not
/// feature-gated: a CPU-only build without the `summariser` feature can still serve summaries this
/// way once the operator enables it. Metric records the outcome, never the text.
async fn generate_external(
    state: &AppState,
    built: &prompt::Prompt,
    cited: &[prompt::Cited],
    lang: OutputLang,
    budget: Duration,
) -> Option<validate::Summary> {
    let federator = state.federator.as_ref()?;
    // One chat message carrying the same system rules + grounded passages the local model gets.
    let prompt_text = format!("{}\n\n{}", built.system, built.user);
    let text = federator
        .summarise(&prompt_text, budget.as_millis() as u64)
        .await;
    let outcome = match text {
        Some(text) => match validate::check(&text, cited, lang) {
            Ok(summary) => ("ok", Some(summary)),
            // Never log the text or the query — only why it was withheld, same as the local path.
            Err(r) => (r.as_str(), None),
        },
        None => ("unavailable", None),
    };
    state.metrics.incr(
        crate::metrics::SUMMARY_EXTERNAL,
        crate::metrics::SUMMARY_EXTERNAL_HELP,
        &[("outcome", outcome.0)],
    );
    outcome.1
}

/// Rejections a second, differently-sampled attempt has a real chance of clearing. Deliberate
/// outcomes (the model answering INSUFFICIENT, an injection attempt) and infra failures are not
/// retried — a retry would only burn time and a model slot to reach the same conclusion.
#[cfg(feature = "summariser")]
fn is_recoverable(reason: &str) -> bool {
    matches!(reason, "uncited" | "wrong_language" | "contact_details")
}

#[cfg(feature = "summariser")]
async fn generate(
    state: &AppState,
    pending: &Pending,
    cited: &[prompt::Cited],
    overall: Duration,
) -> Result<validate::Summary, &'static str> {
    use xustive_ml::engine::{EngineError, Sampling};

    let Some(engine) = state.summariser() else {
        return Err("model_not_loaded");
    };

    // Up to two attempts within the budget the caller has left (BUG-005: this used to be a fresh
    // full deadline even after an external attempt had spent one). The first is faithful (low
    // temperature); if it is withheld for a reason a different sample could fix — a forgotten
    // citation, a language the model slipped out of — a second, more varied attempt often clears
    // it. The retry is gated on time remaining, so on slow CPU (where the first attempt eats the
    // budget) there is simply no second try, while on GPU the retry comes for free.
    let start = Instant::now();
    let mut last = "generation_failed";

    for attempt in 0..2 {
        let remaining = overall.saturating_sub(start.elapsed());
        if remaining < Duration::from_secs(3) {
            break;
        }
        let sampling = if attempt == 0 {
            Sampling::default()
        } else {
            // A hotter sample for the second try: a different path through the passages, still
            // grounded in them, is exactly what a transient miss needs.
            Sampling {
                temperature: 0.7,
                top_p: 0.95,
                ..Sampling::default()
            }
        };
        let Some(prompt) = prompt::build(&pending.query, pending.lang, &pending.passages) else {
            return Err("no_passages");
        };
        let generated = match engine.generate(prompt, sampling, remaining).await {
            Ok(g) => g,
            Err(EngineError::Busy) => return Err("busy"),
            Err(EngineError::Timeout) => return Err("timeout"),
            Err(e) => {
                tracing::warn!(error = %e, "summary generation failed");
                return Err("generation_failed");
            }
        };

        tracing::debug!(
            attempt,
            tokens = generated.tokens,
            total_ms = generated.total.as_millis() as u64,
            "summary generated"
        );

        // Never log the text or the query: only why it was withheld ([[Security and Privacy]] P1).
        match validate::check(&generated.text, cited, pending.lang) {
            Ok(summary) => return Ok(summary),
            Err(r) => {
                last = r.as_str();
                if !is_recoverable(last) {
                    return Err(last);
                }
            }
        }
    }
    Err(last)
}

#[cfg(not(feature = "summariser"))]
async fn generate(
    _state: &AppState,
    _pending: &Pending,
    _cited: &[prompt::Cited],
    _overall: Duration,
) -> Result<validate::Summary, &'static str> {
    Err("summariser_not_compiled")
}

/// Turn ranked hits into summariser passages.
///
/// Takes the raw documents rather than the shaped result cards: cards carry a highlighted
/// excerpt with markup and a fixed length, whereas the summariser wants clean body text it can
/// excerpt around the query itself.
pub fn passages_from_hits(hits: &[&Value], limit: usize) -> Vec<Passage> {
    hits.iter()
        .take(limit)
        .filter_map(|hit| {
            let id = hit.get("id")?.as_str()?.to_string();
            let text = hit
                .get("body")
                .and_then(Value::as_str)
                .or_else(|| hit.get("excerpt").and_then(Value::as_str))
                .unwrap_or_default();
            if text.trim().is_empty() {
                return None;
            }
            Some(Passage {
                id,
                title: hit
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                text: text.to_string(),
                domain: hit
                    .get("domain")
                    .or_else(|| hit.get("source_name"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                published_at: hit.get("published_at").and_then(Value::as_i64),
                quality_score: hit
                    .get("quality_score")
                    .and_then(Value::as_f64)
                    .unwrap_or(1.0) as f32,
                spam_score: hit.get("spam_score").and_then(Value::as_f64).unwrap_or(0.0) as f32,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> PendingStore {
        PendingStore::default()
    }

    #[test]
    fn a_token_can_be_redeemed_once() {
        // One summary per search. A token that could be replayed would let a caller pin a slot
        // open indefinitely against a queue only eight deep.
        let s = store();
        let t = s.insert("q".into(), OutputLang::Arabic, Vec::new());
        assert!(s.take(&t).is_some());
        assert!(s.take(&t).is_none());
    }

    #[test]
    fn an_unknown_token_is_simply_absent() {
        assert!(store().take("nonsense").is_none());
    }

    #[test]
    fn the_store_stays_bounded_under_pressure() {
        let s = store();
        for i in 0..MAX_PENDING + 200 {
            s.insert(i.to_string(), OutputLang::Arabic, Vec::new());
        }
        assert!(s.len() <= MAX_PENDING, "store grew to {} entries", s.len());
    }

    #[test]
    fn passages_skip_documents_with_no_text() {
        // A document with no body cannot ground anything, and including it wastes a citation
        // slot the model will then use.
        let empty = serde_json::json!({"id": "a", "body": "   ", "domain": "x.dz"});
        let full = serde_json::json!({"id": "b", "body": "نص حقيقي", "domain": "y.dz"});
        let hits = vec![&empty, &full];
        let out = passages_from_hits(&hits, 8);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "b");
    }

    #[test]
    fn passages_fall_back_to_the_excerpt_when_there_is_no_body() {
        let hit = serde_json::json!({"id": "a", "excerpt": "ملخص قصير", "domain": "x.dz"});
        let hits = vec![&hit];
        let out = passages_from_hits(&hits, 8);
        assert_eq!(out[0].text, "ملخص قصير");
    }
}
