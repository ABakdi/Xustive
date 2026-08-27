---
tags:
  - operations
  - ops
type: operations
status: living
updated: 2026-08-27
---

# Operating Xustive

> The routine, non-alert side of running the engine: what is up, how to tell, how to restart it,
> what the console pages are for, the secret that has to be set, and the machinery that protects
> the serving path when a dependency goes away. [[Runbooks]] is the *alert* side — one section per
> configured rule — and deliberately holds nothing else, because a lint keeps it in step with
> `deploy/prometheus/alerts.yml`. Everything an operator does *between* alerts is here.
>
> Commands assume the development topology in [[Running Xustive]] §3: containers via compose, the
> Rust binaries and the frontend as host processes. On a containerised deployment substitute the
> orchestrator's restart for "stop the pid and start the binary".

---

## 1. What runs, and how to tell it is up

| Process | How it runs | Health |
|:---|:---|:---|
| `xustive-api` (:8080) | host: `target/debug/xustive-api --config config/dev.toml` | `GET /healthz` → `ok` (process alive); `GET /readyz` → `ready`, or **503** when Meilisearch is unreachable |
| `xustive-web` (:3000) | host: `npx next start -p 3000` (after `npm run build`) | `GET /` → 200; it proxies `/api/v1/*` to the API |
| index worker | host: `target/release/xustive-cli --config config/dev.toml worker` | `/admin/queue` depth falls; `xustive_queue_depth` |
| crawler | host: `target/release/xustive-cli --config config/dev.toml crawld` | `/admin/live` — `fetched` moves, `state` is `running` |
| `toold` | container `xustive-toold` (compose, always on) | `xustive_data_age_seconds{dataset}` — see the ToolData* and Knowledge* runbooks |
| Meilisearch (:7700) | container | `curl -fsS localhost:7700/health` |
| Redis (:6390) / signals (:6391) | containers | `docker exec xustive-redis redis-cli ping` (and `xustive-redis-signals`) |
| Qdrant (:6333) | container | `curl -fsS localhost:6333/healthz` |
| STT sidecar (:8093) | host `services/stt-sidecar/run.sh`, or `--profile voice` | `GET /health` → 200 when the models are loaded, else 503 |
| Federation gateway (:8095) | container, `--profile federation` | `GET /healthz`; the API's own probe is `reachable_from_api` on `/api/v1/admin/integrations` |
| OCR / CLIP / text-embed (:8091/:8092/:8094) | containers, profiles `ocr` / `vector` / `semantic` | `GET /health` on each |
| Prometheus (:9090), Grafana (:3001) | containers | the dashboards; Grafana is `admin`/`admin` |

A one-line sweep of everything that matters:

```bash
for u in 8080/healthz 8080/readyz 3000/ 7700/health 6333/healthz 8093/health 8095/healthz; do
  printf '%-14s ' "$u"; curl -s -o /dev/null -m 3 -w '%{http_code}\n' "http://localhost:$u"; done
docker ps --format '{{.Names}} {{.Status}}'
ss -ltnp | grep -E ':(8080|3000|8093)\b'
```

`readyz` is the one to trust: `healthz` says the process exists, `readyz` says it can answer a
search. The load balancer removes a replica on `readyz`, not `healthz`.

---

## 2. Restarting things

**Nothing here hot-reloads.** A rebuilt binary or a fresh `next build` is not running until the
old process is stopped and the new one started. This is the single most common cause of "the fix
did nothing" on a development box ([[Running Xustive]] §1).

| To restart | Do |
|:---|:---|
| the whole `make dev` stack | Ctrl-C in its terminal, or `make dev-stop` from another; then `make dev` again |
| the API only | find the pid with `ss -ltnp \| grep :8080`, `kill -TERM` it — it drains in-flight requests for up to **25 s** (M4-T02.7) then exits — and start it again. The worker and crawler keep going; they do not depend on the API being up |
| the worker or crawler | `kill -TERM` the `xustive-cli … worker` / `… crawld` pid. Both stop cleanly: the worker leaves an in-flight batch unacked for the next worker, the crawler finishes its fetch. `crawld` resumes from the shared frontier on restart — add `--reset` only to start from the seed list again |
| the frontend | `kill -TERM` the `next-server` pid on :3000, `npm run build` if code changed, `npx next start -p 3000` |
| a container | `docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.dev.yml restart meilisearch` (or `redis`, `toold`, …). Meilisearch takes a few seconds to come back; `readyz` on the API flips to 503 and back by itself |
| the STT sidecar | kill the `uvicorn` pid on :8093, `services/stt-sidecar/run.sh`. The API's breaker to it re-closes on the next successful probe (§6) |
| federation | `docker compose … --profile federation restart federator searxng`; the runtime switch on `/admin/integrations` survives, the breaker re-closes on the next good call |
| everything, and free stuck ports | `make dev-down` — stops every profile's containers, `fuser -k`s 8080 and 3000, keeps the data |

Prefer stopping by pid (from `ss -ltnp`) over a broad `pkill -f xustive`, which matches more than
you mean — a shell whose command line mentions the binary, an editor with the file open.

Restart order when starting cold: containers (`make dev-up` waits for Meilisearch) → the API (it
owns the index settings and applies them at start) → worker → crawler → web. `dev.sh` does
exactly this and waits on `/healthz` between the first two.

---

## 3. The admin console

The console is the Next.js app at **`http://localhost:3000/admin`** — not `:8080/admin`, which
is a 404 since the Rust renderer went. The API's side is JSON under `/api/v1/admin/*`, which every
page reads; `curl` any of those when a page is blank and you want to know whether it is the data or
the page. Nothing here is authenticated in development; in production the console sits behind the
operator network and the API's `admin/*` routes are not published.

| Page | Shows | Operator actions |
|:---|:---|:---|
| `/admin` | the overview: device, model, index counts, versions | — |
| `/admin/live` | the crawler, live over one SSE stream (`/crawler/events`): `waiting`, `inflight`, `fetched`, `parsed`, `indexed`, `failed`, `skipped`, `deferred`, `revisited`, `discovered`, per-host rows, and since M9 the **`images found` / `videos found`** tiles and a `N img · M vid` note per recent page | **Pause / resume** the crawler (`POST /crawler/pause`); enqueue a URL |
| `/admin/crawler` | redirects to `/admin/live` — the old URL, kept for bookmarks | — |
| `/admin/documents` | what is indexed, filterable by text, host, language, **channel** (SearXNG-fed first, then the crawler's own discovery) and **media** — the `image` / `video` drill-in from the `media.type` facet, so "pages with videos" is one click | — |
| `/admin/queue` | the index queue depth and the **dead letters** with their rejection reasons | **Replay all** (`POST /queue/replay`), replay one, drop one — replay only after fixing the cause (see DLQGrowth in [[Runbooks]]) |
| `/admin/integrations` | federation (enabled, gateway reachable from the API, **breaker state**, budget, hits/empty counters, URLs fed), the external summariser (configured, enabled, attempts), image and semantic search | **Switch federation on/off live**; switch the external summariser — the latter is third-party and sends query text out, and the page says so |
| `/admin/evaluation` | the quality trail: every eval, A/B, calibration and SERP-yardstick report with per-language nDCG | — (reports are produced by `make eval` and friends) |
| `/admin/compute` | the device in use (`active`), whether GPU was asked for and refused (no CUDA build, or not enough VRAM), `gpu_layers` | **Switch CPU / GPU** — takes effect on the next model load, not mid-request |
| `/admin/media` | the OCR engines and sidecars, up/down | — |
| `/admin/sources`, `/admin/sources/health` | the registry (`data/sources/registry.jsonl`) by category and region; per-source quality signals against the §7 bands of [[Data Sources Registry]] | approve / activate / disable a source; edit its policy |
| `/admin/discovery`, `/admin/weak-coverage` | the discovery channels and the weak-coverage terms waiting to be resolved | forget a weak-coverage term |
| `/admin/interaction` | the anonymous interaction signals (k-anonymity floor, window, counts) | — |
| `/admin/maintenance` | the takedown control | **Domain takedown** — previews, then requires the exact domain typed to execute; mirrors `xustive-cli takedown` |
| `/admin/config` | the effective configuration, read-only, and the live log filter | change the `tracing` filter without a restart (`POST /log-level`) |

`/bot` — the public crawler page site owners are pointed at by the user-agent — is at
`http://localhost:3000/bot`.

---

## 4. Operating the crawler

- **Pause** from `/admin/live` before anything that changes what the crawler would do (a registry
  edit, a `migrate`, a reset). The crawler also pauses *itself* when Redis passes 85 % of
  `maxmemory` and when the index queue backs up (PROB-001) — the `paused` flag on `/crawler/status`
  says which.
- **Frontier bounds** are enforced (`frontier_max_urls` 200 000, `max_pages_per_host` 20 000,
  `max_outlinks_per_page` 64, `seen_rotate_days` 45 — `[crawl]` in `config/dev.toml`). At the
  ceiling the worst-priority tail is evicted; a frontier that never shrinks is not a bug here.
- **Sources** are `data/sources/seeds.tsv` (bootstrap) plus the approved, active rows of
  `data/sources/registry.jsonl`, both read at `crawld` start — a registry change needs a crawler
  restart or the console's enqueue to take effect now.
- **Media**: a parser change only reaches a page on its next visit. `xustive-cli media-repass`
  re-extracts `media[]` from the raw store for pages still within `raw_ttl_days`, without
  refetching.

---

## 5. `XUSTIVE_THUMB_SECRET`

Result images are never loaded from the source host by the browser — they go through the
frontend's `/api/thumb` proxy, and every proxied URL carries an HMAC so the proxy cannot be used
as an open relay ([[ADR-0021 - Proxied Thumbnails with Signed URLs]]). The key is
`XUSTIVE_THUMB_SECRET`, read by **the Next.js process** (`web/lib/thumb.ts`), not the API.

- **Unset**, the frontend generates a random key at start and holds it on `globalThis`. That
  works for one process — until it restarts: a relation row the browser cached for five minutes
  still carries the old signatures, and every photo is a 403 until the cache turns over. Two
  frontend replicas with two random keys refuse each other's signatures permanently.
- **Set it** on anything with more than one replica or a restart cadence you notice. Any long
  random string; it is compared in constant time:

  ```bash
  XUSTIVE_THUMB_SECRET="$(openssl rand -base64 48)"   # into the frontend's environment, not .env.example
  ```

- **Rotating** it invalidates every signature in flight; expect up to five minutes of broken
  photos (the relation-card cache TTL) and nothing else. Rotate by restarting the frontend with
  the new value — there is no live reload of the key. Wikimedia's image hosts and Open Library
  covers are exempt from the signature (the host itself is the gate), so entity-panel photos
  survive a rotation; crawled-page thumbnails do not.
- The failure when it is wrong is always **visible and harmless** — 403 on an image — never an
  open proxy. The proxy still refuses non-HTTPS, IP literals, credentials in the URL and private
  names regardless of the signature.

---

## 6. Circuit breakers

Every optional dependency the *serving* path calls sits behind a breaker, so a sidecar that is
down costs one probe per cooldown, not a timeout per search. All of them follow
`xustive_core::circuit`: **closed** (normal) → **open** after `failure_threshold` consecutive
failures, the call is refused immediately → **half-open** after `cooldown`, one probe goes
through; success closes it, failure re-opens with the cooldown doubled up to `max_cooldown`.

| Breaker | Around | Threshold | Cooldown → max | Where to see it |
|:---|:---|---:|:---|:---|
| STT | the transcription sidecar | 3 | 5 s → 60 s | the voice button says "unavailable"; `/api/v1/admin/status` |
| federation (`/federate`) | the gateway | 3 | 5 s → 60 s | `breaker` on `/admin/integrations` |
| external summariser (`/summarise`) | the gateway | 3 | 5 s → 60 s | same page |
| text embedder | the bge-m3 sidecar | 3 | 10 s → 120 s | `semantic` block on `/admin/integrations` |
| indexer sink (shared) | Meilisearch writes, from every worker | 5 | 2 s → 60 s (30 s window) | the worker log; `xustive_queue_depth` flat while workers run |
| proxy / platform (crawler) | per-proxy and per-platform fetch, shared via Redis | per tier | escalating | the crawler log; [[Proxy Manager]] |

Two things worth knowing when one is open:

1. **There is no manual reset.** The half-open probe is the reset; fix the dependency and the
   next probe closes it within one cooldown (≤ 60 s for the sidecars). Restarting the API also
   resets its in-process breakers, but that is the blunt way.
2. **A breaker whose Redis is down does not fail closed.** The shared indexer breaker treats "no
   store" as "always allow" — the breaker must never become the outage. If Redis is gone the
   worker fails on the queue itself, which is the honest error.

The search path proper (Meilisearch) has no breaker: it has a timeout (`search.timeout_ms`) inside
the request deadline (`api.timeout_search_ms`), and the deadline ladder degrades the answer —
drops the federation strip, narrows the query — before it expires (BUG-041). `readyz` is what
takes the replica out when Meilisearch is truly gone.

---

## 7. Backup, restore, reset

Three scripts, one destructive, one guarded, one that asks you to type the word.

### `make backup [DEST=dir]` — `scripts/backup.sh`

Non-destructive. Writes `backups/<UTC timestamp>/` with:

- `meili.snapshot` — `POST /snapshots` on Meilisearch, polled to completion, copied out of the
  container with `docker cp` (it looks in the usual snapshot directories and **warns** rather than
  shipping a backup with no Meili in it);
- `qdrant-<collection>.snapshot` — per collection (`QDRANT_COLLECTIONS`, default `image_clip`),
  downloaded over HTTP;
- `redis-dump.rdb` — `BGSAVE` on `xustive-redis`, waited on, copied out. **Only the queue Redis.**
  The signals instance is deliberately never backed up (ADR-0018): a backup of windowed
  identifier-free counters is a durable query log;
- `registry.jsonl` — a copy of the git-versioned registry;
- `manifest.txt`.

Overrides: `MEILI_URL`, `QDRANT_URL`, `MEILI_CONTAINER`, `REDIS_CONTAINER`, `REGISTRY`. It exits
0 with warnings when a store is absent, so read the manifest. Where the directory should
physically live, given data sovereignty, is [[Deployment Topology]] §7's open question.

### `make restore-drill SRC=backups/<ts> CONFIRM=yes` — `scripts/restore.sh`

**Staging only.** It refuses without `CONFIRM=yes` because it overwrites Qdrant collections
(`snapshots/upload?priority=snapshot`) and replaces `dump.rdb` then restarts the Redis container.
Meilisearch is **not** automated: a snapshot is imported at *startup*, so it prints the steps
(stop the node, place the file, start with `--import-snapshot`). The drill's pass/fail is the
verification it prints at the end — `xustive-cli stats` matches the backup's era, a search
returns, `reconcile-vectors --dry-run` lines up — and the wall-clock from wipe to a working
search is your RTO (M4-T04.6).

### `make reset` — `scripts/reset.sh`

Destroys everything the engine collected: the index, the frontier, the queue and dead letters,
robots and tool caches, the sidecar model volumes (it runs `down -v` with every profile). Keeps
seeds, registry, config, code, `models/` and `data/models/`. It stops the application processes
*first* (a crawler still running writes a frontier into the fresh Redis), reports the document
count it is about to discard, and asks you to type `delete`; `--yes` for scripts. Reach for it
when Meilisearch's task scheduler has lost its own update files — the failure it was written
for — and nothing in our code can recover. `make up` rebuilds from the sample corpus afterwards.

`make dev-down CLEAN=1` is the same volume delete without the process sweep and the document
count; `reset` is the one to use.

---

## 8. Logs and the privacy line

- `RUST_LOG` at start, or `POST /api/v1/admin/log-level {"filter":"…"}` on a running API (also on
  `/admin/config`); omit `filter` to revert.
- **No query text, ever.** `scripts/lint-telemetry.sh` blocks it at the source; `make scan-logs
  LOG=<file>` checks a log after the fact; the smoke suite fires a canary query and greps
  `/tmp/xustive-api.log` for it. SearXNG's container has `logging: driver: "none"` because it is
  third-party code that prints upstream URLs — its whole traffic is query text.
- During an incident the same rule holds: never paste a query into a channel. Reproduce with
  `make search Q='…'` instead ([[ADR-0008 - No Query Logging]]).

---

## Related

[[Runbooks]] · [[Running Xustive]] · [[Observability]] · [[Error Handling and Resilience]] ·
[[Security and Privacy]] · [[ADR-0021 - Proxied Thumbnails with Signed URLs]] · [[Deployment Topology]]
