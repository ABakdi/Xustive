# STT sidecar

Speech-to-text for voice search ([[Milestone 3 - Multimodal Input]], M3-T02). The browser records a
short clip, this turns it into text, and the text lands in the search box **editable and never
auto-submitted**. It wraps Whisper `small` on [faster-whisper](https://github.com/SYSTRAN/faster-whisper)
(CTranslate2) behind a tiny HTTP contract, matching the OCR and CLIP sidecars.

**CPU-capable.** Whisper `small` at int8 transcribes a short clip in a second or two on CPU, so voice
is **not** GPU-gated — it uses a GPU when one is present (`STT_DEVICE=cuda`, `STT_COMPUTE=float16`).

## Wire contract

```
POST /transcribe   body = raw audio bytes (Content-Type: audio/*), optional ?lang=ar
                   ->  {"text": "...", "language": "ar"}
GET  /health                                                        ->  200 when loaded, else 503
```

The Rust client is in `xustive-api` (`stt.rs`). The frontend passes the UI language as `?lang=` so
Arabic audio is not mis-detected as something else on a short clip.

## Run it

```bash
uv venv -p 3.12 .venv && uv pip install -p .venv/bin/python -r requirements.txt   # 3.14 has no PyAV wheel yet
STT_MODEL=$PWD/../../data/models/faster-whisper-small .venv/bin/uvicorn app:app --host 127.0.0.1 --port 8093
```

`STT_MODEL` may be a Whisper size (`small`, fetched into `HF_HOME` on first start) or a directory.
Prefer the directory: on 2026-08-27 the hub client's unauthenticated download ran at ~60 KB/s
while a plain fetch of the same files ran at 6 MB/s, so the weights are fetched by hand —
`config.json`, `model.bin`, `tokenizer.json`, `vocabulary.txt` from
`https://huggingface.co/Systran/faster-whisper-small/resolve/main/` into
`data/models/faster-whisper-small/` (gitignored, ~486 MB). The model then loads in three seconds.

Docker (weights mounted, not baked in):

```bash
docker build -t xustive-stt-sidecar .
docker run -v /path/to/hf-cache:/models/hf xustive-stt-sidecar          # CPU
```

Turn it on in `config/*.toml`:

```toml
[stt]
enabled = true
endpoint = "http://stt-sidecar:8093/transcribe"
```

Wired into `deploy/docker-compose.yml` as the `stt-sidecar` service behind the `voice` profile, on
the internal `core` network with a `mem_limit`, no published port, and offline weights
(`HF_HUB_OFFLINE=1`) provisioned into the `stt_models` volume.

## Privacy

The audio is transcribed in memory and never written to disk. Only that a request succeeded and its
latency is logged — never the audio or the transcript. This is the entire reason to self-host
transcription instead of sending voice to a cloud API.

## Quality note

Darija (Algerian Arabic) transcription is imperfect — the transcript is editable precisely so a
wrong word can be fixed before searching, and the UI says so rather than presenting it as certain.
Modern Standard Arabic and French fare better. WER targets are in the milestone's exit gate.
