# Unlimited-OCR sidecar

The optional, high-quality OCR backend for Xustive. It wraps [`baidu/Unlimited-OCR`][model] — a
3B-parameter vision-language model (MIT licence) that parses layout, tables and multi-page documents
in one shot — behind a tiny HTTP contract the Rust engine talks to.

This service is **opt-in and GPU-only**. It exists alongside, not instead of, the always-on
tesseract engine that runs in-process in `xustive-media`:

| | tesseract (default) | Unlimited-OCR (this sidecar) |
|---|---|---|
| Where | in-process, Rust | separate Python/GPU service |
| Hardware | CPU, or a 4 GB GPU | a real GPU (see below) |
| Used by | crawl-time enrichment **and** the tools | the tools, when selected |
| Strength | fast, always available | layout, tables, multi-page, handwriting |

The crawl-time path — OCR over every crawled image — **always** uses tesseract regardless of this
service, because it must fit the CPU-only reference hardware. This sidecar only ever serves the
user-facing tools (the standalone OCR tool and the "photograph → search" flow), and only when
`media.ocr_backend = "unlimited"`.

## Hardware

The model is 3B parameters in bf16 ≈ **6 GB of weights**, plus image-activation memory. It does
**not** fit the reference Quadro T1000 (4 GB) — that is exactly why it is not the default. Run it on
a GPU with **≥ 8 GB** of VRAM. There is no CPU mode here; on CPU a 3B VLM is minutes per image.

## Wire contract

```
POST /ocr    body = raw image bytes (Content-Type: image/*)   ->  {"text": "...", "confidence"?: n}
GET  /health                                                   ->  200 when the model is loaded, else 503
```

The Rust client is `xustive-media::backend::Sidecar`. It sends the image as a raw POST body and
reads back `text`. If this service is unreachable or errors, the Rust side **falls back to
tesseract** — selecting `"unlimited"` never turns OCR off.

## Run it

Bare metal (a box with a CUDA GPU and the model downloaded):

```bash
pip install -r requirements.txt
uvicorn app:app --host 0.0.0.0 --port 8091
```

Docker (weights mounted, not baked in):

```bash
docker build -t xustive-ocr-sidecar .
docker run --gpus all -v /path/to/hf-cache:/models/hf xustive-ocr-sidecar
```

Then point the engine at it in `config/*.toml`:

```toml
[media]
ocr_backend = "unlimited"

[media.sidecar]
endpoint   = "http://ocr-sidecar:8091/ocr"   # service name on the private network
timeout_ms = 30000
```

Wire it into `deploy/docker-compose.yml` as a service on the internal `core` network with a
`mem_limit`, a GPU reservation (`deploy.resources.reservations.devices`), **no published port**, and
`profiles: ["ocr"]` so it stays off on GPU-less hosts. It is kept out of the base compose here on
purpose: the base topology must come up on CPU-only hardware.

## Privacy

The uploaded image never persists. Each request writes the bytes to a private temp file (the model's
`infer()` API takes a path), runs one inference, and deletes the file in a `finally`. Nothing about
the request content is logged — only that a request of some size succeeded and its latency. This
mirrors the in-process engine's zero-disk-write posture ([[Security and Privacy]] P4); here it is
enforced by cleanup, because the model API leaves no path-free option.

## The one coupling point

`_run_infer()` in `app.py` is the only code tied to the model's exact API. It handles both shapes of
the model card's `infer()` (returns text, or writes it under `output_path`). A future model version
that changes the signature is a change to that one function.

[model]: https://huggingface.co/baidu/Unlimited-OCR
