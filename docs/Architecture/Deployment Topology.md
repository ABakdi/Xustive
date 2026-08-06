---
tags:
  - architecture
  - ops
type: architecture
status: specified
updated: 2026-08-06
---

# Deployment Topology

> How the [[Component Map]] binaries become running containers on Algerian infrastructure.

---

## 1. Environments

| Env | Purpose | Scale | Data |
|:---|:---|:---|:---|
| `dev` | laptop, `docker compose up` | 1 of everything | 10k sample docs |
| `staging` | pre-prod, same topology as prod | ½ prod | weekly index snapshot |
| `prod` | public | see §3 | live |

Staging and prod must differ only in replica counts and secrets. No config flag may be
`if env == "prod"` — differences live in `config/{env}.toml`.

---

## 2. Container Inventory

| Service | Image | Replicas (prod) | CPU | RAM | Volume |
|:---|:---|:---|:---|:---|:---|
| `caddy` | `caddy:2-alpine` | 1 | 0.5 | 256 MB | certs |
| `xustive-api` | built | 3 | 2 | 1 GB | — |
| `xustive-ml` | built | 2 | 6 | 12 GB | `models:/models:ro` |
| `xustive-crawler` | built | 4 | 1 | 1 GB | — |
| `xustive-worker` | built | 4 | 2 | 3 GB | — |
| `meilisearch` | `getmeili/meilisearch:v1.x` | 1 (+1 replica) | 8 | 32 GB | `meili_data` |
| `qdrant` | `qdrant/qdrant:latest` | 1 | 4 | 8 GB | `qdrant_data` |
| `redis` | `redis:7-alpine` | 1 | 2 | 8 GB | `redis_data` (AOF) |
| `prometheus` | `prom/prometheus` | 1 | 1 | 2 GB | `prom_data` |
| `grafana` | `grafana/grafana` | 1 | 0.5 | 512 MB | `grafana_data` |

**Baseline host for 10M documents:** 32 vCPU / 128 GB RAM / 2 TB NVMe. Meilisearch is the RAM-hungry
service; [[Summarizer]] is the CPU-hungry one. A single GPU (8 GB+) collapses `xustive-ml` to one
replica and cuts summary latency ~4× — optional, not required.

---

## 3. Network Segmentation

```
        internet
           │ 443
      ┌────▼────┐
      │  caddy  │   TLS 1.3, HTTP/2, HSTS, CSP header
      └────┬────┘
   net: edge (external)
           │
      ┌────▼────────┐
      │ xustive-api │
      └────┬────────┘
   net: core (internal only)
     ├── meilisearch      ← NEVER exposed publicly
     ├── qdrant           ← NEVER exposed publicly
     ├── redis            ← NEVER exposed publicly
     └── xustive-ml
   net: ingest (internal only, egress allowed)
     ├── xustive-crawler  ← the ONLY services with outbound internet
     └── xustive-worker
   net: obs (internal only)
     ├── prometheus  └── grafana (behind VPN/basic auth)
```

**Egress rule:** only `xustive-crawler` may reach the public internet. `xustive-api` and
`xustive-ml` have **no** egress route — this is what makes "no data leaves the country" enforceable
rather than aspirational. See [[Security and Privacy]].

---

## 4. `docker-compose.yml` skeleton

```yaml
services:
  meilisearch:
    image: getmeili/meilisearch:v1.13
    environment:
      MEILI_MASTER_KEY: ${MEILI_MASTER_KEY}
      MEILI_ENV: production
      MEILI_MAX_INDEXING_MEMORY: 16Gb
      MEILI_EXPERIMENTAL_DUMPLESS_UPGRADE: "true"
    volumes: [meili_data:/meili_data]
    networks: [core]
    healthcheck:
      test: ["CMD", "curl", "-fsS", "http://localhost:7700/health"]
      interval: 10s
    deploy: { resources: { limits: { memory: 32G } } }

  xustive-api:
    build: { context: ., dockerfile: crates/api/Dockerfile }
    environment:
      XUSTIVE_CONFIG: /config/prod.toml
      MEILI_URL: http://meilisearch:7700
      REDIS_URL: redis://redis:6379
    networks: [edge, core]
    depends_on:
      meilisearch: { condition: service_healthy }
    healthcheck:
      test: ["CMD", "curl", "-fsS", "http://localhost:8080/readyz"]
    deploy: { replicas: 3 }
  # … xustive-ml, xustive-crawler, xustive-worker, redis, qdrant, caddy, prometheus, grafana

volumes: { meili_data:, qdrant_data:, redis_data:, models:, prom_data:, grafana_data: }
networks:
  edge:   {}
  core:   { internal: true }
  ingest: {}
  obs:    { internal: true }
```

---

## 5. Build

- Multi-stage Rust builds: `cargo-chef` layer for dependency caching → `rust:1.8x` builder →
  `debian:bookworm-slim` runtime. Target image ≤ 150 MB per binary.
- `xustive-ml` needs `libtorch` / `llama.cpp` shared objects → its runtime layer is larger (~1.2 GB).
- Models are **not** baked into images. They are fetched once by a `model-init` job into the `models`
  volume and checksum-verified. Rationale: image size, and licence terms differ per model.

| Model | File | Size | Used by |
|:---|:---|:---|:---|
| Whisper small (multilingual, quantised) | `ggml-small-q5_1.bin` | ~500 MB | [[Speech to Text]] |
| Summarisation LLM (Qwen2.5-3B / Phi-3-mini, Q4_K_M) | `*.gguf` | ~2.2 GB | [[Summarizer]] |
| CLIP ViT-B/32 | `clip-vit-b32.ot` | ~350 MB | [[Image Pipeline]] |
| DziriBERT | `dziribert/` | ~500 MB | [[Query Expander]], [[Sentiment Engine]] |
| Tesseract traineddata `ara`, `fra`, `eng` | `tessdata/` | ~90 MB | [[Image Pipeline]] |

---

## 6. Startup Order and Readiness

```
redis, meilisearch, qdrant  →  model-init  →  xustive-ml  →  xustive-api  →  caddy
                                            →  xustive-worker  →  xustive-crawler
```

`readyz` semantics per service:

| Service | Ready when |
|:---|:---|
| `xustive-api` | Meilisearch `/health` ok **and** Redis PING ok |
| `xustive-ml` | all model files loaded into memory |
| `xustive-worker` | Redis consumer group joined |
| `xustive-crawler` | Redis ok **and** [[Proxy Manager]] has ≥ 1 healthy proxy |

Crawlers start **last** on purpose — never begin fetching before there is somewhere to put the data.

---

## 7. Backup and Restore

| Asset | Method | Frequency | RPO | RTO |
|:---|:---|:---|:---|:---|
| Meilisearch | native snapshot → off-host | 6 h | 6 h | 30 min |
| Qdrant | collection snapshot | daily | 24 h | 20 min |
| Redis | AOF `everysec` + RDB copy | 1 h | 1 h | 5 min |
| Source registry | JSON export to git | on change | 0 | 1 min |
| Grafana dashboards | provisioned from git | on change | 0 | 1 min |

Redis loss is **acceptable** — it costs re-crawl work, not data. Meilisearch loss is expensive but
recoverable by full re-crawl (days). Restore drills are a checklist item in
[[Milestone 4 - Quality and Operations]].

---

## 8. Deployment Procedure

1. CI builds and tags images `:{git-sha}`.
2. Deploy to `staging`; run smoke suite + relevance eval ([[Testing Strategy]]).
3. Roll `xustive-api` one replica at a time (`readyz` gate between each).
4. Roll `xustive-worker`/`xustive-crawler` (they drain by finishing the in-flight message, then exit).
5. Meilisearch upgrades: snapshot → stop → upgrade → start → verify doc count → resume ingestion.
6. Rollback = redeploy previous tag; index schema changes roll back via alias flip ([[Data Model]]).

**Graceful shutdown:** on `SIGTERM`, workers stop claiming new messages, finish the current one,
`XACK`, then exit. Grace period 30 s; anything in-flight is redelivered by the consumer group anyway.

---

## 9. Open Questions

- [ ] Kubernetes at what scale? Compose is sufficient to ~20M docs on one big host.
- [ ] Where do off-host backups physically live, given the data-sovereignty requirement?
- [ ] Do we need a second availability zone before beta, or is a documented RTO acceptable?

## Related

[[System Architecture]] · [[Observability]] · [[Security and Privacy]] ·
[[Error Handling and Resilience]] · [[Local Development]]
