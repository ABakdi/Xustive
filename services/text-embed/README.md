# text-embed sidecar

The embedding half of **semantic search** (M7-T02). A batch of strings in, one unit vector out per
string. Kept a separate service so the ML runtime stays out of the Rust build; the Rust client is
`xustive_vector::text::TextEmbedder`, and the vectors live in a Qdrant text collection.

## Model

Default **`BAAI/bge-m3`** — multilingual (Arabic, Darija, French, English), Apache-2.0, 1024-d. It
runs on CPU (the reference-hardware path) and uses the GPU automatically when one is present.

Swap it with `TEXT_EMBED_MODEL` (e.g. `intfloat/multilingual-e5-small`, 384-d, much lighter on CPU).
**If you change the model, set the Rust `[vector] text_dim` to match** — the Qdrant collection is
created with a fixed vector size, and a mismatch is rejected at upsert time.

## Wire contract

```
POST /embed   {"texts": ["...", ...]}   ->  {"vectors": [[N floats], ...]}
GET  /health                             ->  200 once the model is loaded
```

## Weights

Provisioned into the mounted `text_models` volume out-of-band (`HF_HUB_OFFLINE=1` in compose), never
downloaded at runtime — the serving plane has no route to the internet. First run on a fresh machine
needs the model fetched into that cache once (a model-init step, like the summariser's).

## GPU

The image installs CPU torch. For a GPU host, install the CUDA-matched wheel; the code picks the GPU
up via `torch.cuda.is_available()`.
