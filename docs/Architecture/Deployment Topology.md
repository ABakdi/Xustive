---
tags:
  - architecture
  - ops
type: architecture
status: implemented
updated: 2026-08-27
---

# Deployment Topology

> How the [[Component Map]] processes become running containers on Algerian infrastructure.

> **Audited against `deploy/` on 2026-08-27.** The compose files are the source of truth:
> `deploy/docker-compose.yml` (base, production posture) and `deploy/docker-compose.dev.yml`
> (localhost port publishing). Where the 2026-08-06 plan differs from them it is marked below.

---

## 1. Environments

| Env | Purpose | Scale | Data |
|:---|:---|:---|:---|
| `dev` | laptop, `make dev` | 1 of everything; Rust processes and the web tier run **on the host**, infrastructure in Compose | 10k sample docs (`make corpus seed`) or a real crawl |
| `staging` | pre-prod, same topology as prod | `api.max_concurrent = 256` | weekly index snapshot |
| `prod` | public | `api.max_concurrent = 512`, `timeout_search_ms = 1500`, JSON logs | live |

Staging and prod must differ only in replica counts and secrets. No config flag may be
`if env == "prod"` — differences live in `config/{env}.toml`. Today `prod.toml` and
`staging.toml` differ from `dev.toml` in exactly: `environment`, `api.max_concurrent`,
`api.timeout_search_ms`, `api.cors_origins` (empty — same origin), `search.meili_url`
(`http://meilisearch:7700`), `search.timeout_ms` (800), and `telemetry` (`info`, JSON). Every
other section is left at the defaults in `xustive-core::config`.

---

## 2. Container Inventory

What `deploy/docker-compose.yml` declares. Limits are `mem_limit`/`cpus` (the Compose-v2
spellings that work without Swarm — `deploy.resources` is silently ignored by `docker compose up`)
and are the **development slice** for a 12-core host; production figures are in
[[Performance Budgets]] §7.

| Service | Image | Profile | Networks | CPU | RAM | Volume |
|:---|:---|:---|:---|:---|:---|:---|
| `meilisearch` | `getmeili/meilisearch:v1.13` | always | `core` | 6 | 5 GB (`MEILI_MAX_INDEXING_MEMORY=3Gb`, 6 threads) | `meili_data` |
| `qdrant` | `qdrant/qdrant:v1.12.4` | always | `core` | 2 | 2 GB | `qdrant_data` |
| `redis` | `redis:7-alpine` | always | `core` | 1 | 1.5 GB (`maxmemory 1gb noeviction`, AOF `everysec`) | `redis_data` |
| `redis-signals` | `redis:7-alpine` | always | `core` | 0.5 | 256 MB (`volatile-lru`, no AOF, no RDB) | **none, by design** |
| `toold` | built, `deploy/Dockerfile.toold` | always | `ingest` + `core` | 0.5 | 256 MB | — |
| `ocr-sidecar` | built, `services/ocr-sidecar` | `ocr` | `core` | 4 | 10 GB + **1 GPU** | `ocr_models:ro` |
| `clip-embed` | built, `services/clip-embed` | `vector` | `core` | 2 | 2 GB | `clip_models:ro` |
| `stt-sidecar` | built, `services/stt-sidecar` | `voice` | `core` | 2 | 2 GB | `stt_models:ro` |
| `text-embed` | built, `services/text-embed` | `semantic` | `core` | 2 | 4 GB | `text_models:ro` |
| `searxng` | `searxng/searxng:latest` | `federation` | `ingest` | 1 | 512 MB; `logging: driver: none` | — |
| `federator` | built, `deploy/Dockerfile.federator` | `federation` | `core` + `ingest` | 0.5 | 256 MB | — |
| `prometheus` | `prom/prometheus:v3.1.0` | always | `core` + `obs` | 1 | 1 GB, 15 d retention | `prom_data` |
| `grafana` | `grafana/grafana:11.4.0` | always | `obs` | 0.5 | 512 MB | `grafana_data` |

**Not in Compose (2026-08-27):** `xustive-api`, the Next.js web tier, `xustive-cli crawld` and
`xustive-cli worker`. In development they run on the host (`scripts/dev.sh` starts api → web →
worker → toold `--once` → crawld, in that order) and reach the containers through the
localhost ports the dev override publishes. There is no `Dockerfile` for the API or the web tier
yet and no `caddy` service — TLS termination and the production packaging of the host-run
processes are still to be written; see §10. `searxng:latest` is the one deliberate exception
to the pin-every-image rule (it publishes only rolling date tags, and it is off by default).

**Baseline host for 10M documents:** 32 vCPU / 128 GB RAM / 2 TB NVMe. Meilisearch is the RAM-hungry
service; [[Summarizer]] is the CPU-hungry one. The reference GPU is a Quadro T1000 (4 GB); the
summariser uses it through the API's `cuda` feature when `nvcc` is found, and the STT sidecar
when CTranslate2 sees it. Only the OCR sidecar *requires* a GPU (≥ 8 GB).

---

## 3. Network Segmentation

```
        internet
           │
   net: ingest (the ONLY network with a route out)
     ├── toold          ← weather, rates, Wikidata harvest; no user input
     ├── searxng        ← [profile federation] third-party engines see our IP, never a reader's
     └── federator ─────┐   [profile federation] dual-homed
                        │
   net: core (internal: true — NO egress)
     ├── meilisearch    ← NEVER exposed publicly
     ├── qdrant         ← NEVER exposed publicly
     ├── redis          ← NEVER exposed publicly
     ├── redis-signals  ← ephemeral
     ├── toold          ← dual-homed: joins core to reach Redis and Meilisearch
     ├── ocr-sidecar · clip-embed · stt-sidecar · text-embed   (HF_HUB_OFFLINE=1)
     └── prometheus
   host (dev) / edge (prod, to be packaged)
     ├── xustive-api    ← reaches core only; no internet route
     ├── web (Next.js)  ← has egress: Wikipedia/Wikidata lookups, thumbnail proxy
     ├── crawld · worker ← egress, on the host today
   net: obs (internal only)
     ├── prometheus  └── grafana (admin/admin default — change it; anonymous off)
   net: devhost (dev override only)
     └── bridge with IP masquerade DISABLED: published-port ingress works, egress does not
```

**Egress rule:** the serving plane has no route to the internet. `core` is `internal: true`, and
`make egress-test` (`scripts/test-egress.sh`) starts a throwaway container on that network and
proves HTTP and DNS both fail. The dev `devhost` bridge exists only so host-run processes can
reach the containers, and `enable_ip_masquerade: "false"` keeps it from becoming an egress path
(BUG-009). Three things cross the boundary, each by design: `toold` (scheduled, no user input),
`federator` (query text, [[ADR-0017 - Query-Time Federation with External Metasearch]]) and the
web tier ([[ADR-0014 - Knowledge Panel from Wikipedia via the Web Tier]]). See
[[Security and Privacy]].

**Dev ports** (all bound to `127.0.0.1`, overridable in `.env`): Meilisearch 7700, Qdrant 6333,
Redis **6390** (not 6379 — another local stack owning 6379 is common), redis-signals 6391, OCR
8091, CLIP 8092, STT 8093, text-embed 8094, federator 8095, reranker 8096, Prometheus 9090, Grafana 3001. The
API listens on 8080 and the web tier on 3000; the web tier proxies `/api/v1/*` so the browser
talks to one origin.

---

## 4. `docker-compose.yml` — the shape

The real file is `deploy/docker-compose.yml`; this is its skeleton, kept here so the note reads
without the repo open.

```yaml
name: xustive
services:
  meilisearch:
    image: getmeili/meilisearch:v1.13
    mem_limit: 5g
    cpus: 6.0
    environment:
      MEILI_ENV: development            # prod: production
      MEILI_MASTER_KEY: ${MEILI_MASTER_KEY:-}
      MEILI_NO_ANALYTICS: "true"
      MEILI_MAX_INDEXING_MEMORY: 3Gb
      MEILI_MAX_INDEXING_THREADS: 6
      MEILI_EXPERIMENTAL_ENABLE_METRICS: "true"
    volumes: [meili_data:/meili_data]
    networks: [core]
    healthcheck: { test: ["CMD-SHELL", "curl -fsS http://localhost:7700/health || exit 1"] }

  redis-signals:                        # ADR-0018: no persistence on purpose
    image: redis:7-alpine
    command: redis-server --save "" --appendonly no --maxmemory 192mb --maxmemory-policy volatile-lru
    networks: [core]

  toold:
    build: { context: .., dockerfile: deploy/Dockerfile.toold }
    environment: { REDIS_URL: redis://redis:6379 }
    networks: [ingest, core]

  stt-sidecar:                          # likewise ocr-sidecar (ocr), clip-embed (vector), text-embed (semantic)
    build: { context: ../services/stt-sidecar }
    profiles: ["voice"]
    environment: { STT_MODEL: small, HF_HOME: /models/hf, HF_HUB_OFFLINE: "1" }
    volumes: [stt_models:/models/hf:ro]
    networks: [core]

  searxng:
    image: searxng/searxng:latest
    profiles: ["federation"]
    volumes: [../services/searxng/settings.yml:/etc/searxng/settings.yml:ro]
    networks: [ingest]
    logging: { driver: "none" }         # its stdout is query text

  federator:
    build: { context: .., dockerfile: deploy/Dockerfile.federator }
    profiles: ["federation"]
    environment:
      FEDERATOR_BIND: 0.0.0.0:8095
      SEARXNG_URL: http://xustive-searxng:8080
      EXTERNAL_LLM_URL: ${XUSTIVE_EXTERNAL_LLM_URL:-}      # opt-in external summariser
      EXTERNAL_LLM_KEY_FILE: ${XUSTIVE_EXTERNAL_LLM_KEY_FILE:-}
    networks: [core, ingest]
  # … qdrant, redis, prometheus, grafana

volumes: { meili_data:, qdrant_data:, redis_data:, prom_data:, grafana_data:,
           text_models:, ocr_models:, clip_models:, stt_models: }
networks:
  core:   { internal: true }
  obs:    { internal: true }
  ingest: {}
```

Superseded (2026-08-06 → 2026-08-27): the planned `xustive-api` service with `crates/api/Dockerfile`,
`MEILI_URL`/`REDIS_URL` env, `edge` network and `deploy: { replicas: 3 }` was never written; the
API is configured through `config/{env}.toml` plus a short list of `XUSTIVE_*` overrides, and runs
on the host.

---

## 5. Build

- Rust images that exist: `deploy/Dockerfile.toold` and `deploy/Dockerfile.federator`. Both are
  two-stage — `rust:1-slim` builder → `debian:bookworm-slim` runtime with `ca-certificates`,
  running as an unprivileged `xustive` user. Each is a **separate image on purpose**: the two
  processes that cross the egress boundary must not share an image with the serving plane, so the
  separation is a build-time fact. No `cargo-chef` layer yet.
- `xustive-api` links llama.cpp (feature `summariser`, default on; `--no-default-features` or
  `make dev --fast` skips it; `--features cuda` for the GPU). The first build compiles llama.cpp
  from source and takes several minutes.
- Sidecars are Python (FastAPI/uvicorn) images built from `services/*`. Models are **not** baked
  in: each mounts a read-only volume in HuggingFace cache layout and runs with
  `HF_HUB_OFFLINE=1` / `TRANSFORMERS_OFFLINE=1`, so a sidecar on `core` can never download.
- Models are fetched by `scripts/fetch-models.sh` (summariser GGUFs), `scripts/fetch-geoip.sh`
  (DB-IP city-lite `.mmdb`) and per-sidecar READMEs, into `data/models/` on the host or the
  named volumes. Licence terms differ per model, which is the other reason they are not in images.

| Model | Where | Size | Used by |
|:---|:---|:---|:---|
| Qwen2.5-3B-Instruct Q4_K_M (default; **non-commercial** research licence) or Qwen2.5-1.5B-Instruct Q4_K_M (Apache-2.0) | `data/models/*.gguf` via `fetch-models.sh` | ~2.0 GB / ~1.1 GB | [[Summarizer]] |
| faster-whisper `small` (final) + `base` (partials), CTranslate2 | `data/models/faster-whisper-*` / `stt_models` | ~500 MB | [[Speech to Text]] |
| `openai/clip-vit-base-patch32` | `clip_models` | ~600 MB | [[Image Pipeline]], [[Vector Index]] |
| `BAAI/bge-m3` (1024-d) | `text_models` | ~2.3 GB | semantic candidates, [[Vector Index]] |
| `baidu/Unlimited-OCR` (3B VLM) | `ocr_models` | GPU ≥ 8 GB | [[Image Pipeline]] via `ocr-sidecar` |
| Tesseract `ara`, `fra`, `eng` | `data/tessdata` (`media.tessdata_dir`) | ~90 MB | [[Image Pipeline]] in-process |
| DB-IP city lite | `data/geoip/dbip-city-lite.mmdb` | ~130 MB | approximate location, [[ADR-0020 - Approximate Location from a Local Database]] |

Superseded: the 2026-08-06 table listed `ggml-small-q5_1.bin` (whisper.cpp), `clip-vit-b32.ot`
(tch) and DziriBERT. Whisper runs in the Python sidecar on faster-whisper, CLIP in the
`clip-embed` sidecar, and DziriBERT was never adopted — the [[Query Expander]] and
[[Sentiment Engine]] are lexicon- and morphology-driven.

---

## 6. Startup Order and Readiness

```
redis, redis-signals, meilisearch, qdrant  →  toold (needs redis healthy)
                                            →  [profiles] sidecars, searxng → federator
host:  xustive-api (waits for /healthz)  →  web  →  worker  →  toold --once  →  crawld
```

`readyz` semantics per process:

| Process | Ready when |
|:---|:---|
| `xustive-api` | Meilisearch `/health` ok (`GET /readyz` → 200; 503 "search backend unavailable/unreachable" otherwise). Redis is **not** part of readiness: search must serve while the queue is down. |
| sidecars | `GET /health` → 200 once the model is loaded, 503 before (start periods 60–120 s in Compose) |
| `federator` | `GET /healthz` |
| `worker` | Redis consumer group `indexers` joined |
| `crawld` | Redis reachable; frontier seeded from `data/sources/registry.jsonl` and `seeds` |

Crawlers start **last** on purpose — never begin fetching before there is somewhere to put the data.
`crawld` also pauses itself when `q:index` is deep, so a slow indexer never turns into a Redis
full of unindexed documents.

---

## 7. Backup and Restore

`make backup [DEST=dir]` runs `scripts/backup.sh`; `make restore-drill` exercises
`scripts/restore.sh`.

| Asset | Method | Frequency | RPO | RTO |
|:---|:---|:---|:---|:---|
| Meilisearch | native snapshot (`POST /snapshots`, wait for the task, `docker cp` out) | 6 h | 6 h | 30 min |
| Qdrant | collection snapshot (`image_clip`; set `QDRANT_COLLECTIONS` to add `text_bge`) | daily | 24 h | 20 min |
| Redis | AOF `everysec` + RDB copy from the container | 1 h | 1 h | 5 min |
| `redis-signals` | **never backed up** — ephemeral by [[ADR-0018 - Anonymous Search History]] | — | — | — |
| Source registry | `data/sources/registry.jsonl` copied; also in git | on change | 0 | 1 min |
| Grafana dashboards | provisioned from `deploy/grafana/provisioning` in git | on change | 0 | 1 min |

Redis loss is **acceptable** — it costs re-crawl work, not data. Meilisearch loss is expensive but
recoverable by full re-crawl (days). The `knowledge` index is rebuilt by `toold`'s harvest from
`data/knowledge/seeds.tsv`. Restore drills are in [[Runbooks]] and were a checklist item in
[[Milestone 4 - Quality and Operations]].

---

## 8. Deployment Procedure

The intended procedure; steps 1 and 3 assume packaging that does not exist yet (§10).

1. CI builds and tags images `:{git-sha}`.
2. Deploy to `staging`; run `make smoke` + relevance eval `make eval` ([[Testing Strategy]]).
3. Roll `xustive-api` one replica at a time (`readyz` gate between each).
4. Roll `worker`/`crawld` — both drain: on `SIGTERM` or Ctrl-C the in-flight fetch finishes, its
   document is queued, and the loop exits; the worker finishes its batch and acks.
5. Meilisearch upgrades: snapshot → stop → upgrade → start → verify doc count → resume ingestion.
6. Rollback = redeploy previous tag; index settings changes go through `xustive-cli reindex`
   ([[Data Model]]).

**Graceful shutdown:** workers stop claiming new messages, finish the current one, `XACK`, then
exit. Anything in flight is reclaimed by the consumer group after `RECLAIM_AFTER` (300 s) anyway,
and a job that fails three times goes to `q:index:dead`.

---

## 9. Open Questions

- [ ] Kubernetes at what scale? Compose is sufficient to ~20M docs on one big host.
- [ ] Where do off-host backups physically live, given the data-sovereignty requirement?
- [ ] Do we need a second availability zone before beta, or is a documented RTO acceptable?

## 10. Not Built Yet

Built since (2026-09-01), and now described in [[Deploying to a VPS]]: images for `xustive-api`
(`Dockerfile.api`), the Next.js tier (`Dockerfile.web`, `output: 'standalone'`) and the crawl
plane (`Dockerfile.cli`, one image serving `crawld`, `worker` and the one-shot commands); the
`edge` network and the Caddy terminator; `docker-compose.prod.yml`; `make preflight` /
`make deploy`.

Still outstanding:

- `model-init` as a Compose job; model provisioning is scripts plus READMEs.
- `cargo-chef` dependency caching in the Rust Dockerfiles — the API image recompiles llama.cpp
  from scratch on any source change, which is 15–30 minutes.
- Everything in [[Milestone 14 - One Server, Many Hands]]: volunteer contributions, sharding,
  the Helm chart.

## 11. Three deployments of the same system

Specified in [[Milestone 14 - One Server, Many Hands]]; §1–§10 above describe the first of them,
which is what exists today.

| | **One server** | **One server, many hands** | **Many servers** |
|:---|:---|:---|:---|
| Runs | everything in §2 on one host | the same, plus the [[Contribution Coordinator]] | the same, sharded, under Kubernetes (§12) |
| Crawls with | one IP, one host at a time | its own IP **and** every volunteer's ([[ADR-0033]]) | the same, from a crawler Deployment |
| GPU work | one card, mostly deferred | volunteers' cards, batch only ([[ADR-0034]]) | a GPU node pool **and** volunteers |
| Index | one Meilisearch | one Meilisearch | N shards, routed by us ([[ADR-0035]]) |
| Install | `docker compose up` | the above, plus an ingress for `/contrib` and keys | `helm install` |
| Ceiling | host memory ([[Problems#PROB-004\|PROB-004]]) and host diversity ([[PROB-002 - Crawl and Index Throughput]]) | volunteers, and the single index | shards × their memory |

The step from the first to the second is **one route group and one ingress** — no new datastore,
no new process. The step to the third changes where things run, not what they do: the search
path, the ranking and the console are identical, which is the test [[ADR-0035]] sets for itself.

**The prerequisite for both steps is authentication**, which does not exist today: the API has no
keys and `/admin` is open (M14-T01). Nothing here may face the internet before it does.

## 12. Kubernetes

One chart, `deploy/helm/xustive`, with the compose topology preserved where it matters — the
network segregation of §3 becomes NetworkPolicies, and the only pods allowed to leave the cluster
are the federator and the crawler.

| Workload | Kind | Notes |
|:---|:---|:---|
| `api` | Deployment + HPA | stateless; CPU/latency-scaled; the search path is unchanged by sharding |
| `coordinator` | Deployment | the same image, `--role coordinator`, behind the **contributor** ingress with its own rate limits and key scope |
| `web` | Deployment | Next.js tier |
| `worker` | Deployment + KEDA | scaled on `q:index` stream length — the queue depth is already the signal the console draws |
| `crawld` | Deployment (1) | the operator's own crawler; egress-allowed |
| `federator` | Deployment | the one allow-listed hop to SearXNG ([[ADR-0017]]) |
| sidecars (CLIP, OCR, STT, text-embed, reranker) | Deployments | `nodeSelector` on the GPU pool; CPU replicas where the model allows |
| `meilisearch-0..N` | StatefulSet | one PVC each, one shard each; **memory limit above that shard's `usedDatabaseSize`** ([[Problems#PROB-004\|PROB-004]]) |
| `redis-queue`, `redis-signals` | StatefulSet | queue is durable, signals are not |
| `qdrant` | StatefulSet | vectors |
| Prometheus, Grafana | Deployments | as today; Grafana reachable only through the edge at `/grafana`, behind the console password |

**Sharding.** `search.shards` lists the shard services; rendezvous hashing on the document id
picks the owner. Reads fan out and merge before the existing re-rank; writes route by id; a slow
shard degrades like any other stage. Adding a shard is a StatefulSet replica plus
`xustive shard rebalance`, which moves about `1/N` of the corpus and nothing else.

**Orchestrating the volunteers.** They are not pods and never become pods. The coordinator holds
their leases in Redis, so scaling the coordinator is horizontal and a restart costs nothing worse
than a set of expiring leases. The contributor ingress is a separate host so it can be
rate-limited, or switched off, without touching search.

**Backups.** A per-shard Meilisearch dump on a CronJob to object storage, plus the Redis queue's
AOF; the restore runbook and its RTO live in [[Runbooks]].

## Related

[[System Architecture]] · [[Observability]] · [[Security and Privacy]] ·
[[Error Handling and Resilience]] · [[Running Xustive]] ·
[[Milestone 14 - One Server, Many Hands]] · [[Contribution Coordinator]] · [[Community Node]]
