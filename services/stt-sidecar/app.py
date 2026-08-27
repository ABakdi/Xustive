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
# The model for live partials — a reading of the words so far every half-second while the person
# is still speaking. Whisper encodes a fixed thirty-second window, so a partial costs a whole
# encoder pass whatever the clip's length; `base` does that in a fraction of `small`'s time and is
# decoded greedily. The final pass, once they stop, still gets `small` with a beam. Unset, the
# main model answers partials too — correct, just not live.
PARTIAL_MODEL_NAME = os.environ.get("STT_PARTIAL_MODEL", "") or None
# CTranslate2 defaults to a handful of threads; the encoder scales to the cores it is given.
CPU_THREADS = int(os.environ.get("STT_CPU_THREADS", str(min(8, os.cpu_count() or 4))))
# Audio is small; a generous ceiling still bounds a runaway upload. 30 s of Opus is well under 1 MB.
MAX_BYTES = int(os.environ.get("STT_MAX_BYTES", str(16 * 1024 * 1024)))
# Bias detection toward the languages Algerian audio actually is, unless the caller forces one.
DEFAULT_LANG = os.environ.get("STT_DEFAULT_LANG", "") or None

_state: dict = {"model": None, "partial": None}


@asynccontextmanager
async def lifespan(_app: FastAPI):
    from faster_whisper import WhisperModel

    # int8 on CPU is the reference path; a GPU host can set STT_DEVICE=cuda / STT_COMPUTE=float16.
    device = os.environ.get("STT_DEVICE", "cpu")
    compute = os.environ.get("STT_COMPUTE", "int8")
    # The partial model may run at a different precision: on a 4 GB card the final model is
    # quantised to leave room, while the partial one — whose whole job is speed — stays float32.
    partial_compute = os.environ.get("STT_PARTIAL_COMPUTE", "") or compute
    _state["model"] = WhisperModel(MODEL_NAME, device=device, compute_type=compute, cpu_threads=CPU_THREADS)
    if PARTIAL_MODEL_NAME:
        _state["partial"] = WhisperModel(
            PARTIAL_MODEL_NAME, device=device, compute_type=partial_compute, cpu_threads=CPU_THREADS
        )
    print(
        f"stt ready model={MODEL_NAME}({compute}) partial={PARTIAL_MODEL_NAME or '-'}({partial_compute}) "
        f"device={device} threads={CPU_THREADS}",
        flush=True,
    )
    yield
    _state["model"] = None
    _state["partial"] = None


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
    partial = request.query_params.get("partial") in ("1", "true")
    started = time.monotonic()
    try:
        try:
            text, detected = _run(body, lang, partial)
        except RuntimeError as exc:
            # The GPU is shared — with the API's own models and the desktop — and the careful
            # model's beam search is the first thing to run out of room. The light model's
            # reading is a worse answer than the careful one and a far better one than a 500.
            if "out of memory" not in str(exc).lower() or partial or _state["partial"] is None:
                raise
            print("stt oom on final; answering with the partial model", flush=True)
            text, detected = _run(body, lang, True)
    except Exception as exc:  # noqa: BLE001 — a bad container is a 422, everything else a 500
        name = type(exc).__name__
        # The whole traceback, to the log only: the reply stays a bare 500. The first version
        # printed the name alone, and "RuntimeError" was not a diagnosis.
        import traceback

        print(f"stt error: {name}: {exc}", flush=True)
        traceback.print_exc()
        return JSONResponse({"error": "transcription failed"}, status_code=500)

    elapsed_ms = int((time.monotonic() - started) * 1000)
    print(
        f"stt ok {'partial' if partial else 'final'} bytes={len(body)} chars={len(text)} lang={detected} ms={elapsed_ms}",
        flush=True,
    )
    return JSONResponse({"text": text, "language": detected})


# Whisper hallucinates confident-sounding phrases on silence ("thank you", "subscribe"). Its own
# per-segment signals catch most of it: a high no-speech probability with a low average token
# log-probability means "the model was guessing". Segments past these thresholds are dropped. The
# Rust side applies a second, phrase-based filter as defence in depth (M3-T02.6).
NO_SPEECH_MAX = 0.6
AVG_LOGPROB_MIN = -1.0


def _run(data: bytes, lang: str | None, partial: bool = False) -> tuple[str, str]:
    model = (_state["partial"] if partial else None) or _state["model"]
    # faster-whisper decodes via PyAV from a file-like object — no temp file, so nothing persists.
    # A partial is greedy and timestamp-free: it is replaced half a second later, so its job is
    # to be fast, and the words it gets wrong the final pass gets right.
    segments, info = model.transcribe(
        io.BytesIO(data),
        language=lang,
        vad_filter=True,  # trim leading/trailing silence so a near-silent clip returns little
        beam_size=1 if partial else 5,
        best_of=1 if partial else 5,
        without_timestamps=partial,
        condition_on_previous_text=not partial,
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
