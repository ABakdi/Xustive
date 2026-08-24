# Text embed sidecar (M7-T02). A plain Python base, not a CUDA one: the default bge-m3 runs on CPU,
# which is the reference-hardware path. On a GPU host, swap in a CUDA torch wheel and the code uses
# the GPU automatically (it checks torch.cuda.is_available()). Weights are provisioned into a mounted
# cache, not baked into the image.
FROM python:3.12-slim

ENV PYTHONUNBUFFERED=1 \
    HF_HOME=/models/hf \
    TEXT_EMBED_MODEL=BAAI/bge-m3

WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY app.py .

# No EXPOSE of a host port: reached only over the private network by the Rust API and worker.
CMD ["uvicorn", "app:app", "--host", "0.0.0.0", "--port", "8094"]
