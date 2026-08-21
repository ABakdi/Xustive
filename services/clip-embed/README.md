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
POST /embed   body = raw image bytes (Content-Type: image/*)   ->  {"embedding": [512 floats]}
GET  /health                                                    ->  200 when the model is loaded, else 503
```

The Rust client is `xustive_vector::embed::SidecarEmbedder`. It re-normalises the vector, so the
service's own normalisation is belt-and-braces.

## Run it

```bash
pip install -r requirements.txt
uvicorn app:app --host 0.0.0.0 --port 8092
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
