"""Text embed sidecar: a batch of strings → multilingual sentence embeddings.

The embedding half of semantic search (M7-T02). Both planes use it: the ingestion side embeds every
indexed document so it is findable by meaning, and the query side embeds the search query. Kept a
separate service so the model and its ML runtime stay out of the Rust build; the Rust client is
`xustive_vector::text::TextEmbedder`.

Default model is **BAAI/bge-m3** — multilingual (strong on Arabic, Darija and French), Apache-2.0,
and 1024-dimensional. It runs on CPU (the reference-hardware path) and uses the GPU when one is
present. Swap it via TEXT_EMBED_MODEL for a lighter CPU model (e.g. intfloat/multilingual-e5-small,
384-d) — keep the Rust `[vector] text_dim` in step with whatever you choose.

# Wire contract

    POST /embed   body = {"texts": ["...", ...]}   ->  {"vectors": [[N floats], ...]}
    GET  /health                                    ->  200 when the model is loaded

Vectors are L2-normalised here; the Rust side normalises again, so correctness does not depend on
it, but the stored/queried vectors are unit vectors and cosine is a dot product either way.

# Privacy

Text is embedded in memory and never written to disk; only that a request succeeded, the batch size,
and latency are logged — never the text or the vectors.
"""

from __future__ import annotations

import os
import time
from contextlib import asynccontextmanager

from fastapi import FastAPI, Request, Response
from fastapi.responses import JSONResponse

MODEL_NAME = os.environ.get("TEXT_EMBED_MODEL", "BAAI/bge-m3")
# Guardrails: a runaway batch or a giant document must not exhaust memory. Documents are truncated
# rather than rejected — the head of a page carries its topic, which is what a retrieval embedding
# needs, and refusing a long page would silently drop it from semantic search.
MAX_TEXTS = int(os.environ.get("TEXT_EMBED_MAX_TEXTS", "128"))
MAX_CHARS = int(os.environ.get("TEXT_EMBED_MAX_CHARS", "8192"))

_state: dict = {"model": None, "device": "cpu"}


@asynccontextmanager
async def lifespan(_app: FastAPI):
    import torch
    from sentence_transformers import SentenceTransformer

    device = "cuda" if torch.cuda.is_available() else "cpu"
    model = SentenceTransformer(MODEL_NAME, device=device)
    _state.update(model=model, device=device)
    dim = model.get_sentence_embedding_dimension()
    print(f"text-embed ready model={MODEL_NAME} device={device} dim={dim}", flush=True)
    yield
    _state.update(model=None)


app = FastAPI(lifespan=lifespan, title="Xustive text embed sidecar")


@app.get("/health")
def health() -> Response:
    return Response(status_code=200 if _state["model"] is not None else 503)


@app.post("/embed")
async def embed(request: Request) -> Response:
    if _state["model"] is None:
        return JSONResponse({"error": "model not loaded"}, status_code=503)

    try:
        payload = await request.json()
    except Exception:  # noqa: BLE001
        return JSONResponse({"error": "invalid json"}, status_code=422)

    texts = payload.get("texts") if isinstance(payload, dict) else None
    if not isinstance(texts, list) or not texts:
        return JSONResponse({"error": "expected {texts: [...]}"}, status_code=422)
    if len(texts) > MAX_TEXTS:
        return JSONResponse({"error": f"at most {MAX_TEXTS} texts per request"}, status_code=413)
    # Empty strings would embed to a meaningless point; a single space keeps the model happy and the
    # index position aligned with the caller's list.
    clean = [(t if isinstance(t, str) else "")[:MAX_CHARS] or " " for t in texts]

    started = time.monotonic()
    try:
        vectors = _embed_texts(clean)
    except Exception as exc:  # noqa: BLE001
        print(f"text-embed error: {type(exc).__name__}", flush=True)
        return JSONResponse({"error": "embed failed"}, status_code=500)

    elapsed_ms = int((time.monotonic() - started) * 1000)
    print(f"text-embed ok n={len(clean)} dim={len(vectors[0]) if vectors else 0} ms={elapsed_ms}", flush=True)
    return JSONResponse({"vectors": vectors})


def _embed_texts(texts: list[str]) -> list[list[float]]:
    model = _state["model"]
    # normalize_embeddings=True → unit vectors, so cosine similarity is a dot product downstream.
    embeddings = model.encode(
        texts,
        normalize_embeddings=True,
        convert_to_numpy=True,
        show_progress_bar=False,
    )
    return embeddings.tolist()
