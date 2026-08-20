//! Translation, streamed.
//!
//! Runs on the same local model as the summariser. That is the whole feature: a translation box
//! receives some of the most sensitive text a person ever types — a medical letter, a contract, a
//! message they are anxious about getting right — and every mainstream translation service is a
//! remote endpoint that receives all of it. Here nothing leaves the machine.
//!
//! # Why this streams when the summariser does not
//!
//! The summariser withholds its output until the validator has checked every citation, so
//! streaming would only let it display text it might then have to retract. Translation has
//! nothing equivalent to withhold, and on CPU a 3B model produces a paragraph in something like
//! ten seconds. First text at 400 ms against a paragraph at 12 s is the difference between a
//! feature people use and one they navigate away from.
//!
//! # Cancellation
//!
//! Closing the connection drops the receiver, the worker notices on its next token and stops. This
//! is not a nicety: with two slots on a 4 GB card, one abandoned generation running to completion
//! is half the capacity spent producing text nobody will read.
//!
//! # Privacy
//!
//! The text being translated is never logged, at any level. It is the single most sensitive field
//! this service handles, and it is a POST body precisely so it never reaches a URL, a referrer, or
//! an access log.

use std::convert::Infallible;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::Json;
use futures_util::stream::Stream;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::state::AppState;

/// Ceiling on one translation.
///
/// Generous compared with search, because a translation is something the reader is waiting on
/// deliberately rather than a page they expect instantly. Still bounded: without it a pathological
/// prompt could hold a model slot indefinitely.
const BUDGET: Duration = Duration::from_secs(60);

/// Output tokens allowed.
///
/// Roughly four times the summariser's, since a translation is as long as its input and the input
/// can be a paragraph. Bounded by the same reasoning as the time budget.
const MAX_TOKENS: usize = 512;

#[derive(Debug, Deserialize)]
pub struct TranslateRequest {
    /// The text. Never logged.
    pub text: String,
    /// Source language code, or absent for auto-detection.
    #[serde(default)]
    pub from: Option<String>,
    pub to: String,
}

#[derive(Debug, Serialize)]
pub struct LanguageInfo {
    pub code: &'static str,
    pub name_ar: &'static str,
    pub name_fr: &'static str,
    pub name_en: &'static str,
    pub approximate: bool,
}

#[derive(Debug, Serialize)]
pub struct LanguagesResponse {
    pub languages: Vec<LanguageInfo>,
}

/// The languages on offer, so the client does not carry its own list.
pub async fn languages() -> Json<LanguagesResponse> {
    Json(LanguagesResponse {
        languages: xustive_ml::translate::LANGUAGES
            .iter()
            .map(|l| LanguageInfo {
                code: l.code,
                name_ar: l.name_ar,
                name_fr: l.name_fr,
                name_en: l.name_en,
                approximate: l.approximate,
            })
            .collect(),
    })
}

/// One frame of the stream.
///
/// A tagged enum rather than bare text, because the client has to distinguish "here is more
/// output" from "this ended, and here is why". A stream that simply stops is indistinguishable
/// from a dropped connection, and the two need different behaviour on screen.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Frame<'a> {
    Delta {
        text: &'a str,
    },
    /// Terminal. `truncated` says the output hit a limit rather than finishing, which the card
    /// shows — a translation cut off mid-sentence with no marker reads as a bad translation.
    Done {
        truncated: bool,
        took_ms: u64,
    },
    Error {
        reason: &'static str,
    },
}

fn frame(f: &Frame<'_>) -> Event {
    // Serialisation of a small fixed enum cannot fail; if it somehow did, an empty error frame is
    // still a well-formed terminal event.
    Event::default().data(
        serde_json::to_string(f)
            .unwrap_or_else(|_| r#"{"type":"error","reason":"internal"}"#.to_string()),
    )
}

/// Counts a run that ended without a terminal frame.
///
/// The `Cancelled` arm below can never fire for a client disconnect, which is the case it looks
/// like it covers: a disconnect drops the stream, dropping the stream drops the receiver, and the
/// arm that would record it is inside the stream that no longer exists. Verified by cancelling a
/// request and watching the counter stay at zero.
///
/// So cancellation is recorded on `Drop` instead — the one thing that definitely still runs.
/// Disarmed when a terminal frame is emitted, so a normal finish does not count twice.
struct RunGuard {
    metrics: crate::metrics::Metrics,
    finished: bool,
}

impl RunGuard {
    fn finish(&mut self) {
        self.finished = true;
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        if !self.finished {
            // A high rate here means translations are too slow to wait for, not that people
            // changed their minds — which is why it is worth a counter at all.
            self.metrics.incr(
                TRANSLATE_TOTAL,
                TRANSLATE_TOTAL_HELP,
                &[("outcome", "cancelled")],
            );
        }
    }
}

#[cfg(feature = "summariser")]
pub async fn handler(
    State(state): State<AppState>,
    Json(req): Json<TranslateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    use async_stream::stream;
    use xustive_ml::engine::{Chunk, EngineError, Sampling};

    let started = Instant::now();

    let Some(target) = xustive_ml::translate::language(&req.to) else {
        return Err(ApiError::untranslatable("unknown_language"));
    };
    let source = req
        .from
        .as_deref()
        .and_then(xustive_ml::translate::language);

    let Some(prompt) = xustive_ml::translate::build(&req.text, source, target) else {
        // Covers empty input, input over the length limit, and same-language pairs. Not
        // distinguished in the response: the client already knows which it sent, and the length
        // limit is published.
        return Err(ApiError::untranslatable("untranslatable"));
    };

    let Some(engine) = state.summariser() else {
        return Err(ApiError::model_unavailable("model_not_loaded"));
    };

    // Near-greedy, and **no repetition penalty at all**.
    //
    // A repetition penalty is wrong for a translator, not merely unnecessary. Translation output
    // legitimately repeats — the same word twice in a sentence, a term echoed from earlier — and
    // penalising that pushes the model off the correct word onto whatever ranks next. Set to 1.1
    // it turned "good morning my friend" into "صباحك Okم يا друг": صباح had just been emitted, so
    // the penalty demoted it and the model reached across languages for a replacement.
    //
    // Temperature is low for the same reason. This is transcription between languages, not
    // writing, and there is no version of this task where variety is a virtue.
    let sampling = Sampling {
        max_tokens: MAX_TOKENS,
        ..Sampling::default()
    };

    let mut rx = match engine.generate_stream(prompt, sampling, BUDGET) {
        Ok(rx) => rx,
        Err(EngineError::Busy) => return Err(ApiError::model_unavailable("busy")),
        Err(_) => return Err(ApiError::model_unavailable("model_unavailable")),
    };

    let metrics = state.metrics.clone();
    let stream = stream! {
        let mut guard = RunGuard { metrics: metrics.clone(), finished: false };
        // Accumulated so the terminal frame can carry cleaned text. The per-token deltas are raw:
        // a token boundary can fall inside a multi-byte character or inside a wrapper the model
        // added, so cleaning a fragment in isolation would corrupt it.
        let mut produced = 0usize;

        while let Some(chunk) = rx.recv().await {
            match chunk {
                Chunk::Token(text) => {
                    produced += 1;
                    // Strip any character from a script no target language uses (a stray CJK/kana
                    // token the quantised model substitutes). A token that was *only* such
                    // characters becomes empty and is dropped rather than sent as a blank delta.
                    let cleaned = xustive_ml::translate::strip_foreign_scripts(&text);
                    if !cleaned.is_empty() {
                        yield Ok::<Event, Infallible>(frame(&Frame::Delta { text: cleaned.as_ref() }));
                    }
                }
                Chunk::Done(generated) => {
                    metrics.incr(
                        TRANSLATE_TOTAL,
                        TRANSLATE_TOTAL_HELP,
                        &[("outcome", if generated.truncated { "truncated" } else { "ok" })],
                    );
                    guard.finish();
                    yield Ok(frame(&Frame::Done {
                        truncated: generated.truncated,
                        took_ms: started.elapsed().as_millis() as u64,
                    }));
                    return;
                }
                Chunk::Failed(EngineError::Cancelled) => {
                    // Reachable only when the engine cancels for a reason other than a client
                    // disconnect, since a disconnect destroys this stream before the message can
                    // arrive. The guard counts that case on drop.
                    return;
                }
                Chunk::Failed(_) => {
                    guard.finish();
                    metrics.incr(TRANSLATE_TOTAL, TRANSLATE_TOTAL_HELP, &[("outcome", "failed")]);
                    yield Ok(frame(&Frame::Error { reason: "generation_failed" }));
                    return;
                }
            }
        }

        // The channel closed without a terminal chunk — a worker died. Say so rather than ending
        // the stream silently, which the client cannot tell from a dropped connection.
        let _ = produced;
        guard.finish();
        metrics.incr(TRANSLATE_TOTAL, TRANSLATE_TOTAL_HELP, &[("outcome", "interrupted")]);
        yield Ok(frame(&Frame::Error { reason: "interrupted" }));
    };

    Ok(sse(stream))
}

#[cfg(not(feature = "summariser"))]
pub async fn handler(
    State(_state): State<AppState>,
    Json(_req): Json<TranslateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    Err::<axum::response::Response, _>(ApiError::model_unavailable("not_built"))
}

#[allow(dead_code)]
fn sse<S>(stream: S) -> impl IntoResponse
where
    S: Stream<Item = Result<Event, Infallible>> + Send + 'static,
{
    // A comment line every 15 seconds. Without it an intermediary with a read timeout closes a
    // stream that is working — prefill on CPU can take longer than a default proxy timeout before
    // the first token exists.
    Sse::new(stream).keep_alive(KeepAlive::default().interval(Duration::from_secs(15)))
}

pub const TRANSLATE_TOTAL: &str = "xustive_translate_total";
pub const TRANSLATE_TOTAL_HELP: &str = "Translations by outcome";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_tagged_so_an_end_is_distinguishable_from_a_disconnect() {
        // A stream that simply stops looks identical to a dropped connection, and the two need
        // different behaviour on screen.
        let done = serde_json::to_string(&Frame::Done {
            truncated: false,
            took_ms: 12,
        })
        .unwrap();
        assert!(done.contains(r#""type":"done""#));
        assert!(done.contains(r#""truncated":false"#));

        let delta = serde_json::to_string(&Frame::Delta { text: "مرحبا" }).unwrap();
        assert!(delta.contains(r#""type":"delta""#));

        let error = serde_json::to_string(&Frame::Error { reason: "busy" }).unwrap();
        assert!(error.contains(r#""type":"error""#));
    }

    #[test]
    fn a_frame_never_contains_a_raw_newline() {
        // SSE terminates a message on a blank line, so a newline inside the payload would split
        // one frame into two malformed ones. JSON escaping is what prevents it — asserted because
        // translated text contains newlines routinely.
        let event = serde_json::to_string(&Frame::Delta {
            text: "line one\nline two",
        })
        .unwrap();
        assert!(
            !event.contains('\n'),
            "raw newline would break the frame: {event}"
        );
        assert!(event.contains("\\n"));
    }

    #[test]
    fn the_budget_is_generous_but_bounded() {
        // A translation is something the reader waits on deliberately, unlike a page they expect
        // instantly. Unbounded would let one prompt hold a model slot forever.
        assert!(BUDGET > Duration::from_secs(10));
        assert!(BUDGET <= Duration::from_secs(120));
        const { assert!(MAX_TOKENS >= 256) };
    }

    #[test]
    fn the_request_carries_text_in_a_body_not_a_url() {
        // The most sensitive field this service handles. A POST body keeps it out of URLs,
        // referrers and access logs; a query parameter would put it in all three.
        let json = r#"{"text":"bonjour","to":"ar"}"#;
        let req: TranslateRequest = serde_json::from_str(json).expect("parses");
        assert_eq!(req.text, "bonjour");
        assert_eq!(req.to, "ar");
        assert!(req.from.is_none(), "source language is optional");
    }
}
