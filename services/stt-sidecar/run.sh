#!/usr/bin/env bash
# Start the STT sidecar with what this machine has.
#
# The GPU is used when CTranslate2 can see one: on a Quadro T1000 the encoder runs in ~130 ms at
# float32 — and, measured, float16 is *slower* on that part (435 ms), so float32 is the default
# compute type here; a card with real FP16 throughput can set STT_COMPUTE=float16. The CUDA 12
# runtime libraries come from the pip wheels in the venv (the host's CUDA 13 does not match
# CTranslate2's build), which is what the LD_LIBRARY_PATH line is for. Without a GPU it falls
# back to CPU int8 — correct, just not live.
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$(cd ../.. && pwd)"
export STT_MODEL="${STT_MODEL:-$ROOT/data/models/faster-whisper-small}"
export STT_PARTIAL_MODEL="${STT_PARTIAL_MODEL:-$ROOT/data/models/faster-whisper-base}"
NVLIBS="$(ls -d .venv/lib/python3.*/site-packages/nvidia/*/lib 2>/dev/null | tr '\n' ':' || true)"
export LD_LIBRARY_PATH="${NVLIBS}${LD_LIBRARY_PATH:-}"
if [ -z "${STT_DEVICE:-}" ]; then
  if .venv/bin/python -c "import ctranslate2 as c, sys; sys.exit(0 if c.get_cuda_device_count() > 0 else 1)" 2>/dev/null; then
    export STT_DEVICE=cuda STT_COMPUTE="${STT_COMPUTE:-float32}"
  else
    export STT_DEVICE=cpu STT_COMPUTE="${STT_COMPUTE:-int8}"
  fi
fi
exec .venv/bin/uvicorn app:app --host "${STT_HOST:-127.0.0.1}" --port "${STT_PORT:-8093}"
