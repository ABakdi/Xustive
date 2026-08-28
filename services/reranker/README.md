# Reranker sidecar

The cross-encoder of [[Milestone 13 - Distilled Ranking]] (ADR-0032): a query and the top of
the page go in, one relevance score per candidate comes out, and the API fuses that order with
its own by reciprocal rank. **Off by default** (`[reranker] enabled = false`, and a runtime
switch in the console); the search page is unchanged when the service is down or slow.

**Model.** Qwen3-Reranker-0.6B (Apache-2.0), INT8 ONNX (~570 MB) via
[`qwen3-embed`](https://github.com/n24q02m/qwen3-embed) — ONNX Runtime, no PyTorch. Runs on the
CPU-only reference machine; uses CUDA when `onnxruntime-gpu` is installed and a GPU is present.
Weights are downloaded on first start into `data/models/hf/` (git-ignored).

## Run

```bash
cd services/reranker
uv venv .venv --python 3.12 && . .venv/bin/activate
uv pip install -r requirements.txt
uvicorn app:app --host 127.0.0.1 --port 8096
python test_contract.py            # against the running service
```

Then in `config/dev.toml`: `[reranker] enabled = true`, or flip the switch on the console's
Compute page.

## Wire contract

```
POST /rerank   {"query": "...", "documents": ["title — excerpt", ...]}  -> {"scores": [0..1, ...]}
GET  /health   -> 200 when the model is loaded, else 503
```

`RERANK_MAX_DOCS` (50) and `RERANK_MAX_CHARS` (1200) cap a request. Only latency, status and
the pair count are logged.
