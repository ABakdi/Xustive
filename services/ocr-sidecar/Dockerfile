# Unlimited-OCR sidecar. CUDA base image because the 3B VLM runs on the GPU — this container has no
# CPU-only mode, which is the whole reason the engine ships tesseract as the default and this as an
# opt-in. The model weights are NOT baked in; mount or download them at runtime (see README) so the
# image stays small and the licence/model provenance stays explicit.
FROM nvidia/cuda:12.9.0-runtime-ubuntu22.04

ENV PYTHONUNBUFFERED=1 \
    HF_HOME=/models/hf \
    OCR_MODEL=baidu/Unlimited-OCR

RUN apt-get update \
    && apt-get install -y --no-install-recommends python3 python3-pip \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY requirements.txt .
# torch/torchvision CUDA wheels come from the PyTorch index; transformers etc. from PyPI.
RUN pip3 install --no-cache-dir --extra-index-url https://download.pytorch.org/whl/cu129 \
    -r requirements.txt

COPY app.py .

# No EXPOSE of a host port: this is reached only over the private network by the Rust API, never
# published. The orchestrator maps it internally.
CMD ["uvicorn", "app:app", "--host", "0.0.0.0", "--port", "8091"]
