# STT sidecar. A plain Python base — whisper `small` at int8 runs on CPU, the reference path. On a
# GPU host, set STT_DEVICE=cuda and STT_COMPUTE=float16 (and use a CUDA base). ffmpeg is needed by
# PyAV to decode the browser's Opus/WebM recording. Weights are provisioned into a mounted cache.
FROM python:3.12-slim

ENV PYTHONUNBUFFERED=1 \
    HF_HOME=/models/hf \
    STT_MODEL=small

RUN apt-get update \
    && apt-get install -y --no-install-recommends ffmpeg \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY app.py .

# No EXPOSE of a host port: reached only over the private network by the Rust API.
CMD ["uvicorn", "app:app", "--host", "0.0.0.0", "--port", "8093"]
