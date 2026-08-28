"""Reranker sidecar: a query and a short list of candidates → one relevance score each.

The cross-encoder of [[Milestone 13 - Distilled Ranking]] (ADR-0032). Stage 1 (Meilisearch) and
stage 2 (the bounded re-rank in `xustive-search::rank`) never read the query and the document
*together*; this does, for the top of the page only, and the Rust side fuses its order with
stage 2's by reciprocal rank — never substituting one for the other.

The model is Qwen3-Reranker-0.6B (Apache-2.0), INT8 ONNX through `qwen3-embed`, so there is no
PyTorch in this service and it runs on the CPU-only reference machine; ONNX Runtime uses a GPU
when one is present and the matching runtime is installed. Twenty pairs of a title and an
excerpt are a few hundred tokens each and score in well under a second on the CPU.

# Wire contract

    POST /rerank   {"query": "...", "documents": ["...", ...]}   -> {"scores": [float in [0,1], ...]}
    GET  /health                                                 -> 200 when the model is loaded

`documents` is capped at RERANK_MAX_DOCS (default 50) and each document at RERANK_MAX_CHARS
(default 1200) — the excerpt, not the body, is what the page shows and what is judged.

# Privacy

The query and the excerpts are first-party and stay in memory; only latency, status and the
number of pairs are logged — never the text (ADR-0029 allows raw queries to leave; this sidecar
does not even do that).
"""

from __future__ import annotations

import os
import time
from contextlib import asynccontextmanager

from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse

MODEL_NAME = os.environ.get("RERANK_MODEL", "n24q02m/Qwen3-Reranker-0.6B-ONNX")
MAX_DOCS = int(os.environ.get("RERANK_MAX_DOCS", "50"))
MAX_CHARS = int(os.environ.get("RERANK_MAX_CHARS", "1200"))
MAX_QUERY_CHARS = int(os.environ.get("RERANK_MAX_QUERY_CHARS", "300"))
# Where `qwen3-embed` caches the weights; defaults resolve into the repo's data/models so the
# download is shared with the other sidecars and ignored by git.
os.environ.setdefault(
    "HF_HOME",
    os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))), "data", "models", "hf"),
)

_model = None


def _load():
    global _model
    if _model is None:
        from qwen3_embed import TextCrossEncoder  # imported late: slow, and only needed to serve

        _model = TextCrossEncoder(model_name=MODEL_NAME)
    return _model


@asynccontextmanager
async def lifespan(_: FastAPI):
    started = time.monotonic()
    try:
        _load()
        print(f"reranker: model loaded in {time.monotonic() - started:.1f}s", flush=True)
    except Exception as e:  # noqa: BLE001 — the health check reports it; the process stays up
        print(f"reranker: model failed to load: {type(e).__name__}", flush=True)
    yield


app = FastAPI(lifespan=lifespan)


@app.get("/health")
async def health():
    if _model is None:
        return JSONResponse({"ok": False, "model": MODEL_NAME}, status_code=503)
    return {"ok": True, "model": MODEL_NAME}


@app.post("/rerank")
async def rerank(request: Request):
    started = time.monotonic()
    if _model is None:
        return JSONResponse({"error": "model not loaded"}, status_code=503)
    try:
        body = await request.json()
    except Exception:  # noqa: BLE001
        return JSONResponse({"error": "invalid json"}, status_code=400)
    query = str(body.get("query", ""))[:MAX_QUERY_CHARS].strip()
    docs = body.get("documents")
    if not query or not isinstance(docs, list) or not docs:
        return JSONResponse({"error": "query and documents required"}, status_code=400)
    docs = [str(d)[:MAX_CHARS] for d in docs[:MAX_DOCS]]
    try:
        scores = [float(s) for s in _model.rerank(query, docs)]
    except Exception as e:  # noqa: BLE001
        print(f"reranker: scoring failed: {type(e).__name__}", flush=True)
        return JSONResponse({"error": "scoring failed"}, status_code=500)
    print(f"reranker: {len(docs)} pairs in {(time.monotonic() - started) * 1000:.0f}ms", flush=True)
    return {"scores": scores}
