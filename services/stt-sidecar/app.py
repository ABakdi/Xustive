"""Speech-to-text sidecar: audio bytes → a transcript.

Voice is a primary input for a large share of the audience — typing Arabic on a phone is slow
([[Milestone 3 - Multimodal Input]] §Why). This service is the transcription half; the browser
records, this turns the recording into text, and the text lands in the search box **editable and not
auto-submitted** (the frontend's job).

It follows the same pattern as the OCR and CLIP sidecars ([[ADR-0016 - Two OCR Engines with an
Optional Unlimited-OCR Sidecar]]): a tiny HTTP contract in front of a model kept out of the Rust
build. Whisper `small` on `faster-whisper` (CTranslate2) runs on CPU at int8, so voice is **not**
GPU-gated — it uses a GPU when present.

# Wire contract

    POST /transcribe   body = raw audio bytes (Content-Type: audio/*)
                       optional ?lang=ar to force a language
                       ->  {"text": "...", "language": "ar"}
    GET  /health                                                      ->  200 when the model is loaded

# Language

Arabic-preferred: when no language is forced, whisper detects it, but Algerian audio is mostly
Arabic/Darija/French. The frontend passes the UI language as a hint.

# Privacy

The audio is transcribed in memory and never written to disk; only that a request succeeded and its
latency is logged, never the audio or the transcript — the same posture as the OCR sidecar, and the
whole point of self-hosting transcription rather than sending voice to a cloud API.
"""

from __future__ import annotations

import io
import os
import time
from contextlib import asynccontextmanager

from fastapi import FastAPI, Request, Response
from fastapi.responses import JSONResponse

MODEL_NAME = os.environ.get("STT_MODEL", "small")
# Audio is small; a generous ceiling still bounds a runaway upload. 30 s of Opus is well under 1 MB.
MAX_BYTES = int(os.environ.get("STT_MAX_BYTES", str(16 * 1024 * 1024)))
# Bias detection toward the languages Algerian audio actually is, unless the caller forces one.
DEFAULT_LANG = os.environ.get("STT_DEFAULT_LANG", "") or None

_state: dict = {"model": None}


@asynccontextmanager
async def lifespan(_app: FastAPI):
    from faster_whisper import WhisperModel

    # int8 on CPU is the reference path; a GPU host can set STT_DEVICE=cuda / STT_COMPUTE=float16.
    device = os.environ.get("STT_DEVICE", "cpu")
    compute = os.environ.get("STT_COMPUTE", "int8")
    _state["model"] = WhisperModel(MODEL_NAME, device=device, compute_type=compute)
    print(f"stt ready model={MODEL_NAME} device={device} compute={compute}", flush=True)
    yield
    _state["model"] = None


app = FastAPI(lifespan=lifespan, title="Xustive STT sidecar")


@app.get("/health")
def health() -> Response:
    return Response(status_code=200 if _state["model"] is not None else 503)


@app.post("/transcribe")
async def transcribe(request: Request) -> Response:
    if _state["model"] is None:
        return JSONResponse({"error": "model not loaded"}, status_code=503)

    body = await request.body()
    if not body:
        return JSONResponse({"error": "empty body"}, status_code=422)
    if len(body) > MAX_BYTES:
        return JSONResponse({"error": "audio too large"}, status_code=413)

    lang = request.query_params.get("lang") or DEFAULT_LANG
    started = time.monotonic()
    try:
        text, detected = _run(body, lang)
    except Exception as exc:  # noqa: BLE001 — a bad container is a 422, everything else a 500
        name = type(exc).__name__
        print(f"stt error: {name}", flush=True)
        return JSONResponse({"error": "transcription failed"}, status_code=500)

    elapsed_ms = int((time.monotonic() - started) * 1000)
    print(f"stt ok bytes={len(body)} chars={len(text)} lang={detected} ms={elapsed_ms}", flush=True)
    return JSONResponse({"text": text, "language": detected})


# Whisper hallucinates confident-sounding phrases on silence ("thank you", "subscribe"). Its own
# per-segment signals catch most of it: a high no-speech probability with a low average token
# log-probability means "the model was guessing". Segments past these thresholds are dropped. The
# Rust side applies a second, phrase-based filter as defence in depth (M3-T02.6).
NO_SPEECH_MAX = 0.6
AVG_LOGPROB_MIN = -1.0


def _run(data: bytes, lang: str | None) -> tuple[str, str]:
    model = _state["model"]
    # faster-whisper decodes via PyAV from a file-like object — no temp file, so nothing persists.
    segments, info = model.transcribe(
        io.BytesIO(data),
        language=lang,
        vad_filter=True,  # trim leading/trailing silence so a near-silent clip returns little
        beam_size=5,
    )
    kept = []
    for seg in segments:
        no_speech = getattr(seg, "no_speech_prob", 0.0)
        avg_logprob = getattr(seg, "avg_logprob", 0.0)
        if no_speech > NO_SPEECH_MAX and avg_logprob < AVG_LOGPROB_MIN:
            continue  # a hallucinated segment on near-silence
        kept.append(seg.text.strip())
    text = " ".join(kept).strip()
    return text, getattr(info, "language", lang or "und")
