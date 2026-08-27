# CLIP embed sidecar

The embedding half of image similarity ([[Vector Index]], M3-T05). It turns image bytes into a
**512-d CLIP ViT-B/32** vector; [`xustive-vector`](../../crates/xustive-vector) stores those in
Qdrant and searches them. Both planes use this one endpoint — the crawler embeds every image so it
becomes findable, and the search box embeds an uploaded photo to find visually similar posts.

**CPU-capable.** CLIP ViT-B/32 is ~150M parameters and embeds an image in a fraction of a second on
CPU, so — unlike the [OCR sidecar](../ocr-sidecar/README.md) — image similarity is **not** gated on a
GPU. It uses a GPU automatically when one is present (`torch.cuda.is_available()`), and runs on the
CPU-only reference machine otherwise.

## Wire contract

```
POST /embed                body = raw image bytes (Content-Type: image/*)
                           ->  {"embedding": [512 floats]}
POST /embed?describe=1     ->  + "styles": {id: cosine}, "subjects": [{id, score}] (top 5)
POST /embed/text           {"texts": [...]}       ->  {"vectors": [[512 floats], ...]}
POST /classify             {"vectors": [[...]]}   ->  {"items": [{"styles": {...}, "subjects": [...]}]}
GET  /health               ->  200 when the model is loaded, else 503
```

`describe`, `/embed/text` and `/classify` arrived with [[Milestone 10 - Reverse Image Search]]
(ADR-0028): the text tower scores an image against two reviewable vocabularies —
`data/styles.tsv` (photo, illustration, 3D render, screenshot…) and `data/subjects.tsv` (mosque,
Casbah, desert, football, couscous…; Algeria first) — so the reverse search can name what a
picture is and send *words* to the web rather than the picture. `/classify` labels vectors already
stored, which is how the existing index gets its styles without re-fetching an image
(`xustive vector-repass --describe`). `CLIP_VOCAB_DIR` overrides where the TSVs are read from;
a malformed row fails start-up on purpose.

Scores are raw cosines (~0.18–0.30 for CLIP ViT-B/32). The Rust side turns styles into a label
with a softmax over `100 × cosine` (CLIP's own logit scale) and keeps the top style only when
its probability is ≥ 0.5 — measured: a screenshot 0.95, photographs 0.70–0.83.

## Run it

```bash
uv venv -p 3.12 .venv && uv pip install -p .venv/bin/python --index-url https://download.pytorch.org/whl/cpu torch==2.10.0
uv pip install -p .venv/bin/python -r requirements.txt
.venv/bin/uvicorn app:app --host 127.0.0.1 --port 8092      # ~600 MB model fetched on first start
```

Docker (weights mounted, not baked in):

```bash
docker build -t xustive-clip-embed .
docker run -v /path/to/hf-cache:/models/hf xustive-clip-embed          # CPU
docker run --gpus all -v /path/to/hf-cache:/models/hf xustive-clip-embed # GPU
```

Then turn on image similarity in `config/*.toml`:

```toml
[vector]
enabled = true
qdrant_url = "http://qdrant:6333"
embedder_endpoint = "http://clip-embed:8092/embed"
```

It is wired into `deploy/docker-compose.yml` as the `clip-embed` service behind the `vector` profile,
on the internal `core` network with a `mem_limit` and no published port. `core` has no egress, so the
weights are provisioned into the `clip_models` volume out-of-band (`HF_HUB_OFFLINE=1`).

## Privacy

The image is embedded in memory and never written to disk. Only that a request succeeded and its
latency is logged — never the image or the vector.
