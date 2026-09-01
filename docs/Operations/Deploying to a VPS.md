---
tags:
  - operations
  - deployment
status: built
date: 2026-09-01
---
# Deploying to a VPS

> One server, from a bare Ubuntu box to a search engine with a certificate. The topology is
> [[Deployment Topology]] §11 tier one; the community and multi-shard tiers are
> [[Milestone 14 - One Server, Many Hands]] and not built.

## What you need

| | Minimum | Comfortable | Why |
|:---|:---|:---|:---|
| vCPU | 4 | 8 | the crawler, the indexer and the API all want cores; the summariser wants more |
| RAM | 8 GB | 16–32 GB | **Meilisearch's limit must exceed its index** ([[Problems#PROB-004 — Indexing throughput decays as the index grows (260 → 10 documents a minute)\|PROB-004]]) — this is the number that decides how big a corpus the box can hold |
| Disk | 80 GB SSD | 200 GB+ | the index is roughly 40 GB per million documents with media |
| Ports | 80, 443 | | Caddy binds them; nothing else is published |
| DNS | an A/AAAA record pointing at the box | | the certificate is issued against it |

Without the summariser (`XUSTIVE_API_FEATURES=` empty) a 4 GB box serves search perfectly well;
summaries then say they are unavailable rather than being silently missing.

## First deployment

```bash
# 1. On the box: Docker, then the repository.
curl -fsSL https://get.docker.com | sh
sudo usermod -aG docker "$USER" && newgrp docker
git clone https://github.com/ABakdi/Xustive.git && cd Xustive

# 2. Secrets and settings.
cp .env.example .env
make secrets                 # prints XUSTIVE_ADMIN_KEY and MEILI_MASTER_KEY — paste them into .env
make admin-password          # asks for the console password, prints the hash for .env
$EDITOR .env                 # XUSTIVE_DOMAIN, XUSTIVE_ACME_EMAIL, and the three values above

# 3. Check the box is actually ready. It refuses rather than half-deploying.
make preflight

# 4. Build and start everything. The first build compiles llama.cpp — allow 15–30 minutes.
make deploy

# 5. Create the indexes, then give the crawler its seeds.
make deploy-migrate
make deploy-seed

# 6. Watch it come alive.
make deploy-logs
```

Then open `https://your-domain` for search, and `https://your-domain/admin` for the console —
the browser asks for the password you hashed in step 2.

## What is running, and what is exposed

```
                      ┌──▶ web (Next.js)  ── pages
internet ──443──▶ caddy
                      └──▶ api ──┬──▶ meilisearch   (network: core, no egress)
   basic auth on               ├──▶ redis
   /admin* and /api/v1/admin/*, └──▶ qdrant
   plus the API's key injected upstream

                        crawld ──▶ the open web    (network: ingest)
                        worker ──▶ meilisearch
                        toold  ──▶ weather / rates (network: ingest)
```

**The edge routes `/api/v1/*` straight to the API**, not through the web tier. A Next.js rewrite
compiles its destination into `routes-manifest.json` at *build* time, so an image built once and
deployed anywhere would proxy forever to whatever URL the builder happened to have — the smoke
test for this deployment found exactly that, as a 500 on every API call. One hop fewer, and the
web image no longer cares where the API lives.

**Only Caddy publishes a port.** Meilisearch, Redis, Qdrant and the API are unreachable from the
internet even if the firewall is wrong, because `core` is an internal Docker network with no
route out — which is what makes "your queries do not leave this box" structural rather than a
promise ([[ADR-0008]], as amended by [[ADR-0029]]).

**The admin surface has two locks.** Caddy asks the browser for a password; the API separately
requires its own key on every `/api/v1/admin/*` request, and **refuses to start on a public
address without one** ([[Security and Privacy]]). Losing either lock does not open the console.

## Day two

| Task | Command |
|:---|:---|
| Update to the latest code | `make deploy-update` (pull, rebuild, restart, migrate — no data loss) |
| See what is healthy | `make deploy-ps` |
| Tail everything | `make deploy-logs` |
| **Check the index still fits its memory** | `make index-size` — do this whenever the corpus doubles |
| Back up | `make backup DEST=/srv/backups` ([[Runbooks]]) |
| Stop | `make deploy-down` (volumes are kept) |

### The one number to watch

`make index-size` compares Meilisearch's `usedDatabaseSize` against the container's memory
limit. Past about 80 % of the limit, raise `meilisearch.mem_limit` in
`deploy/docker-compose.yml` and restart that one service. Ignoring it does not degrade
gracefully — indexing fell from 260 to 8 documents a minute the day it happened here.

### Models

The summariser reads GGUF files from `XUSTIVE_MODEL_DIR` (mounted read-only at `/app/models`).
`scripts/fetch-models.sh` downloads them; without them the API starts and says summaries are
unavailable. The Qwen 3B default is **not licensed for commercial use** — pin a commercial-safe
size in `config/prod.toml` before you charge anyone for anything.

## When it does not work

| Symptom | Cause, usually |
|:---|:---|
| Caddy loops on "obtaining certificate" | DNS does not point here yet, or 80/443 is blocked upstream. `make preflight` catches both. |
| `api` exits immediately | `XUSTIVE_ADMIN_KEY` is empty or under 16 characters. It is refusing on purpose. |
| Search returns nothing | the crawl has not run yet — `make deploy-seed`, then wait; `make deploy-logs` shows fetches. |
| Indexing crawls | `make index-size`. |
| The console 401s | Caddy's basic auth passed but the injected key is wrong — `XUSTIVE_ADMIN_KEY` must match in `.env` and the API's environment (they come from the same variable, so this means a stale container: `make deploy`). |
| Weather cards vanish after a few hours | the tool fetcher is not running or its image is stale (`docker compose … ps toold`). |

## Not built yet

Volunteer crawling and GPUs, multi-server sharding, and the Kubernetes chart — all specified in
[[Milestone 14 - One Server, Many Hands]] and all resting on work this deployment does not need.

## Related

[[Deployment Topology]] · [[Operating Xustive]] · [[Runbooks]] · [[Security and Privacy]] ·
[[Performance Budgets]]
