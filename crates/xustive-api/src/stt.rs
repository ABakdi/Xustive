//! The transcription endpoint: audio in, text out.
//!
//! Backs voice search ([[Speech to Text]], M3-T02). The browser records a short clip, posts the raw
//! bytes here, and gets back a transcript that lands in the search box **editable and never
//! auto-submitted** (the frontend enforces that). Transcription itself runs in the STT sidecar
//! (`services/stt-sidecar`), reached as an internal-network service — the same shape as the OCR and
//! CLIP sidecars.
//!
//! # Isolation and availability
//!
//! Voice is optional. When `[stt] enabled = false` or the sidecar is unreachable, the endpoint
//! returns a clean "unavailable" and nothing else is affected — text search never depends on it.
//!
//! # Privacy
//!
//! The audio is a raw POST body, forwarded to the local sidecar, and never stored or logged — voice
//! is among the most sensitive input a person gives, and the entire reason to self-host STT is that
//! it never reaches a cloud API.

use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::state::AppState;

/// A client for the STT sidecar. Present only when `[stt] enabled`.
#[derive(Clone)]
pub struct SttClient {
    http: reqwest::Client,
    endpoint: String,
    max_bytes: usize,
    /// Fail fast when the sidecar is down instead of waiting out the timeout on every request
    /// (M4-T02.2). A tripped breaker returns "unavailable" immediately and probes for recovery.
    breaker: xustive_core::circuit::SharedBreaker,
}

#[derive(Deserialize)]
struct SidecarReply {
    text: String,
    #[serde(default)]
    language: Option<String>,
}

impl SttClient {
    /// Build from `[stt]`, or `None` when disabled or the client cannot be constructed.
    pub fn from_config(cfg: &xustive_core::config::SttConfig) -> Option<Self> {
        if !cfg.enabled {
            return None;
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_millis(cfg.timeout_ms))
            .build()
            .ok()?;
        let breaker = xustive_core::circuit::SharedBreaker::new(xustive_core::circuit::Config {
            failure_threshold: 3,
            cooldown: Duration::from_secs(5),
            max_cooldown: Duration::from_secs(60),
        });
        Some(Self {
            http,
            endpoint: cfg.endpoint.clone(),
            max_bytes: cfg.max_audio_bytes,
            breaker,
        })
    }

    /// The breaker's state, for the admin console (`"closed"`, `"open"`, `"half-open"`).
    pub fn breaker_state(&self) -> &'static str {
        use xustive_core::circuit::State;
        match self.breaker.state() {
            State::Closed => "closed",
            State::Open => "open",
            State::HalfOpen => "half-open",
        }
    }

    /// Liveness probe against the sidecar's `/health` (the endpoint is the `/transcribe` path).
    pub async fn healthy(&self) -> bool {
        let base = self
            .endpoint
            .trim_end_matches("/transcribe")
            .trim_end_matches('/');
        matches!(self.http.get(format!("{base}/health")).send().await, Ok(r) if r.status().is_success())
    }

    async fn transcribe(
        &self,
        audio: Vec<u8>,
        lang: Option<&str>,
        partial: bool,
    ) -> Result<Transcript, ApiError> {
        // Fail fast if the sidecar has been failing — no request, no timeout wait.
        if !self.breaker.allow() {
            return Err(ApiError::model_unavailable("stt_unavailable"));
        }
        match self.try_transcribe(audio, lang, partial).await {
            Ok(t) => {
                self.breaker.on_success();
                Ok(t)
            }
            Err(e) => {
                self.breaker.on_failure();
                Err(e)
            }
        }
    }

    async fn try_transcribe(
        &self,
        audio: Vec<u8>,
        lang: Option<&str>,
        partial: bool,
    ) -> Result<Transcript, ApiError> {
        let mut req = self
            .http
            .post(&self.endpoint)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(audio);
        if let Some(l) = lang {
            req = req.query(&[("lang", l)]);
        }
        if partial {
            req = req.query(&[("partial", "1")]);
        }
        let resp = req.send().await.map_err(|e| {
            tracing::warn!(error = %e, "STT sidecar request failed");
            ApiError::model_unavailable("stt_unavailable")
        })?;
        if !resp.status().is_success() {
            tracing::warn!(status = %resp.status(), "STT sidecar returned an error");
            return Err(ApiError::model_unavailable("stt_unavailable"));
        }
        let reply: SidecarReply = resp.json().await.map_err(|e| {
            tracing::warn!(error = %e, "STT sidecar reply decode failed");
            ApiError::model_unavailable("stt_unavailable")
        })?;
        Ok(Transcript {
            text: reply.text,
            language: reply.language.unwrap_or_default(),
        })
    }
}

#[derive(Serialize)]
pub struct Transcript {
    /// The transcript. May be empty for silence — the client shows an empty state, not an error.
    pub text: String,
    /// Detected (or forced) language code.
    pub language: String,
}

#[derive(Deserialize)]
pub struct TranscribeParams {
    /// Optional language hint (the UI language), so a short Arabic clip is not mis-detected.
    pub lang: Option<String>,
    /// `partial=1` — a reading of the words so far while the person is still speaking. The
    /// sidecar answers these with its fast model and a greedy decode: the box updates every
    /// half-second and the final pass, without this flag, gets the careful model.
    #[serde(default)]
    pub partial: Option<String>,
}

/// `POST /api/v1/transcribe` — turn an uploaded audio clip into text.
pub async fn handler(
    State(state): State<AppState>,
    Query(params): Query<TranscribeParams>,
    body: Bytes,
) -> Result<Json<Transcript>, ApiError> {
    let Some(stt) = state.stt.clone() else {
        return Err(ApiError::model_unavailable("stt_unavailable"));
    };
    if body.is_empty() {
        return Err(ApiError::BadImage {
            code: "empty_audio",
        });
    }
    if body.len() > stt.max_bytes {
        return Err(ApiError::BadImage {
            code: "audio_too_large",
        });
    }
    // Only pass a language hint we recognise, so a stray query value cannot reach the model.
    let lang = params.lang.as_deref().filter(|l| is_known_lang(l));
    let partial = matches!(params.partial.as_deref(), Some("1") | Some("true"));
    let mut transcript = stt.transcribe(body.to_vec(), lang, partial).await?;
    // Defence in depth against whisper's silence hallucinations (M3-T02.6). The sidecar drops
    // low-confidence segments; this blanks a transcript that is *only* a known phantom phrase, which
    // the per-segment signal occasionally lets through.
    transcript.text = strip_artefacts(&transcript.text);
    Ok(Json(transcript))
}

/// Phrases whisper emits on silence or noise, normalised for comparison. When the *entire*
/// transcript reduces to one of these, it is an artefact, not speech, and is blanked — a voice
/// search must not run for a word nobody said. Only whole-transcript matches are removed; a real
/// utterance that merely contains "thank you" is untouched.
const ARTEFACTS: &[&str] = &[
    // English — the notorious ones from video-transcript training data.
    "thank you",
    "thanks for watching",
    "thank you for watching",
    "please subscribe",
    "like and subscribe",
    "you",
    // French. `normalize` keeps Latin accents, so these carry them.
    "merci",
    "merci d'avoir regardé",
    "merci d'avoir regardé cette vidéo",
    "abonnez-vous",
    "sous-titrage",
    // Arabic / Darija — subscribe/like call-outs and the stray "ترجمة".
    "شكرا",
    "شكرا لكم",
    "اشتركوا في القناة",
    "اشترك في القناة",
    "لا تنسوا الاشتراك",
    "ترجمة",
];

/// Blank a transcript that is nothing but a known hallucination phrase.
fn strip_artefacts(text: &str) -> String {
    let normalised = xustive_text::normalize(text);
    let trimmed = normalised
        .trim_end_matches(['.', '!', '؟', '?', '،', ','])
        .trim();
    if ARTEFACTS
        .iter()
        .any(|a| trimmed == xustive_text::normalize(a))
    {
        return String::new();
    }
    // Not an artefact — return the original text (not the normalised form, which would strip
    // diacritics and casing a reader may want).
    text.trim().to_string()
}

/// The languages the UI offers — a whitelist so the `lang` hint is bounded.
fn is_known_lang(l: &str) -> bool {
    matches!(l, "ar" | "ary" | "fr" | "en")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_config_builds_no_client() {
        let cfg = xustive_core::config::SttConfig::default(); // enabled = false
        assert!(SttClient::from_config(&cfg).is_none());
    }

    #[test]
    fn enabled_config_builds_a_client() {
        let cfg = xustive_core::config::SttConfig {
            enabled: true,
            ..Default::default()
        };
        assert!(SttClient::from_config(&cfg).is_some());
    }

    #[test]
    fn only_known_languages_are_forwarded() {
        assert!(is_known_lang("ar"));
        assert!(is_known_lang("ary"));
        assert!(!is_known_lang("zh"));
        assert!(!is_known_lang("'; drop"));
    }

    #[test]
    fn reply_decodes_with_and_without_language() {
        let a: SidecarReply = serde_json::from_str(r#"{"text":"hi","language":"ar"}"#).unwrap();
        assert_eq!(a.language.as_deref(), Some("ar"));
        let b: SidecarReply = serde_json::from_str(r#"{"text":"hi"}"#).unwrap();
        assert_eq!(b.language, None);
    }

    #[test]
    fn a_pure_hallucination_phrase_is_blanked() {
        assert_eq!(strip_artefacts("Thank you."), "");
        assert_eq!(strip_artefacts("Thanks for watching!"), "");
        assert_eq!(strip_artefacts("شكرا لكم"), "");
        assert_eq!(strip_artefacts("Merci d'avoir regardé"), "");
    }

    #[test]
    fn real_speech_containing_an_artefact_phrase_survives() {
        // "thank you for the information about Oran" is a real query — only a *whole* match is an
        // artefact, so this must pass through unchanged.
        let real = "thank you for the information about Oran";
        assert_eq!(strip_artefacts(real), real);
    }

    #[test]
    fn ordinary_transcripts_are_untouched() {
        assert_eq!(
            strip_artefacts("مواقيت الصلاة في وهران"),
            "مواقيت الصلاة في وهران"
        );
        assert_eq!(
            strip_artefacts("  paracetamol dosage  "),
            "paracetamol dosage"
        );
    }
}
