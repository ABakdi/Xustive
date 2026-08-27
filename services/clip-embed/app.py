"""CLIP sidecar: image bytes → a 512-d vector, and — since M10 — words about the image.

The embedding half of image similarity ([[Vector Index]], M3-T05). Both planes use it: the
ingestion side embeds every crawled image so it is findable, and the query side embeds an
uploaded photo to search. Kept a separate service so the model and its ML runtime stay out of
the Rust build; the Rust client is `xustive_vector::embed::SidecarEmbedder`.

CLIP ViT-B/32 is ~150M parameters and runs fine on CPU, so this service has a real CPU path and
image similarity is **not** gated on a GPU. It uses the GPU when one is present.

# Wire contract

    POST /embed                body = raw image bytes           -> {"embedding": [512 floats]}
    POST /embed?describe=1     same                             -> + "styles": {id: cosine},
                                                                    "subjects": [{id, score}] top-k
    POST /embed/text           {"texts": [...]}                 -> {"vectors": [[512 floats], ...]}
    POST /classify             {"vectors": [[...], ...]}        -> {"items": [{styles, subjects}]}
    GET  /health                                                -> 200 when the model is loaded

# Describing an image (M10, ADR-0028)

Reverse image search wants words for two reasons: the web leg sends *labels* to the metasearch
engine rather than the reader's picture, and the page offers chips — "photo", "screenshot",
"illustration" — that are whatever the results actually are. Both come from CLIP's text tower:
the vocabularies in `data/styles.tsv` and `data/subjects.tsv` are embedded once at start, and an
image is scored against them by cosine. `/classify` does the same for vectors already stored, so
the points in Qdrant can be labelled without fetching a single image again.

# Privacy

The image is embedded in memory and never written to disk; only that a request succeeded and its
latency is logged — never the content, the vector, or the words.
"""

from __future__ import annotations

import io
import os
import time
from contextlib import asynccontextmanager
from pathlib import Path

from fastapi import FastAPI, Request, Response
from fastapi.responses import JSONResponse

MODEL_NAME = os.environ.get("CLIP_MODEL", "openai/clip-vit-base-patch32")
MAX_BYTES = int(os.environ.get("CLIP_MAX_BYTES", str(8 * 1024 * 1024)))
# The vocabularies. Defaults resolve relative to the repo (services/clip-embed → ../../data).
DATA_DIR = Path(os.environ.get("CLIP_VOCAB_DIR", str(Path(__file__).resolve().parents[2] / "data")))
SUBJECTS_TOP_K = int(os.environ.get("CLIP_SUBJECTS_TOP_K", "5"))
# A vector is bounded at 512 items and 512 floats each — /classify is for batches, not for a
# whole collection in one request.
MAX_CLASSIFY = 512
MAX_TEXTS = 256

_state: dict = {
    "model": None,
    "processor": None,
    "device": "cpu",
    # id -> row, and the matrix of prompt vectors in the same order (torch tensors).
    "styles": [],
    "subjects": [],
    "style_matrix": None,
    "subject_matrix": None,
}


def _load_vocab(path: Path) -> list[dict]:
    """A TSV of `id, prompt, ar, ary, fr, en`. A bad row fails loudly: a vocabulary that loads
    half of itself would mislabel every image in the index until someone noticed."""
    rows = []
    with path.open(encoding="utf-8") as fh:
        for n, line in enumerate(fh, 1):
            line = line.rstrip("\n")
            if not line or line.startswith("#"):
                continue
            parts = line.split("\t")
            if len(parts) < 2 or not parts[0] or not parts[1]:
                raise ValueError(f"{path.name}:{n}: expected at least `id<TAB>prompt`")
            rows.append({"id": parts[0], "prompt": parts[1]})
    if not rows:
        raise ValueError(f"{path.name}: no rows")
    return rows


@asynccontextmanager
async def lifespan(_app: FastAPI):
    import torch
    from transformers import CLIPModel, CLIPProcessor

    device = "cuda" if torch.cuda.is_available() else "cpu"
    model = CLIPModel.from_pretrained(MODEL_NAME).to(device).eval()
    processor = CLIPProcessor.from_pretrained(MODEL_NAME)
    _state.update(model=model, processor=processor, device=device)

    styles = _load_vocab(DATA_DIR / "styles.tsv")
    subjects = _load_vocab(DATA_DIR / "subjects.tsv")
    _state.update(
        styles=styles,
        subjects=subjects,
        style_matrix=_embed_texts([r["prompt"] for r in styles]),
        subject_matrix=_embed_texts([r["prompt"] for r in subjects]),
    )
    print(
        f"clip-embed ready model={MODEL_NAME} device={device} "
        f"styles={len(styles)} subjects={len(subjects)}",
        flush=True,
    )
    yield
    _state.update(model=None, processor=None, style_matrix=None, subject_matrix=None)


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
    describe = request.query_params.get("describe") in ("1", "true")

    started = time.monotonic()
    try:
        features = _embed_image(body)
    except Exception as exc:  # noqa: BLE001 — a bad image is a 422, everything else a 500
        name = type(exc).__name__
        print(f"clip-embed error: {name}", flush=True)
        if name in ("UnidentifiedImageError", "OSError"):
            return JSONResponse({"error": "undecodable image"}, status_code=422)
        return JSONResponse({"error": "embed failed"}, status_code=500)

    out = {"embedding": features[0].cpu().tolist()}
    if describe:
        out.update(_describe(features)[0])
    elapsed_ms = int((time.monotonic() - started) * 1000)
    print(
        f"clip-embed ok bytes={len(body)} dim={len(out['embedding'])} describe={describe} ms={elapsed_ms}",
        flush=True,
    )
    return JSONResponse(out)


@app.post("/embed/text")
async def embed_text(request: Request) -> Response:
    """The text tower. Also what text-to-image ranking on the Images tab will use."""
    if _state["model"] is None:
        return JSONResponse({"error": "model not loaded"}, status_code=503)
    payload = await request.json()
    texts = payload.get("texts") if isinstance(payload, dict) else None
    if not isinstance(texts, list) or not texts or not all(isinstance(t, str) and t.strip() for t in texts):
        return JSONResponse({"error": "texts must be a non-empty list of strings"}, status_code=422)
    if len(texts) > MAX_TEXTS:
        return JSONResponse({"error": "too many texts"}, status_code=413)
    started = time.monotonic()
    vectors = _embed_texts(texts).cpu().tolist()
    print(f"clip-embed text ok n={len(texts)} ms={int((time.monotonic() - started) * 1000)}", flush=True)
    return JSONResponse({"vectors": vectors})


@app.post("/classify")
async def classify(request: Request) -> Response:
    """Words for vectors already computed — the backfill path (M10-T02.4)."""
    if _state["model"] is None:
        return JSONResponse({"error": "model not loaded"}, status_code=503)
    import torch

    payload = await request.json()
    vectors = payload.get("vectors") if isinstance(payload, dict) else None
    if not isinstance(vectors, list) or not vectors:
        return JSONResponse({"error": "vectors must be a non-empty list"}, status_code=422)
    if len(vectors) > MAX_CLASSIFY:
        return JSONResponse({"error": "too many vectors"}, status_code=413)
    try:
        t = torch.tensor(vectors, dtype=torch.float32, device=_state["device"])
    except (TypeError, ValueError):
        return JSONResponse({"error": "vectors must be lists of floats"}, status_code=422)
    if t.ndim != 2 or t.shape[1] != _state["style_matrix"].shape[1]:
        return JSONResponse({"error": "wrong vector dimension"}, status_code=422)
    t = t / t.norm(p=2, dim=-1, keepdim=True).clamp_min(1e-8)
    return JSONResponse({"items": _describe(t)})


def _embed_image(data: bytes):
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
    return features / features.norm(p=2, dim=-1, keepdim=True)


def _embed_texts(texts: list[str]):
    import torch

    model = _state["model"]
    processor = _state["processor"]
    device = _state["device"]
    inputs = processor(text=texts, return_tensors="pt", padding=True, truncation=True).to(device)
    with torch.no_grad():
        features = model.get_text_features(**inputs)
    return features / features.norm(p=2, dim=-1, keepdim=True)


def _describe(features) -> list[dict]:
    """Cosine of each image vector against the style and subject prompts.

    Styles come back whole — the caller decides what margin makes a label — and subjects as the
    top-k, because twenty-two subject scores per image is noise on the wire and the web leg only
    wants the few that are clearly there.
    """
    styles = (features @ _state["style_matrix"].T).cpu()
    subjects = (features @ _state["subject_matrix"].T).cpu()
    out = []
    for i in range(features.shape[0]):
        style_scores = {r["id"]: round(float(styles[i, j]), 4) for j, r in enumerate(_state["styles"])}
        top = sorted(
            ((r["id"], round(float(subjects[i, j]), 4)) for j, r in enumerate(_state["subjects"])),
            key=lambda x: -x[1],
        )[:SUBJECTS_TOP_K]
        out.append({"styles": style_scores, "subjects": [{"id": k, "score": v} for k, v in top]})
    return out
