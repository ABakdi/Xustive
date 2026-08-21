//! OCR backends behind one trait.
//!
//! Two engines exist, and they occupy deliberately different places in the system:
//!
//! * [`Tesseract`] runs **in process** on the CPU. It needs nothing but the `*.traineddata` files,
//!   so it is always available — this is the engine the always-on crawl-time enrichment path uses
//!   over every crawled image, and the default for the user-facing tools. It fits the reference
//!   hardware (a 4 GB GPU, or none at all).
//! * [`Sidecar`] is an HTTP client for the optional **Unlimited-OCR** service — a 3 B-parameter
//!   vision-language model (Baidu, MIT) that parses layout, tables and multi-page documents far
//!   better than glyph OCR, but needs a real GPU and a Python runtime. It runs as a separate
//!   process on the private network, so calling it is an *internal-service* call, not internet
//!   egress ([[Deployment Topology]]): the serving plane stays sealed against the public internet.
//!
//! The [`Fallback`] combinator lets a caller prefer the sidecar and quietly drop to tesseract when
//! it is unreachable or errors — so turning the sidecar on never turns a feature *off*.
//!
//! Every backend returns the same scored [`Ocr`]: the usability decision lives once in
//! [`ocr::score`], not per engine.

use std::time::Duration;

use async_trait::async_trait;

use crate::ocr::{self, Ocr, OcrError};

/// One OCR engine. `recognise` takes owned bytes because a backend may hand them to a blocking pool
/// or an HTTP body, both of which outlive the borrow.
#[async_trait]
pub trait OcrBackend: Send + Sync {
    async fn recognise(&self, bytes: Vec<u8>) -> Result<Ocr, OcrError>;
    /// Stable identifier surfaced to the API and admin console (`"tesseract"`, `"unlimited"`).
    fn name(&self) -> &'static str;
}

/// In-process tesseract. Owns its configuration so it can be cloned into a blocking task.
#[derive(Debug, Clone)]
pub struct Tesseract {
    pub tessdata: String,
    pub langs: String,
    pub max_pixels: u64,
}

impl Tesseract {
    pub fn new(tessdata: impl Into<String>, langs: impl Into<String>) -> Self {
        Self {
            tessdata: tessdata.into(),
            langs: langs.into(),
            max_pixels: ocr::MAX_PIXELS,
        }
    }
}

#[async_trait]
impl OcrBackend for Tesseract {
    async fn recognise(&self, bytes: Vec<u8>) -> Result<Ocr, OcrError> {
        // tesseract is blocking and CPU-bound; keep it off the async workers.
        let (tessdata, langs, max) = (self.tessdata.clone(), self.langs.clone(), self.max_pixels);
        tokio::task::spawn_blocking(move || ocr::recognise(&bytes, &tessdata, &langs, max))
            .await
            .map_err(|e| OcrError::Engine(format!("blocking join failed: {e}")))?
    }

    fn name(&self) -> &'static str {
        "tesseract"
    }
}

/// HTTP client for the Unlimited-OCR sidecar.
///
/// The wire contract is intentionally tiny: `POST {endpoint}` with the raw image bytes as the body,
/// reply `{"text": "...", "confidence": <0..100, optional>}`. A VLM has no per-word confidence the
/// way tesseract does, so the sidecar omits it and we treat produced text as high-confidence — the
/// length floor in [`ocr::score`] still rejects an empty or near-empty answer.
#[derive(Clone)]
pub struct Sidecar {
    http: reqwest::Client,
    endpoint: String,
}

/// Confidence assumed when the sidecar returns text but no score of its own.
const ASSUMED_VLM_CONFIDENCE: f32 = 90.0;

#[derive(serde::Deserialize)]
struct SidecarReply {
    text: String,
    #[serde(default)]
    confidence: Option<f32>,
}

impl Sidecar {
    /// Build a client for `endpoint` with a hard request timeout. The timeout is not optional: a 3 B
    /// VLM can wedge, and a user-facing OCR request must fail over to tesseract in bounded time.
    pub fn new(endpoint: impl Into<String>, timeout: Duration) -> Result<Self, OcrError> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| OcrError::Engine(format!("sidecar client: {e}")))?;
        Ok(Self {
            http,
            endpoint: endpoint.into(),
        })
    }

    /// Liveness probe against `{endpoint}/health` — used by the admin console to show whether the
    /// optional sidecar is actually up before anyone selects it.
    pub async fn healthy(&self) -> bool {
        let url = format!("{}/health", self.endpoint.trim_end_matches('/'));
        matches!(self.http.get(url).send().await, Ok(r) if r.status().is_success())
    }
}

#[async_trait]
impl OcrBackend for Sidecar {
    async fn recognise(&self, bytes: Vec<u8>) -> Result<Ocr, OcrError> {
        let resp = self
            .http
            .post(&self.endpoint)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(bytes)
            .send()
            .await
            .map_err(|e| OcrError::Engine(format!("sidecar request: {e}")))?;
        if !resp.status().is_success() {
            return Err(OcrError::Engine(format!(
                "sidecar status {}",
                resp.status()
            )));
        }
        let reply: SidecarReply = resp
            .json()
            .await
            .map_err(|e| OcrError::Engine(format!("sidecar decode: {e}")))?;
        let confidence = reply.confidence.unwrap_or(ASSUMED_VLM_CONFIDENCE);
        Ok(ocr::score(&reply.text, confidence))
    }

    fn name(&self) -> &'static str {
        "unlimited"
    }
}

/// Try `primary`, and on *any* error fall through to `secondary`.
///
/// This is what makes the sidecar safe to prefer: a GPU service that is down, slow, or mid-restart
/// degrades to the always-present tesseract engine instead of failing the request. The `name` a
/// caller sees is the primary's — the fallback is a reliability detail, not a mode.
pub struct Fallback {
    primary: Box<dyn OcrBackend>,
    secondary: Box<dyn OcrBackend>,
}

impl Fallback {
    pub fn new(primary: Box<dyn OcrBackend>, secondary: Box<dyn OcrBackend>) -> Self {
        Self { primary, secondary }
    }
}

#[async_trait]
impl OcrBackend for Fallback {
    async fn recognise(&self, bytes: Vec<u8>) -> Result<Ocr, OcrError> {
        match self.primary.recognise(bytes.clone()).await {
            Ok(ocr) => Ok(ocr),
            Err(e) => {
                tracing::warn!(error = %e, primary = self.primary.name(), "OCR primary failed; falling back");
                self.secondary.recognise(bytes).await
            }
        }
    }

    fn name(&self) -> &'static str {
        self.primary.name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A backend that returns a fixed result, recording that it was asked.
    struct Stub {
        out: Result<Ocr, OcrError>,
        tag: &'static str,
    }

    #[async_trait]
    impl OcrBackend for Stub {
        async fn recognise(&self, _bytes: Vec<u8>) -> Result<Ocr, OcrError> {
            match &self.out {
                Ok(o) => Ok(o.clone()),
                Err(_) => Err(OcrError::Engine("stub".into())),
            }
        }
        fn name(&self) -> &'static str {
            self.tag
        }
    }

    fn ok(text: &str) -> Ocr {
        Ocr {
            text: text.into(),
            confidence: 90.0,
            usable: true,
        }
    }

    #[tokio::test]
    async fn fallback_uses_primary_when_it_succeeds() {
        let fb = Fallback::new(
            Box::new(Stub {
                out: Ok(ok("primary")),
                tag: "unlimited",
            }),
            Box::new(Stub {
                out: Ok(ok("secondary")),
                tag: "tesseract",
            }),
        );
        assert_eq!(fb.recognise(vec![1, 2, 3]).await.unwrap().text, "primary");
        // The reported name is the primary's, even though a fallback exists.
        assert_eq!(fb.name(), "unlimited");
    }

    #[tokio::test]
    async fn fallback_drops_to_secondary_on_primary_error() {
        let fb = Fallback::new(
            Box::new(Stub {
                out: Err(OcrError::Engine("down".into())),
                tag: "unlimited",
            }),
            Box::new(Stub {
                out: Ok(ok("secondary")),
                tag: "tesseract",
            }),
        );
        assert_eq!(fb.recognise(vec![1, 2, 3]).await.unwrap().text, "secondary");
    }

    #[test]
    fn sidecar_reply_decodes_with_and_without_confidence() {
        let with: SidecarReply =
            serde_json::from_str(r#"{"text":"hi","confidence":73.5}"#).unwrap();
        assert_eq!(with.confidence, Some(73.5));
        let without: SidecarReply = serde_json::from_str(r#"{"text":"hi"}"#).unwrap();
        assert_eq!(without.confidence, None);
        // Missing confidence resolves to the assumed VLM confidence, not zero — so real text is
        // never dropped merely because the sidecar declined to score it.
        let scored = ocr::score(
            &without.text,
            without.confidence.unwrap_or(ASSUMED_VLM_CONFIDENCE),
        );
        assert_eq!(scored.confidence, ASSUMED_VLM_CONFIDENCE);
    }

    #[test]
    fn tesseract_reports_its_name() {
        assert_eq!(Tesseract::new("data/tessdata", "eng").name(), "tesseract");
    }
}
