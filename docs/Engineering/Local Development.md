---
tags:
  - engineering
  - ops
type: guide
status: specified
updated: 2026-08-06
---

# Local Development

> [!warning] This is a specification, not a runbook
> It describes the development environment for the **finished** system, including the crawler,
> the ML service and the queue workers — none of which are built yet. Commands here such as
> `make setup`, `make run-crawler`, `make eval` and `make fixture-site` do not exist.
>
> For what actually runs today, with verified commands, see **[[Running the System]]**.

> Getting from a clean machine to a working Xustive in under 30 minutes.
> Parent: [[Home]] · Related: [[Running the System]], [[Deployment Topology]], [[Testing Strategy]]

---

## 1. Repository Layout

```
xustive/
├── Cargo.toml                 # workspace
├── crates/
│   ├── xustive-api/           # C01,C02,C03,C04,C05,C24  → binary
│   ├── xustive-ml/            # C08,C09,C10,C18(transformer) → binary
│   ├── xustive-crawler/       # C11,C12,C13,C14,C15,C21,C22 → binary
│   ├── xustive-worker/        # C16,C17,C19,C23 → binary
│   ├── xustive-cli/           # admin, dlq, eval, migrate → binary
│   ├── xustive-core/          # Document/Comment/Source types, errors, config
│   ├── xustive-text/          # ★ shared normalisation — used at query AND index time
│   ├── xustive-search/        # Meilisearch + Qdrant clients, ranking
│   ├── xustive-queue/         # Redis Streams abstraction
│   └── xustive-telemetry/     # tracing + metrics setup, privacy guards
├── web/                       # Next.js app: app/, components/, lib/, public/
├── data/                      # lexicons, parser rules, gazetteers, blocklists
│   ├── lang/ expansion/ sentiment/ parsers/ suggest/ spam/ crawl/
├── config/                    # dev.toml, staging.toml, prod.toml, ranking.toml
├── eval/                      # golden query sets, relevance harness
├── tests/fixtures/            # HTML, social payloads, audio, images, poison, injection
├── deploy/                    # docker-compose.yml, Caddyfile, prometheus/, grafana/
├── docs/                      # ← this vault
└── Makefile
```

`xustive-text` is marked with a star because it is the crate that quietly holds the system together:
if query-time and index-time normalisation ever diverge, search silently stops matching Arabic
([[Content Parser]] §4.4, [[Query Pipeline]] §4.1).

---

## 2. Prerequisites

| Tool | Version | Notes |
|:---|:---|:---|
| Rust | 1.8x stable | `rustup component add clippy rustfmt` |
| Docker + Compose | v2 | |
| Node | 20+ | UI build only |
| `just` or `make` | — | task runner |
| ~20 GB disk | — | models + dev index |

System libraries for `xustive-ml`: `libtesseract-dev`, `libleptonica-dev`, `libclang-dev`,
`cmake`, `pkg-config`. On Arch: `pacman -S tesseract tesseract-data-ara tesseract-data-fra clang cmake`.

---

## 3. First Run

```bash
git clone … && cd xustive
make setup          # ✅ exists — prerequisites, hooks, .env (model fetch arrives with M3)
make dev-up         # ✅ exists
make seed           # ✅ exists
make run-api        # ✅ exists
make run-web        # ✅ exists — Next.js dev server on :3000, proxies /api/v1 to :8080
```

> **To actually run the system today, follow [[Running the System]].** It is verified against the
> current code. This section describes the eventual shape.

In the finished system `make setup` also fetches model files, checksum-verified and cached in
`./models`, which is bind-mounted into `xustive-ml` ([[Deployment Topology]] §5). Today there are
no models to fetch, so it only checks prerequisites, installs the git hooks and creates `.env`.

### Make targets

| Target | Does | Today |
|:---|:---|:---|
| `make setup` | prerequisites, hooks, `.env` (+ models, eventually) | ✅ |
| `make up` | infrastructure, corpus and a seeded index in one step | ✅ |
| `make dev-up` / `dev-down` | infrastructure containers | ✅ |
| `make run-api` | run the API with `config/dev.toml` | ✅ |
| `make seed` / `migrate` | index the sample corpus; apply index settings | ✅ |
| `make test` / `lint` / `check` | test and lint tiers ([[Testing Strategy]]) | ✅ |
| `make text` / `search` | normalisation and search from the terminal | ✅ |
| `make crawl` / `worker` / `toold` | crawler, index queue drain, tool-data fetcher | ✅ |
| `make run-web` / `web-build` | Next.js dev server on :3000; production build | ✅ |
| `make seed-crawl` | crawl the local fixture site | ❌ M2 |
| `make eval` | relevance harness → nDCG report | ✅ |
| `make dlq` | inspect/replay dead letters | ✅ |

`make help` is always the authoritative list — it is generated from the Makefile, so it cannot
drift from reality the way this table can.

---

## 4. Local Topology

```
localhost:3000  xustive-web (Next.js) — the site; proxies /api/v1 → 8080
localhost:8080  xustive-api
localhost:8081  xustive-ml
localhost:7700  meilisearch
localhost:6333  qdrant
localhost:6379  redis
localhost:9090  prometheus
localhost:3001  grafana (admin/admin)
```

Dev differs from prod in exactly two ways: replica counts, and `MEILI_ENV=development` (no master
key). Everything else comes from `config/dev.toml` so behaviour differences are visible, not hidden
in code ([[Deployment Topology]] §1).

---

## 5. Working Without the Internet

Crawling the real web from a laptop is slow, rude, and non-reproducible. So:

- `make fixture-site` ✅ — serves `tests/fixtures/site/` on `localhost:8099`: a sitemap, an RSS
  feed, an SPA page, redirect chains and a cycle, a 429 with `Retry-After`, a 5-second endpoint,
  a `robots.txt` with `Crawl-delay` and `Disallow`, a `windows-1256` page, malformed markup, a
  prompt-injection page, and an infinitely deep crawler trap. `tests/fixtures/site/README.md`
  says which failure each one reproduces.
- `cargo test -p xustive-ingest --test fixture_site` runs the real `Fetcher` against all of it.
  The server starts and stops with the test binary on an OS-assigned port — a fixed port
  collides with an orphan from an earlier run, and the symptom is a suite quietly testing stale
  code rather than an obvious failure to bind.
- Reaching loopback at all requires `SafeUrl::allow_loopback_for_testing()`. The SSRF guard
  refuses private addresses by default and neither server binary ever turns it off; the switch
  is process-wide, so only that one integration test calls it.
- `config/dev.toml` seeds the frontier from that host only.
- Social connectors run in **replay mode** against recorded fixtures — there is no live-API path in
  dev or CI ([[Social Connector - Facebook]] §11).

❌ *not built yet* — `make seed-crawl` will exercise the entire ingestion pipeline end-to-end
without a single external request.

Today the equivalent is `make up`, which seeds the index from a generated corpus rather than by
crawling ([[Running the System]] §3).

---

## 6. Configuration

Layered: built-in defaults → `config/{env}.toml` → `XUSTIVE_*` env vars → CLI flags.

```toml
# config/dev.toml (excerpt)
[api]
bind_addr = "0.0.0.0:8080"
max_concurrent = 64

[search]
meili_url = "http://localhost:7700"
candidate_pool = 200

[summarizer]
enabled = true
model_path = "./models/qwen2.5-3b-instruct-q4_k_m.gguf"
n_slots = 1        # laptops

[crawler]
seeds = ["http://localhost:8090/"]
respect_crawl_delay = true
```

Secrets never live in `config/*.toml`. `.env` (gitignored) holds them locally; a mounted secrets file
holds them in prod ([[Security and Privacy]] §7).

---

## 7. Running Less Than Everything

Most work needs only part of the stack:

| Working on | Run |
|:---|:---|
| UI | `dev-up`, `run-api`, `run-web` (summary disabled in config) |
| Ranking | `dev-up`, `run-api`, `make eval` ❌ *harness not built (M1-T15)* |
| Parser | `run-worker` + `make fixture-site` ❌ *not built* |
| Crawler | `dev-up`, `run-crawler`, `make fixture-site` ❌ *not built* |
| Summarizer | `run-ml` + a saved response fixture — no crawling required |

`xustive-ml` is the expensive one (~4 GB RAM, slow start). `[summarizer] enabled = false` in
`dev.toml` is the default for UI and ranking work.

---

## 8. Debugging

```bash
# ❌ these arrive with the crawler and the ranking work
RUST_LOG=xustive_crawler=debug,xustive_worker=info make run-crawler
make dlq stats                          # what's failing and why  ❌ not built
make dlq peek --stage parse --limit 5   # ❌ not built
xustive-cli query --explain "سونلغاز"   # ranking breakdown per result

# ✅ available today
RUST_LOG=xustive_search=debug make run-api
make text Q='الجَزَائِر'                 # what normalisation actually does
make search Q='وهران'
```

`query --explain` prints each ranking signal's contribution per result
([[Ranking and Relevance]] §3) — it is the fastest way to answer "why is this result third?".

Tracing spans are visible in the terminal in dev; Grafana at `:3001` is provisioned with the same
dashboards as prod ([[Observability]] §5).

---

## 9. Conventions

| Area | Rule |
|:---|:---|
| Formatting | `cargo fmt`; enforced in CI |
| Lints | `clippy -D warnings`; `unwrap_used`/`expect_used` denied outside tests |
| Errors | `thiserror` per crate, each variant carrying `ErrorClass` ([[Error Handling and Resilience]] §1) |
| Async | Tokio; blocking work in `spawn_blocking` — never block the runtime |
| Logging | `tracing` only; **never** a query, transcript, or user-supplied OCR text |
| Commits | conventional commits; the component id in the scope: `feat(C12): honour Retry-After` |
| Branches | `main` protected; PRs require CI green + one review |
| Data files | `data/**` changes need the corresponding relevance gate to pass ([[Query Expander]] §11) |

---

## 10. Common Problems

| Symptom | Cause | Fix |
|:---|:---|:---|
| Arabic queries match nothing | index-time/query-time normalisation drift | `xustive-cli text normalize`; run the symmetry test |
| `xustive-ml` won't start | model file missing or checksum mismatch | `make setup` again |
| Meilisearch OOM in dev | `MEILI_MAX_INDEXING_MEMORY` unset | it is set in `dev-up`; check you're not running a stray container |
| Crawler does nothing | politeness fail-closed (Redis down or robots unreachable) | `make dev-up`; check `robots:` keys |
| Everything is slow | `xustive-ml` on CPU with `n_slots > 1` | set `n_slots = 1` in `dev.toml` |
| Tests pass locally, fail in CI | fixture ordering or a wall-clock dependency | use fixed timestamps, never `now()` in assertions |

---

## 11. Open Questions

- [ ] Is a devcontainer worth maintaining, given the native ML dependencies?
- [ ] Should the sample corpus ship in the repo (large) or be generated by `make seed` from fixtures?
- [ ] Do we need a shared staging index snapshot developers can pull for realistic relevance work?

## Related

[[Deployment Topology]] · [[Testing Strategy]] · [[Component Map]] · [[Performance Budgets]] ·
[[TODO]] · [[Observability]]
