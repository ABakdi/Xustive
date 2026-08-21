//! The OCR endpoint: image bytes in, text out.
//!
//! This backs two user-facing features — the standalone OCR tool and the "photograph → search"
//! (Lens-style) flow — and both want the same thing: an image, read to text, that the person can
//! see and edit before anything else happens. The text is *never* auto-submitted to search here;
//! the handler's job ends at returning the text ([[UI - Image Search]] M3-T06.4).
//!
//! # Wire shape
//!
//! `POST /api/v1/ocr` with the raw image as the body (`Content-Type: image/*`). A raw body, not a
//! multipart form: exactly one image is sent, and a form wrapper would only add a parser and a
//! failure mode. The reply is `{ "text", "usable", "confidence", "backend" }`.
//!
//! # Privacy
//!
//! The image and the extracted text are the payload, and neither is ever logged — the same posture
//! as [`crate::translate`]. The bytes reach OCR in memory and are dropped when the request ends;
//! nothing touches disk ([[Security and Privacy]] P4, held by [`xustive_media::ocr`]). A raw POST
//! body keeps the image out of any URL, referrer, or access log.

use axum::body::Bytes;
use axum::extract::State;
use axum::Json;
use serde::Serialize;

use xustive_media::ocr::OcrError;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Serialize)]
pub struct OcrResponse {
    /// The recognised text, whitespace-collapsed and normalised. Empty string when nothing usable
    /// was read — the client shows an empty state rather than an error for a blank image.
    pub text: String,
    /// Whether the text cleared the confidence and length floors. `false` means "we read something,
    /// but do not trust it" — the UI can warn rather than silently offer noise as a search.
    pub usable: bool,
    /// Mean confidence, 0–100. Shown qualitatively (a bar, not a number) in the UI.
    pub confidence: f32,
    /// Which engine produced this — `"tesseract"` or `"unlimited"`. Lets the UI note when the
    /// heavier model was used, and lets an operator confirm the sidecar is actually in the path.
    pub backend: &'static str,
}

/// Read an uploaded image to text.
pub async fn handler(
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Json<OcrResponse>, ApiError> {
    if body.is_empty() {
        return Err(ApiError::BadImage {
            code: "empty_image",
        });
    }
    // The route carries its own body limit (see `lib.rs`), so an oversized upload is rejected by
    // the layer before it reaches here; this is the defence-in-depth check against a misconfigured
    // limit, using the same ceiling the image fetcher uses.
    if body.len() > state.config.media.max_image_bytes {
        return Err(ApiError::BadImage {
            code: "image_too_large",
        });
    }

    let backend = state.ocr.name();
    match state.ocr.recognise(body.to_vec()).await {
        Ok(ocr) => Ok(Json(OcrResponse {
            text: ocr.text,
            usable: ocr.usable,
            confidence: ocr.confidence,
            backend,
        })),
        // A bad image is the user's to fix (wrong file, corrupt, too big); an engine failure is
        // ours. The split is what lets the client tell "try another image" from "try again later".
        Err(OcrError::Format | OcrError::Decode) => Err(ApiError::BadImage {
            code: "undecodable_image",
        }),
        Err(OcrError::TooLarge(_)) => Err(ApiError::BadImage {
            code: "image_too_large",
        }),
        Err(OcrError::Engine(cause)) => {
            // The cause is a backend detail (a tesseract init failure, a sidecar 500), never user
            // data — safe to log, and the operator needs it to tell a missing traineddata dir from
            // a down sidecar.
            tracing::warn!(%cause, backend, "OCR engine failed");
            Err(ApiError::model_unavailable("ocr_unavailable"))
        }
    }
}
