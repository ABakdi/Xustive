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

api     │ xustive-api listening addr=127.0.0.1:8080
web     │ ✓ Ready in 480ms
crawler │ crawler starting seeds=20 waiting=983
worker  │ indexed 6 · rejected 0 · dead-lettered 0
```

**The first build takes minutes.** `xustive-api` links llama.cpp for the summariser and llama.cpp
compiles from source. To skip it:

```bash
make dev ARGS=--fast          # no AI summaries; everything else works
make dev ARGS=--no-crawler    # leave the crawler off, for frontend work
```

`make dev` **refuses to start** if something already holds 8080 or 3000. Starting alongside gives
a log full of "address in use" and — worse — the old processes keep serving, which is what makes
code changes appear to do nothing. `make dev-stop` stops a running `make dev` from another
terminal.

### If you would rather run the parts separately

```bash
make dev-up      # infrastructure only
make run-api     # the Rust API,         :8080
make run-web     # the Next.js frontend, :3000
make crawld      # the crawler
make worker      # drain the index queue into Meilisearch
```

---

## 2. Prerequisites

| | Version | Why |
|:---|:---|:---|
| Rust | 1.85+ | edition 2024 |
| Docker + Compose | v2 | Meilisearch, Redis, Qdrant, Prometheus, Grafana |
| Node | 20+ | the frontend |
| `curl`, `jq` | any | the scripts and the smoke suite |

`make setup` checks all of this and tells you what is missing rather than failing halfway through.
It is safe to re-run and will not overwrite an existing `.env`.

Roughly 8 GB of RAM is comfortable; the summariser is the expensive part, and `--fast` removes it.

---

## 3. What runs where

| Port | Process | Serves |
|---:|:---|:---|
| **3000** | `xustive-web` (Next.js) | **the site — open this one** |
| 8080 | `xustive-api` (Rust) | JSON under `/api/v1/…`, plus `/admin`, `/bot`, `/metrics` |
| 7700 | Meilisearch | the index |
| 6390 | Redis | queue, frontier, robots and tool caches |
| 6333 | Qdrant | vectors — unused until M3 |
| 9090 | Prometheus | metrics |
| 3001 | Grafana | dashboards |

> **Open :3000, not :8080.** The API answers JSON and has served no pages since the Rust renderer
> was deleted in M1B — `localhost:8080/search` is a **404**, which reads like a broken install and
> is not. The frontend proxies `/api/v1/*` through to 8080, so the browser only ever talks to one
> origin, which is what keeps the CSP at `default-src 'self'`.

Redis defaults to **6390**, not 6379, because another local stack owning 6379 is common. Override
with `XUSTIVE_REDIS_PORT` in `.env`.

---

## 4. The repository

```
xustive/
├── crates/
│   ├── xustive-api/      HTTP surface: search, suggest, summary, translate, admin, /bot
│   ├── xustive-cli/      migrate, seed, crawl, crawld, worker, dlq, eval
│   ├── xustive-core/     Document/Comment/Source types, config, SafeUrl, errors
│   ├── xustive-ingest/   fetch, robots, frontier, orchestrator, parse, sitemap
│   ├── xustive-lang/     language detection, transliteration, query expansion
│   ├── xustive-ml/       llama.cpp engine, prompts, translation, device selection
│   ├── xustive-queue/    Redis Streams, indexer worker, dead-letter queue
│   ├── xustive-search/   Meilisearch client, ranking, query operators
│   ├── xustive-text/     ★ shared normalisation — query AND index time
│   ├── xustive-toold/    scheduled fetch of external tool data (weather, …)
│   └── xustive-tools/    instant answers: calculator, units, prayer, fuel, …
├── web/                  Next.js app: app/, components/, lib/, public/
├── config/               dev.toml, ci.toml, staging.toml, prod.toml
├── data/                 seeds, parser rules, lexicons, gazetteers
├── deploy/               docker-compose, prometheus/, grafana/
├── eval/                 golden query sets and the relevance harness
├── scripts/              dev.sh, lints, budgets, egress test
└── tests/fixtures/       the offline crawler fixture site, corpora
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
bind_addr = "127.0.0.1:8080"

[search]
meili_url = "http://localhost:7700"

[crawl]
respect_crawl_delay = true
per_host_concurrency = 1
ignore_politeness = false     # testing only; production refuses to start with this on
```

Secrets never live in `config/*.toml`. `.env` holds them locally and is gitignored; a mounted
secrets file holds them in production ([[Security and Privacy]] §7).

---

## 6. Everyday commands

`make help` is the authoritative list — it is generated from the Makefile and cannot drift.

| Command | Does |
|:---|:---|
| `make dev` | everything, one terminal |
| `make dev-stop` | stop a `make dev` from elsewhere |
| `make dev-up` / `dev-down` / `dev-reset` | infrastructure only; `reset` **deletes volumes** |
| `make migrate` / `migrate-check` | apply index settings; report drift |
| `make corpus` / `seed` | generate and index the sample corpus |
| `make crawl` | one-shot crawl of the seed list |
| `make crawld` | the continuous crawler |
| `make worker` | drain the index queue |
| `make toold` | fetch weather and other tool data |
| `make search Q='…'` / `text Q='…'` | search and normalisation from the terminal |
| `make stats` | index document counts |
| `make eval` / `eval-check` | relevance harness; fail on regression |
| `make dlq A=stats\|peek\|replay` | dead letters |
| `make test` / `lint` / `check` | test and lint tiers ([[Testing Strategy]]) |
| `make smoke` | end-to-end checks against a running stack |
| `make egress-test` | prove the serving plane cannot reach the internet |
| `make ui-gates` | client asset budgets and the no-JavaScript path |
| `make fixture-site` | the offline crawler fixture site on :8099 |

---

## 7. Working on one part

| Working on | Run |
|:---|:---|
| Frontend | `make dev ARGS="--fast --no-crawler"` — no summariser, no crawl noise |
| Ranking | `make dev ARGS=--fast`, then `make eval` |
| Parser | `make fixture-site`, then `cargo test -p xustive-ingest` |
| Crawler | `make dev-up`, `make crawld`, and `make fixture-site` for offline cases |
| Summariser | `make dev` (needs the model) — see §9 |
| Instant answers | `cargo test -p xustive-tools`; no infrastructure at all |

Most of the test suite needs nothing running. The exceptions are the Redis-backed frontier and
robots-cache suites, which **skip rather than fail** when Redis is absent.

---

## 8. Working offline

The whole thing runs with no internet:

- `make corpus` generates ~10 000 sample documents.
- `make fixture-site` serves a local site reproducing redirect chains, legacy encodings, robots
  directives, traps and malformed markup.
- The models are on disk once fetched.

Only `make crawld`, `make crawl` and `make toold` need the network, because they are the parts that
talk to the outside.

---

## 9. The summariser

It is optional and it is the expensive part: a local Qwen2.5-3B, roughly 4 GB of RAM, and a first
build that compiles llama.cpp from source.

```bash
make dev ARGS=--fast     # skip it entirely
```

`/admin` shows which device it resolved to and lets you switch between CPU and GPU at runtime. A
CPU-only build is fully functional, only slower — GPU support needs `--features cuda` and the CUDA
toolkit at build time.

---

## 10. Troubleshooting

Every entry is a problem actually hit, not a hypothetical.

| Symptom | Cause | Fix |
|:---|:---|:---|
| `localhost:8080` shows nothing | 8080 is the JSON API | the site is **:3000** |
| "site can't be reached" right after starting | the first build compiles llama.cpp, minutes | wait, or `make dev ARGS=--fast` |
| `make dev` refuses to start | 8080 or 3000 already held | `make dev-stop`, or find the owner with `ss -ltnp \| grep -E ':8080\|:3000'` |
| Code changes have no effect | an older process still holds the port | as above — a stale pidfile will not save you |
| `Bind for 0.0.0.0:6379 failed` | another local stack owns Redis | set `XUSTIVE_REDIS_PORT` in `.env` |
| `curl: (7) … localhost:7700` | you started the base compose file alone, whose networks are `internal: true` | `make dev-up`, which includes the dev override |
| `readyz` returns 503 | Meilisearch not up | `make dev-up`, then `curl -s localhost:7700/health` |
| Arabic query returns nothing | normalisation mismatch | `make text Q='…'` and compare with what is indexed |
| A synonym edit has no effect | index settings not reapplied | `make migrate` |
| `docker compose` transfers gigabytes | a missing `.dockerignore` — fixed | pull latest |
| Crawler says `robots.txt unreachable` for a `.gov.dz` host | that server omits its certificate intermediate; `curl` fails too | not ours to fix — M2-T04.10 |
| `make audit` fails immediately | `cargo-deny` not installed | `cargo install cargo-deny` |

### Reading the logs

```bash
RUST_LOG=debug make run-api                    # everything
RUST_LOG=xustive_ingest=debug make crawld      # one crate
```

What you will **not** find in the logs is **query text**. That is deliberate, and enforced by
`scripts/lint-telemetry.sh` plus a canary in the smoke suite
([[ADR-0008 - No Query Logging]]). To debug a specific query, reproduce it with `make search Q='…'`
rather than looking for it in a log file.

---

## 11. Metrics

Prometheus scrapes the API at `host.docker.internal:8080`. Grafana is on :3001, credentials in
`.env`.

```bash
curl -s localhost:8080/metrics | grep xustive_
```

`xustive_lang_detected_total` is worth watching as a product signal rather than an operational
one: if the `ary` share sits near zero, either detection is broken or the assumption about who uses
this is wrong ([[Language Detector]] §9).

`xustive_data_age_seconds` is the one that catches a silently dead tool fetcher — see
[[Observability]] §6.

---

## 12. Stopping and resetting

```bash
make dev-stop     # stop the processes, leave the containers
make dev-down     # stop the containers, keep the data
make dev-reset    # stop the containers and DELETE every volume
```

`dev-reset` is the only destructive one. After it, `make up` rebuilds the index from the sample
corpus; anything crawled is gone, and re-crawling costs the *sites* bandwidth, not just you.

---

## 13. Related

[[Testing Strategy]] · [[Deployment Topology]] · [[Observability]] · [[Crawler Orchestrator]] ·
[[Politeness and Robots]] · [[Security and Privacy]]
