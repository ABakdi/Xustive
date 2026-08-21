"""Unlimited-OCR sidecar: the optional high-quality OCR backend.

This is the *opt-in* half of Xustive's two-engine OCR design. The always-on engine is tesseract,
in-process in the Rust `xustive-media` crate, which fits the CPU-only reference hardware and reads
every crawled image. This service is what `media.ocr_backend = "unlimited"` points at: Baidu's
Unlimited-OCR, a 3B-parameter vision-language model (MIT) that parses layout, tables and multi-page
documents far better than glyph OCR — but needs a real GPU and a Python runtime, so it runs here, in
its own process, on the private network.

# Wire contract (kept deliberately tiny — see crates/xustive-media/src/backend.rs::Sidecar)

    POST /ocr    body = raw image bytes (Content-Type: image/*)   -> {"text": "...", "confidence"?: n}
    GET  /health                                                   -> 200 when the model is loaded

The Rust client sends the image as a raw body and reads back `text`; a VLM has no per-word
confidence, so `confidence` is omitted and the caller assumes a high value (the length floor in
`ocr::score` still rejects an empty answer).

# Privacy

The image never persists. The model's documented `infer()` API takes a *file path*, so each request
writes the bytes to a private temp file and deletes it in a `finally` — the file exists only for the
duration of one inference and never under a predictable name. Nothing about the request is logged
beyond timing and success. This mirrors the zero-disk-write posture the in-process engine holds by
construction ([[Security and Privacy]] P4); here it is held by cleanup instead, because the model
API leaves no other option.

# Running

    pip install -r requirements.txt
    uvicorn app:app --host 0.0.0.0 --port 8091   # needs a CUDA GPU with the model downloaded

See README.md for hardware notes. If this service is down, the Rust side silently falls back to
tesseract — selecting "unlimited" never turns OCR off.
"""

from __future__ import annotations

import os
import tempfile
import time
from contextlib import asynccontextmanager

from fastapi import FastAPI, Request, Response
from fastapi.responses import JSONResponse

MODEL_NAME = os.environ.get("OCR_MODEL", "baidu/Unlimited-OCR")
# The prompt template from the model card. "document parsing." is the mode that reads a whole page,
# which is exactly the screenshot/document case Xustive cares about.
PROMPT = os.environ.get("OCR_PROMPT", "<image>document parsing.")
# A hard ceiling mirroring the Rust side's max_image_bytes, so a giant upload is refused cheaply.
MAX_BYTES = int(os.environ.get("OCR_MAX_BYTES", str(8 * 1024 * 1024)))

# Populated at startup by the lifespan handler. Kept in a dict so the handlers can see the loaded
# objects without a module-level global rebind.
_state: dict = {"model": None, "tokenizer": None}


@asynccontextmanager
async def lifespan(_app: FastAPI):
    # Imported inside the lifespan so the module can be imported (for tests, for --help) on a box
    # without torch installed; the heavy import only happens when the service actually starts.
    import torch
    from transformers import AutoModel, AutoTokenizer

    tokenizer = AutoTokenizer.from_pretrained(MODEL_NAME, trust_remote_code=True)
    model = AutoModel.from_pretrained(
        MODEL_NAME,
        trust_remote_code=True,
        use_safetensors=True,
        torch_dtype=torch.bfloat16,
    )
    model = model.eval().cuda()
    _state["model"] = model
    _state["tokenizer"] = tokenizer
    yield
    _state["model"] = None
    _state["tokenizer"] = None


app = FastAPI(lifespan=lifespan, title="Xustive Unlimited-OCR sidecar")


@app.get("/health")
def health() -> Response:
    """Liveness: 200 once the model is loaded, 503 while it is still loading.

    The Rust side probes this before showing the sidecar as available in the admin console, and the
    container orchestrator uses it as a readiness gate — a model that takes tens of seconds to load
    must not receive traffic before it can answer.
    """
    if _state["model"] is None:
        return Response(status_code=503)
    return Response(status_code=200)


@app.post("/ocr")
async def ocr(request: Request) -> Response:
    if _state["model"] is None:
        return JSONResponse({"error": "model not loaded"}, status_code=503)

    body = await request.body()
    if not body:
        return JSONResponse({"error": "empty body"}, status_code=422)
    if len(body) > MAX_BYTES:
        return JSONResponse({"error": "image too large"}, status_code=413)

    started = time.monotonic()
    # A private temp file the model can read by path, and an output dir it can write to. Both live
    # only for this request; the finally removes them whatever happens.
    tmp_img = tempfile.NamedTemporaryFile(suffix=".img", delete=False)
    out_dir = tempfile.mkdtemp(prefix="ocr-out-")
    try:
        tmp_img.write(body)
        tmp_img.flush()
        tmp_img.close()

        text = _run_infer(tmp_img.name, out_dir)
        elapsed_ms = int((time.monotonic() - started) * 1000)
        # No text, no image dimensions, nothing about the content is logged — only that a request of
        # some size succeeded and how long it took.
        print(f"ocr ok bytes={len(body)} chars={len(text)} ms={elapsed_ms}", flush=True)
        return JSONResponse({"text": text})
    except Exception as exc:  # noqa: BLE001 — any failure is a 500 the Rust side falls back from
        print(f"ocr error: {type(exc).__name__}", flush=True)
        return JSONResponse({"error": "inference failed"}, status_code=500)
    finally:
        _cleanup(tmp_img.name, out_dir)


def _run_infer(image_path: str, out_dir: str) -> str:
    """Run one inference and return the parsed text.

    This is the single integration point that is coupled to the model's exact API. The model card's
    `infer()` either returns the parsed text or writes it under `output_path`; both are handled so a
    minor version difference does not silently return empty. If a future model version changes the
    signature, this is the one function to adjust.
    """
    model = _state["model"]
    tokenizer = _state["tokenizer"]
    result = model.infer(
        tokenizer,
        prompt=PROMPT,
        image_file=image_path,
        output_path=out_dir,
        base_size=1024,
        image_size=640,
        crop_mode=True,
        max_length=32768,
        no_repeat_ngram_size=35,
        ngram_window=128,
        save_results=True,
    )
    if isinstance(result, str) and result.strip():
        return result.strip()
    return _read_output(out_dir)


def _read_output(out_dir: str) -> str:
    """Concatenate whatever text files the model wrote, in a stable order."""
    parts: list[str] = []
    for name in sorted(os.listdir(out_dir)):
        if name.endswith((".txt", ".md", ".mmd")):
            with open(os.path.join(out_dir, name), encoding="utf-8", errors="replace") as fh:
                parts.append(fh.read())
    return "\n".join(p.strip() for p in parts if p.strip())


def _cleanup(image_path: str, out_dir: str) -> None:
    try:
        os.unlink(image_path)
    except OSError:
        pass
    try:
        for name in os.listdir(out_dir):
            try:
                os.unlink(os.path.join(out_dir, name))
            except OSError:
                pass
        os.rmdir(out_dir)
    except OSError:
        pass
