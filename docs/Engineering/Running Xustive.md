---
tags:
  - engineering
---

# Running Xustive

> Everything about getting it running and keeping it running: first start, the parts, the ports,
> the commands, and what to do when something is wrong.
>
> Replaces the former *Local Development* and *Running the System*, which described the
> architecture from before the frontend was split out and disagreed with each other in several
> places.

---

## 1. One command

```bash
make setup     # once — checks prerequisites, installs git hooks, creates .env
make dev       # build and run everything
```

`make dev` starts the infrastructure containers, builds the workspace, then runs the API, the
frontend, the crawler, the index worker and the tool-data fetcher — logs interleaved with a prefix
per service. **Ctrl-C stops all of them.** The containers stay up; `make dev-down` stops those too.

```
▸ starting infrastructure
▸ building (the first build compiles llama.cpp — several minutes; --fast skips it)
▸ api ready
▸ starting services

  Xustive is running.

    Search      http://localhost:3000
    Admin       http://localhost:8080/admin
    Crawler doc http://localhost:8080/bot
    Grafana     http://localhost:3001

api     │ xustive-api listening addr=0.0.0.0:8080
web     │ ✓ Ready in 480ms
crawler │ crawler starting seeds=20 waiting=983
worker  │ indexed 6 · rejected 0 · dead-lettered 0
```

> The two `:8080` lines in that banner are stale: the admin console and the `/bot` page moved into
> the Next.js app, so they are **`http://localhost:3000/admin`** and **`http://localhost:3000/bot`**.
> `:8080/admin` is a 404 today. (`scripts/dev.sh` still prints the old addresses.)

**The first build takes minutes.** `xustive-api` links llama.cpp for the summariser and llama.cpp
compiles from source. The flags `scripts/dev.sh` accepts:

```bash
make dev ARGS=--fast          # no AI summaries; everything else works; builds in seconds
make dev ARGS=--no-crawler    # leave the crawler off, for frontend work
make dev ARGS=--cpu           # force the CPU even though a CUDA toolkit is present
make dev ARGS=--federation    # also start SearXNG + the gateway, and arm federation (§10)
```

When `/opt/cuda/bin/nvcc` exists the API is built with `--features cuda` and started with
`XUSTIVE_DEVICE=gpu`; otherwise CPU-only, and it says so. `--fast` implies CPU because it drops
llama.cpp altogether.

`make dev` **refuses to start** if something already holds 8080 or 3000. Starting alongside gives
a log full of "address in use" and — worse — the old processes keep serving, which is what makes
code changes appear to do nothing. `make dev-stop` stops a running `make dev` from another
terminal; `make dev-down` additionally kills whatever holds the two ports and stops the containers.

### If you would rather run the parts separately

```bash
make dev-up      # infrastructure only
make run-api     # the Rust API,         :8080  (builds with cuda when nvcc is present)
make run-api-fast # the API without the summariser
make run-web     # the Next.js frontend, :3000  (`next dev`)
make crawld      # the crawler
make worker      # drain the index queue into Meilisearch
make toold       # one pass of the tool-data fetcher (weather, knowledge)
```

### How the host processes are actually run day to day

`make dev` is the one-command path. The honest picture of a long-running development box — the
one this document was checked against — is four host processes started by hand, plus the
containers:

```bash
make dev-up                                                        # containers
cargo build -p xustive-api --features cuda && \
  target/debug/xustive-api --config config/dev.toml                # the API, debug build
cargo build --release -p xustive-cli && \
  target/release/xustive-cli --config config/dev.toml worker       # the index worker
  target/release/xustive-cli --config config/dev.toml crawld       # the crawler
cd web && npm run build && npx next start -p 3000                  # the frontend, production build
services/stt-sidecar/run.sh                                        # voice, optional (§10)
```

The crawler and worker are **release** builds because they are the CPU-bound paths; the API stays
**debug** for iteration speed (its CUDA build is heavy either way). The frontend is served with
`next start` rather than `next dev` because the dev server's hot-reload has no bearing on the
Rust side and `next start` behaves like the deployment.

**The caveat that costs afternoons:** none of these processes reload. A `cargo build` produces a
new binary, but the *running* `xustive-api`, `worker` and `crawld` are the old one until you stop
and start them, and a `next build` is not served until `next start` is restarted. "My change does
nothing" is, nine times in ten, a process that was never restarted. Find the owner of a port with
`ss -ltnp | grep -E ':8080|:3000'` and stop that pid, rather than a broad `pkill -f`, which will
happily take a shell or an editor with it.

---

## 2. Prerequisites

| | Version | Why |
|:---|:---|:---|
| Rust | 1.85+ | `rust-version` in the workspace `Cargo.toml` (edition 2021) |
| Docker + Compose | v2 | Meilisearch, Redis ×2, Qdrant, Prometheus, Grafana, `toold`, the optional sidecars |
| Node | 20+ | the frontend (Next 16) |
| Python 3 | any | the corpus generator and the bidi lint |
| `curl`, `psmisc` (`fuser`) | any | the scripts; `dev-down` frees ports with `fuser` |
| CUDA toolkit at `/opt/cuda` | 12+ | **optional** — GPU summaries; detected at build time, never required |
| `uv` + Python 3.12 | — | **optional** — the STT sidecar's venv (§10) |

`make setup` checks Rust, Docker, Python and `cargo-deny`, installs the git hooks
(`git config core.hooksPath .githooks`) and copies `.env.example` to `.env`. It is safe to re-run
and will not overwrite an existing `.env`. It does **not** check Node — `make run-web` does — and
it does not fetch models; that is `scripts/fetch-models.sh` (§9). Its closing hint still points at
`:8080` and a document that no longer exists; the target list in §6 is current.

Roughly 8 GB of RAM is comfortable; the summariser is the expensive part, and `--fast` removes it.
On the reference development card, a 4 GB Quadro T1000, the GPU budget is tight — see §9.

---

## 3. What runs where

| Port | Process | Serves |
|---:|:---|:---|
| **3000** | `xustive-web` (Next.js) | **the site — open this one**; also `/admin/*` (the console) and `/bot` |
| 8080 | `xustive-api` (Rust) | JSON only: `/api/v1/…` (search, suggest, summary, translate, transcribe, tools, `admin/*`), `/healthz`, `/readyz`, `/metrics` |
| 7700 | Meilisearch | the indexes: `documents`, `comments_v1`, `knowledge_v1`, `sources_v1` |
| 6390 | Redis (`xustive-redis`) | queue, frontier, robots, raw bodies and tool caches — persistent (AOF) |
| 6391 | Redis (`xustive-redis-signals`) | interaction counters and weak-coverage terms — **ephemeral, never backed up** (ADR-0018) |
| 6333 | Qdrant | image (`image_clip`) and text (`text_bge`) vectors — only used when `[vector]` is enabled |
| 9090 | Prometheus | metrics |
| 3001 | Grafana | dashboards (`admin` / `admin`) |
| 8091 | `ocr-sidecar` | `--profile ocr` only — Unlimited-OCR, needs an 8 GB GPU |
| 8092 | `clip-embed` | `--profile vector` only — image embeddings, CPU-capable |
| 8093 | `stt-sidecar` | `--profile voice`, or `services/stt-sidecar/run.sh` on the host (§10) |
| 8094 | `text-embed` | `--profile semantic` only — bge-m3, CPU-capable |
| 8095 | `federator` | `--profile federation` — the one egress hop, in front of SearXNG (§10) |

> **Open :3000, not :8080.** The API answers JSON and has served no pages since the Rust renderer
> was deleted in M1B — `localhost:8080/search` and `localhost:8080/admin` are **404s**, which read
> like a broken install and are not. The frontend proxies `/api/v1/*` through to 8080 (`XUSTIVE_API_URL`
> overrides the target), so the browser only ever talks to one origin, which is what keeps the CSP
> at `default-src 'self'`.

Every container port is published on `127.0.0.1` only, by `deploy/docker-compose.dev.yml`; the base
`deploy/docker-compose.yml` publishes nothing, and `scripts/lint-compose.sh` keeps it that way.
Redis defaults to **6390**, not 6379, because another local stack owning 6379 is common. Override
any of them in `.env`: `XUSTIVE_MEILI_PORT`, `XUSTIVE_QDRANT_PORT`, `XUSTIVE_REDIS_PORT`,
`XUSTIVE_REDIS_SIGNALS_PORT`, `XUSTIVE_PROM_PORT`, `XUSTIVE_GRAFANA_PORT`, and
`XUSTIVE_{OCR,CLIP,STT,TEXT_EMBED,FEDERATOR}_PORT` for the sidecars. SearXNG itself has no host
port — only the gateway talks to it.

---

## 4. The repository

```
xustive/
├── crates/
│   ├── xustive-api/        HTTP surface: search, suggest, summary, translate, transcribe, admin JSON
│   ├── xustive-cli/        migrate, seed, crawl, crawld, worker, dlq, eval, registry, takedown, …
│   ├── xustive-core/       Document/Comment/Source types, config, SafeUrl, circuit breaker, errors
│   ├── xustive-federation/ query-time federation client (the API's side of the gateway)
│   ├── xustive-federator/  the Federation Gateway binary — the one allowlisted egress hop
│   ├── xustive-ingest/     fetch, robots, frontier, orchestrator, parse, sitemap, discovery, SERP
│   ├── xustive-knowledge/  entity panels: Wikidata harvest, the knowledge index
│   ├── xustive-lang/       language detection, transliteration, query expansion
│   ├── xustive-loadgen/    `make load` — the load generator
│   ├── xustive-media/      image fetch, OCR (tesseract in-process, Unlimited-OCR sidecar client)
│   ├── xustive-ml/         llama.cpp engine, prompts, translation, device selection, model registry
│   ├── xustive-queue/      Redis Streams, indexer worker, dead-letter queue, shared breaker
│   ├── xustive-search/     Meilisearch client, ranking, query operators
│   ├── xustive-text/       ★ shared normalisation — query AND index time
│   ├── xustive-toold/      scheduled fetch of external tool data (weather, knowledge)
│   ├── xustive-tools/      instant answers: calculator, units, prayer, fuel, …
│   └── xustive-vector/     Qdrant client, CLIP and bge-m3 embedder clients
├── web/                    Next.js app: app/ (incl. (operator)/admin, /bot, api/thumb), components/, lib/
├── services/               the Python sidecars: ocr-sidecar, clip-embed, stt-sidecar, text-embed; searxng/settings.yml
├── config/                 dev.toml, ci.toml, staging.toml, prod.toml
├── data/                   seeds, registry, authority, parser rules, lexicons, knowledge seeds
│   ├── models/             ⊘ gitignored — STT weights fetched by hand (§9)
│   ├── geoip/              ⊘ gitignored — DB-IP City Lite, scripts/fetch-geoip.sh
│   └── tessdata/           ⊘ gitignored — tesseract language data
├── models/                 ⊘ gitignored except LICENSES.md — summariser GGUFs, scripts/fetch-models.sh
├── deploy/                 docker-compose(.dev).yml, Dockerfile.toold/.federator, prometheus/, grafana/
├── eval/                   golden query sets, reports/ (gitignored), the relevance harness
├── scripts/                dev.sh, dev-down.sh, reset.sh, backup.sh, restore.sh, fetch-*.sh, lints, budgets
├── .githooks/              pre-commit — fmt, telemetry, compose and docs lints
└── tests/fixtures/         site/ (the offline crawler fixture), corpus/, pages/, serp/
```

`xustive-text` carries a star because it quietly holds the system together: if query-time and
index-time normalisation ever diverge, search silently stops matching Arabic
([[Content Parser]] §4.4, [[Query Pipeline]] §4.1).

---

## 5. Configuration

Layered: built-in defaults → `config/{env}.toml` → `XUSTIVE_*` environment variables → CLI flags.

Each file declares its own `environment`, and the safety guards key off that rather than the
filename — a guard that decides how careful to be by parsing a path stops working the day someone
renames a file.

```toml
environment = "dev"

[api]
bind_addr = "0.0.0.0:8080"
timeout_search_ms = 2500       # the whole-request deadline; the ladder's floors are fractions of it

[queue]
url = "redis://127.0.0.1:6390"          # frontier, streams — persistent
signals_url = "redis://127.0.0.1:6391"  # interaction counters — the ephemeral instance

[search]
meili_url = "http://localhost:7700"
timeout_ms = 1200              # one Meilisearch round trip; must stay inside timeout_search_ms

[federation]
enabled = false                # the runtime switch lives on /admin/integrations
federator_url = "http://127.0.0.1:8095"
budget_ms = 900                # the "from the web" strip wait; Config::validate() refuses > timeout_search_ms

[stt]
enabled = true                 # the voice button answers honestly: "unavailable" until the sidecar is up
endpoint = "http://127.0.0.1:8093/transcribe"
```

### The timing ladder (BUG-041)

Four numbers have to agree, and three of them are in different files:

| Knob | Where | Dev value | Rule |
|:---|:---|---:|:---|
| `search.timeout_ms` | `config/dev.toml` | 1200 | one Meilisearch call; ~65 ms idle, several hundred while it indexes a backlog |
| `federation.budget_ms` | `config/dev.toml` | 900 | strip wait; `< api.timeout_search_ms` or the API refuses to start |
| `api.timeout_search_ms` | `config/dev.toml` | 2500 | the request deadline; stages degrade (drop the strip, narrow) before it expires |
| `SEARCH_GRACE_MS` | `crates/xustive-api/src/lib.rs` (constant) | 1000 | added to the above for the HTTP-layer timeout, so the handler's own degraded answer wins over a bare 504 |
| `experimental.proxyTimeout` | `web/next.config.ts` | 90 000 | the Next rewrite proxy; must exceed `ml.deadline_ms` + 10 s or a slow summary collapses to nothing |

`config/prod.toml` still carries `timeout_search_ms = 1500`; the dev value was raised so searches
narrow under indexing load instead of failing. Raise `search.timeout_ms` and `api.timeout_search_ms`
together — the first inside the second.

### Environment overrides that matter in development

| Variable | Read by | Effect |
|:---|:---|:---|
| `XUSTIVE_DEVICE=gpu\|cpu\|auto` | API | overrides `[ml] device`; `dev.sh` sets it from the build it chose |
| `XUSTIVE_FEDERATION_ENABLED=true` | API | arms federation at start; `dev.sh --federation` sets it |
| `XUSTIVE_API_URL` | web | where the `/api/v1/*` rewrite goes (default `http://127.0.0.1:8080`) |
| `XUSTIVE_THUMB_SECRET` | web | pins the thumbnail-signing key across restarts and replicas ([[Operating Xustive]] §5) |
| `XUSTIVE_QHASH_SALT` / `[interaction] salt` | API | required outside `dev`; keys the query hash in counter keys (BUG-036) |
| `RUST_LOG` | all Rust processes | log filter; the running API can also be changed live via `POST /api/v1/admin/log-level` |
| `MEILI_MASTER_KEY`, `MEILI_KEY` | compose, API | empty in dev; scoped keys via `xustive-cli keys --show` |
| `SEARXNG_SECRET`, `XUSTIVE_EXTERNAL_LLM_{URL,MODEL,KEY,KEY_FILE}` | compose (federation profile) | SearXNG's secret and the optional external summariser, which lives on the gateway so the API never holds the key |

Secrets never live in `config/*.toml`. `.env` holds them locally and is gitignored; a mounted
secrets file holds them in production ([[Security and Privacy]] §7).

---

## 6. Everyday commands

`make help` is the authoritative list — it is generated from the Makefile and cannot drift.

| Command | Does |
|:---|:---|
| `make dev` | everything, one terminal (`ARGS=` passes `--fast --no-crawler --cpu --federation`) |
| `make dev-stop` | stop a `make dev` from elsewhere |
| `make dev-up` / `dev-down` | infrastructure only; `dev-down` also frees 8080/3000 and keeps the data; `CLEAN=1` deletes volumes (asks) |
| `make up` | `dev-up` + corpus + seed, then says what to start next |
| `make reset` | stop everything and **delete all crawled data** (§13) |
| `make migrate` / `migrate-check` | apply index settings; report drift |
| `make corpus` / `seed` | generate and index the sample corpus |
| `make crawl` | one-shot crawl of the seed list |
| `make crawld` | the continuous crawler (`ARGS=--reset` starts from an empty frontier; `--discover` follows off-seed hosts) |
| `make worker` | drain the index queue |
| `make toold` | one pass of the tool-data fetcher (weather, knowledge) |
| `make search Q='…'` / `text Q='…'` | search and normalisation from the terminal |
| `make stats` | index document counts |
| `make eval` / `eval-check` | relevance harness; fail on regression vs `eval/reports/baseline.json` |
| `make eval-ab` / `calibrate` / `mine-synonyms` / `golden` | settings A/B; side-weight calibration against SearXNG; synonym candidates for review; regenerate the golden set |
| `make dlq A=stats\|peek\|replay` | dead letters |
| `make backup [DEST=dir]` | snapshot Meili + Qdrant + Redis + registry ([[Operating Xustive]] §7) |
| `make restore-drill SRC=backups/<ts> CONFIRM=yes` | restore from a backup — **staging only** |
| `make load S=search\|suggest\|summary\|mixed [RPS= DUR=]` | load-test the running API |
| `make test` / `lint` / `check` | test and lint tiers ([[Testing Strategy]]) |
| `make audit` | `cargo deny` advisories, licences, bans, sources |
| `make smoke` | end-to-end checks against a running API on :8080 |
| `make scan-logs LOG=…` | scan a log file for query text that should never be there |
| `make egress-test` | prove the serving plane cannot reach the internet |
| `make ui-gates` | client asset budgets, no-JavaScript path, RTL icons, contrast (needs web on :3000) |
| `make fixture-site` | the offline crawler fixture site on :8099 |
| `make web` / `web-build` | open the UI; production build then `next start` |

The CLI has more than the Makefile wraps. Run any of these as
`cargo run -q -p xustive-cli -- --config config/dev.toml <cmd>` (or `target/release/xustive-cli`):
`registry {list,stats,lint,fmt,approve,activate,disable}`, `keys --show`, `reindex [--rollback]`,
`takedown --domain … [--yes]`, `media-repass`, `reconcile-vectors`, `common-crawl`, `discover`,
`page-rank`, `parse-check`, `eval-serp`, `score-transcripts`, `worker --once`.
`cargo run -p xustive-cli -- --help` is the list that cannot drift.

### The git hook and the lints

`make setup` points `core.hooksPath` at `.githooks/`. The **pre-commit** hook runs only what
finishes in seconds: `cargo fmt --check`, `scripts/lint-telemetry.sh` (no query or credential
field in a `tracing::` call), `scripts/lint-compose.sh` (the base compose file publishes no port,
`core`/`obs` stay `internal`, every service has a `mem_limit`) and `scripts/lint-docs.sh` (a
documented `make` target or `scripts/*.sh` must exist). `make lint` adds clippy,
`scripts/check-alerts.sh` (`promtool test rules` on the alert file), `scripts/lint-runbooks.sh`
(every alert has a `## Name · severity` section in [[Runbooks]], and no orphan section) and
`scripts/lint-bidi.sh`. CI runs the same set plus the egress test and the smoke suite.

---

## 7. Working on one part

| Working on | Run |
|:---|:---|
| Frontend | `make dev ARGS="--fast --no-crawler"` — no summariser, no crawl noise |
| Ranking | `make dev ARGS=--fast`, then `make eval` |
| Parser | `make fixture-site`, then `cargo test -p xustive-ingest` |
| Crawler | `make dev-up`, `make crawld`, and `make fixture-site` for offline cases |
| Summariser | `make dev` (needs the model) — see §9 |
| Federation | `make dev ARGS=--federation`, then the Integrations page — see §10 |
| Voice | `services/stt-sidecar/run.sh` in its own terminal — see §10 |
| Instant answers | `cargo test -p xustive-tools`; no infrastructure at all |

Most of the test suite needs nothing running. The exceptions are the `*_redis.rs` suites in
`crates/xustive-ingest/tests/` (frontier, robots cache, dedup, raw store, breakers…), which
**skip rather than fail** when Redis is absent on :6390.

---

## 8. Working offline

The whole thing runs with no internet:

- `make corpus` generates ~10 000 sample documents.
- `make fixture-site` serves a local site reproducing redirect chains, legacy encodings, robots
  directives, traps and malformed markup.
- The models are on disk once fetched (§9).

Only `make crawld`, `make crawl`, `make toold`, the federation containers and the `fetch-*`
scripts need the network, because they are the parts that talk to the outside.

---

## 9. Models and data that live outside git

Weights are gigabytes, change independently of the code and carry their own licences, so none of
them are committed. Each has a fetch path, and each directory is gitignored:

| What | Where | How to get it | Licence |
|:---|:---|:---|:---|
| Summariser GGUFs | `models/` | `scripts/fetch-models.sh [default\|all\|<id>]` | see below — **the default is non-commercial** |
| Whisper weights (STT) | `data/models/faster-whisper-small/`, `…-base/` | by hand with `curl` (below) | MIT |
| IP-to-city database | `data/geoip/dbip-city-lite.mmdb` | `scripts/fetch-geoip.sh [YYYY-MM]` | CC BY 4.0 — attribution is on the weather card |
| tesseract language data | `data/tessdata/` (`ara`, `fra`, `eng`) | copy from a tesseract install | Apache-2.0 |
| Sidecar weights (OCR, CLIP, bge-m3) | the `*_models` compose volumes | provisioned out-of-band; the containers run `HF_HUB_OFFLINE=1` | per `models/LICENSES.md` |

`models/LICENSES.md` is the audit for all of them and is the one file under `models/` that *is*
committed. Nothing above is needed to search: without the summariser there are no AI summaries,
without geoip the weather card never guesses a place, without whisper the microphone says
"unavailable".

### The summariser

It is optional and it is the expensive part: a local Qwen2.5 GGUF through llama.cpp, and a first
build that compiles llama.cpp from source. `make dev ARGS=--fast` skips it entirely.

`scripts/fetch-models.sh` fetches `qwen2.5-3b-instruct-q4_k_m.gguf` by default (`all` adds the
1.5B). `[ml] summariser_model = ""` means "the first present file in `models/`", or pin an id from
the registry in `crates/xustive-ml/src/registry.rs`.

> [!warning] Licence: the default 3B model is research-only
> Qwen2.5-**3B** is released under the `qwen-research` licence — **non-commercial** — unlike the
> 0.5B/1.5B/7B sizes, which are Apache-2.0. It is fine for local evaluation, which is what a
> development box does. Before any commercial deployment pin an Apache-2.0 size
> (`summariser_model = "qwen2.5-1.5b-instruct-q4"`, or fetch the 7B for quality on a bigger card)
> and remove the 3B file. `models/LICENSES.md` records the finding; [[Legal and Compliance]] §7 owns it.

**GPU or CPU.** GPU support is a *build-time* decision — the `cuda` feature needs `nvcc` — which is
why `make run-api` and `dev.sh` detect `/opt/cuda/bin/nvcc` rather than reading config. With it
compiled in, `[ml] device = auto|gpu|cpu` (or `XUSTIVE_DEVICE`) picks the device, and
`/admin/compute` switches it live; the change takes effect on the next model load. Without it,
CPU-only and the compute page says why. A CPU-only build is fully functional, only slower: a
summary takes ~20–30 s on CPU against a few seconds on the card, which is what the 90 s proxy
timeout in §5 exists for.

**The 4 GB budget.** On the reference development card (Quadro T1000, 4 GB) the residents are,
measured with `nvidia-smi` on 2026-08-27:

| Process | VRAM |
|:---|---:|
| `xustive-api` (Qwen2.5-3B Q4, all layers offloaded) | ~1.6 GB |
| STT sidecar (`small` int8_float16 + `base` float32) | ~0.75 GB |
| the desktop | ~0.5 GB |

That leaves headroom for one of them to grow, not both. The 7B model does not fit — run it on CPU
or a bigger card — and `gpu_layers` is the knob for a partial offload when something else needs
the card. The STT sidecar falls back to its light model when its final pass hits an OOM.

---

## 10. Optional services

Everything below is off by default and the product works without it. Each is a compose profile
(or, for voice in development, a host process), a config switch, and for two of them a runtime
toggle on the admin console.

| Feature | Start | Config | Runtime switch |
|:---|:---|:---|:---|
| Query-time federation (SearXNG + gateway) | `make dev ARGS=--federation`, or `docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.dev.yml --profile federation up -d` | `[federation] federator_url` (set by default) | **`/admin/integrations`** — `enabled` flips live, no restart |
| External AI summariser | the gateway's `XUSTIVE_EXTERNAL_LLM_*` env | `[ml] external_summaries` | `/admin/integrations` — **third-party**: query text leaves the box when on |
| Voice search (STT) | `services/stt-sidecar/run.sh` (dev) or `--profile voice` | `[stt] enabled = true` (already on in dev) | none — the button says "unavailable" until `/health` on :8093 is 200 |
| Semantic text search | `--profile semantic` + bge-m3 weights in `text_models` | `[vector] text_enabled` | none |
| Image similarity | `--profile vector` + CLIP weights | `[vector] enabled` | none |
| Unlimited-OCR | `--profile ocr` (needs an 8 GB GPU) | `[media] ocr_backend = "unlimited"` | none; falls back to tesseract when down |

### SearXNG and the gateway

SearXNG runs on the egress `ingest` network only, with `logging: driver: "none"` — its entire
traffic is query text and it logs upstream URLs on failure, so its stdout is never kept. The
gateway (`xustive-federator`, :8095, routes `/federate`, `/summarise`, `/healthz`) is the *only*
thing the serving API may call outbound. `services/searxng/settings.yml` enables JSON output and
turns the public rate limiter off; set `SEARXNG_SECRET` in `.env` on anything that is not a
laptop. With `dev.sh --federation` the API starts armed (`XUSTIVE_FEDERATION_ENABLED=true`);
otherwise the switch is on the Integrations page, which refuses to arm federation when no gateway
is configured, and shows the gateway's reachability and breaker state.

### The STT sidecar on the host (development)

The compose service exists (`--profile voice`, CPU), but on a machine with the card the sidecar
runs on the host so it can use it. `services/stt-sidecar/README.md` is the reference; the short
version:

```bash
cd services/stt-sidecar
uv venv -p 3.12 .venv && uv pip install -p .venv/bin/python -r requirements.txt   # 3.14 has no PyAV wheel
uv pip install -p .venv/bin/python nvidia-cublas-cu12 "nvidia-cudnn-cu12>=9,<10"     # GPU only
./run.sh
```

- **Python 3.12 via `uv`**, because PyAV has no 3.14 wheel yet.
- **CUDA 12 runtime from pip wheels** (`nvidia-cublas-cu12`, `nvidia-cudnn-cu12<10`): CTranslate2
  is built against CUDA 12 and the host's CUDA 13 does not match. `run.sh` puts the wheels' `lib/`
  directories on `LD_LIBRARY_PATH` and uses the GPU when CTranslate2 reports one, else CPU int8.
- **Weights as directories**: `STT_MODEL` and `STT_PARTIAL_MODEL` default to
  `data/models/faster-whisper-small` and `…-base`. Fetch `config.json`, `model.bin`,
  `tokenizer.json`, `vocabulary.txt` with `curl` from
  `https://huggingface.co/Systran/faster-whisper-{small,base}/resolve/main/` — the hub client's
  unauthenticated download throttled to ~60 KB/s on 2026-08-27 while `curl` ran at 6 MB/s.
- On the T1000, `float32` beats `float16` for the encoder (128 ms vs 435 ms), so `run.sh`
  defaults the partial model to float32 and the final model to `int8_float16`; override with
  `STT_COMPUTE` / `STT_PARTIAL_COMPUTE` on a card with real FP16 throughput.

---

## 11. Troubleshooting

Every entry is a problem actually hit, not a hypothetical.

| Symptom | Cause | Fix |
|:---|:---|:---|
| `localhost:8080` shows nothing; `:8080/admin` is a 404 | 8080 is the JSON API | the site **and** the console are on **:3000** |
| "site can't be reached" right after starting | the first build compiles llama.cpp, minutes | wait, or `make dev ARGS=--fast` |
| `make dev` refuses to start | 8080 or 3000 already held | `make dev-down`, or find the owner with `ss -ltnp \| grep -E ':8080\|:3000'` |
| Code changes have no effect | the old binary is still the running process — nothing here hot-reloads | rebuild **and restart** that process (§1); a stale pidfile will not save you |
| "That search took too long" while the crawler is busy | Meilisearch is indexing a backlog and the search timeout was too tight (BUG-041) | the dev ladder in §5 is sized for it; if it recurs, check `xustive_search_duration_seconds{stage}` before widening anything |
| The summary box appears, then collapses to nothing | the Next proxy hung up before the CPU summary finished | `experimental.proxyTimeout` in `web/next.config.ts` must exceed `ml.deadline_ms` + 10 s (it is 90 s) |
| `Config::validate` refuses to start: federation budget | `federation.budget_ms` ≥ `api.timeout_search_ms` | lower the budget or raise the deadline |
| `Bind for 0.0.0.0:6379 failed` | another local stack owns Redis | set `XUSTIVE_REDIS_PORT` in `.env` |
| `curl: (7) … localhost:7700` | you started the base compose file alone, whose networks are `internal: true` | `make dev-up`, which includes the dev override |
| `readyz` returns 503 | Meilisearch not up | `make dev-up`, then `curl -s localhost:7700/health` |
| Arabic query returns nothing | normalisation mismatch | `make text Q='…'` and compare with what is indexed |
| A synonym edit has no effect | index settings not reapplied | `make migrate` |
| Photos on relation cards are broken for five minutes after a restart | the thumbnail signature key is per process unless pinned | set `XUSTIVE_THUMB_SECRET` ([[Operating Xustive]] §5) |
| The microphone says "unavailable" | the STT sidecar is not up, or its breaker is open | `curl localhost:8093/health`; start `run.sh`; the breaker closes on the next successful probe |
| Voice on GPU: `libcublas.so.12: cannot open` | the host CUDA 13 libraries do not match CTranslate2 | install the CUDA 12 pip wheels into the venv (§10) |
| "From the web" strip never appears | federation off, or the gateway unreachable | `/admin/integrations`: `enabled`, `reachable_from_api`, `breaker` |
| `make dev-down` says nothing running but 3000 is busy | a `next start` from a shell, not from `dev.sh` | `dev-down` frees the port with `fuser` anyway; check `ss -ltnp` |
| A sidecar container survives `docker compose down` | profiled services are skipped by a plain `down` | `COMPOSE_PROFILES='*'` — `dev-down` and `reset` already do this |
| `docker compose` transfers gigabytes | a missing `.dockerignore` — fixed | pull latest |
| Crawler says `robots.txt unreachable` for a `.gov.dz` host | that server omits its certificate intermediate; `curl` fails too | not ours to fix — M2-T04.10 |
| Meilisearch idles with hundreds of tasks it never processes (`Can't access update file`) | its task DB and update files diverged on disk | `make reset` — nothing in our code recovers that |
| `make audit` fails immediately | `cargo-deny` not installed | `cargo install cargo-deny` |
| `make smoke` skips the log-scan section | it reads `/tmp/xustive-api.log`, which `make dev` does not write | run the API with `… 2>&1 \| tee /tmp/xustive-api.log`, as CI does |

### Reading the logs

```bash
RUST_LOG=debug make run-api                    # everything
RUST_LOG=xustive_ingest=debug make crawld      # one crate
```

The running API's filter can be changed without a restart:
`curl -X POST localhost:8080/api/v1/admin/log-level -H 'content-type: application/json' -d '{"filter":"info,xustive_search=debug"}'`
(omit `filter` to revert), which is also on `/admin/config`.

What you will **not** find in the logs is **query text**. That is deliberate, and enforced by
`scripts/lint-telemetry.sh` plus a canary in the smoke suite
([[ADR-0008 - No Query Logging]]). To debug a specific query, reproduce it with `make search Q='…'`
rather than looking for it in a log file. SearXNG's container has no log driver at all for the
same reason.

---

## 12. Metrics

Prometheus scrapes the API at `host.docker.internal:8080` and Meilisearch at `meilisearch:7700`
(`deploy/prometheus/prometheus.yml`); the alert rules are `deploy/prometheus/alerts.yml`, each
with a section in [[Runbooks]]. Grafana is on :3001, `admin`/`admin` (set in the compose file, not
`.env`).

```bash
curl -s localhost:8080/metrics | grep xustive_
```

`xustive_lang_detected_total` is worth watching as a product signal rather than an operational
one: if the `ary` share sits near zero, either detection is broken or the assumption about who uses
this is wrong ([[Language Detector]] §9).

`xustive_data_age_seconds` is the one that catches a silently dead tool fetcher — see
[[Observability]] §6.

---

## 13. Stopping and resetting

```bash
make dev-stop     # stop the processes, leave the containers
make dev-down     # stop the containers (all profiles), free 8080/3000, keep the data
make dev-down CLEAN=1   # …and delete the volumes (asks for 'yes')
make reset        # stop EVERYTHING and DELETE every trace of crawled data
```

`make reset` is the one to reach for when the engine is in a state you no longer trust — a
stalled index, a frontier full of junk, counters that sit at a fixed number. It stops the
application processes *before* removing the volumes (a crawler still running while Redis is
recreated writes a frontier into the fresh instance, so a "clean" slate is dirty before you have
looked at it), then deletes:

- the search index,
- the crawl frontier — every discovered URL,
- the index queue and its dead letters,
- cached robots rules and tool data,
- the sidecar model volumes, because `-v` takes every profile's volumes with it.

Your seeds, config, code, `models/` and `data/models/` are kept (they are host directories, not
volumes). It prints how many documents you are about to lose and asks you to type `delete`; pass
`--yes` in a script (`./scripts/reset.sh --yes`). Afterwards `make up` rebuilds from the sample
corpus — anything crawled is gone, and re-crawling costs the *sites* bandwidth, not just you.

`dev-reset` remains as an alias. Backups and the restore drill are in [[Operating Xustive]] §7.

---

## 14. Related

[[Operating Xustive]] · [[Runbooks]] · [[Testing Strategy]] · [[Deployment Topology]] ·
[[Observability]] · [[Crawler Orchestrator]] · [[Politeness and Robots]] · [[Security and Privacy]]
