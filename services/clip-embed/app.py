"""CLIP embed sidecar: image bytes → a 512-d CLIP ViT-B/32 vector.

The embedding half of image similarity ([[Vector Index]], M3-T05). Both planes use it: the ingestion
side embeds every crawled image so it is findable, and the query side embeds an uploaded photo to
search. Kept a separate service so the model and its ML runtime stay out of the Rust build; the
Rust client is `xustive_vector::embed::SidecarEmbedder`.

Unlike the OCR sidecar, CLIP ViT-B/32 is ~150M parameters and runs fine on CPU, so this service has
a real CPU path and image similarity is **not** gated on a GPU. It uses the GPU when one is present.

# Wire contract

    POST /embed   body = raw image bytes (Content-Type: image/*)   ->  {"embedding": [512 floats]}
    GET  /health                                                    ->  200 when the model is loaded

The vector is L2-normalised again on the Rust side, so whether it is normalised here does not matter
for correctness — it is, anyway, because a normalised vector is what the index stores.

# Privacy

The image is embedded in memory and never written to disk; only that a request succeeded and its
latency is logged, never the content or the vector.
"""

from __future__ import annotations

import io
import os
import time
from contextlib import asynccontextmanager

from fastapi import FastAPI, Request, Response
from fastapi.responses import JSONResponse

MODEL_NAME = os.environ.get("CLIP_MODEL", "openai/clip-vit-base-patch32")
MAX_BYTES = int(os.environ.get("CLIP_MAX_BYTES", str(8 * 1024 * 1024)))

_state: dict = {"model": None, "processor": None, "device": "cpu"}


@asynccontextmanager
async def lifespan(_app: FastAPI):
    import torch
    from transformers import CLIPModel, CLIPProcessor

    device = "cuda" if torch.cuda.is_available() else "cpu"
    model = CLIPModel.from_pretrained(MODEL_NAME).to(device).eval()
    processor = CLIPProcessor.from_pretrained(MODEL_NAME)
    _state.update(model=model, processor=processor, device=device)
    print(f"clip-embed ready model={MODEL_NAME} device={device}", flush=True)
    yield
    _state.update(model=None, processor=None)


app = FastAPI(lifespan=lifespan, title="Xustive CLIP embed sidecar")


@app.get("/health")
def health() -> Response:
    return Response(status_code=200 if _state["model"] is not None else 503)


@app.post("/embed")
async def embed(request: Request) -> Response:
    if _state["model"] is None:
        return JSONResponse({"error": "model not loaded"}, status_code=503)

    body = await request.body()
    if not body:
        return JSONResponse({"error": "empty body"}, status_code=422)
    if len(body) > MAX_BYTES:
        return JSONResponse({"error": "image too large"}, status_code=413)

    started = time.monotonic()
    try:
        vector = _embed_image(body)
    except Exception as exc:  # noqa: BLE001 — a bad image is a 422, everything else a 500
        name = type(exc).__name__
        print(f"clip-embed error: {name}", flush=True)
        if name in ("UnidentifiedImageError", "OSError"):
            return JSONResponse({"error": "undecodable image"}, status_code=422)
        return JSONResponse({"error": "embed failed"}, status_code=500)

    elapsed_ms = int((time.monotonic() - started) * 1000)
    print(f"clip-embed ok bytes={len(body)} dim={len(vector)} ms={elapsed_ms}", flush=True)
    return JSONResponse({"embedding": vector})


def _embed_image(data: bytes) -> list[float]:
    import torch
    from PIL import Image

    model = _state["model"]
    processor = _state["processor"]
    device = _state["device"]

    image = Image.open(io.BytesIO(data)).convert("RGB")
    inputs = processor(images=image, return_tensors="pt").to(device)
    with torch.no_grad():
        features = model.get_image_features(**inputs)
    # L2-normalise so the stored/queried vectors are unit vectors and cosine is a dot product.
    features = features / features.norm(p=2, dim=-1, keepdim=True)
    return features[0].cpu().tolist()
