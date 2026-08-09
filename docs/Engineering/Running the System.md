---
tags:
  - engineering
  - ops
  - runbook
type: runbook
status: living
updated: 2026-08-06
---

# Running the System

> **This note describes what actually runs today.** Every command here has been executed against
> the current code, and the timings and output are real.
>
> For the design of the *eventual* system — the crawler, the ML service, the queue workers — see
> [[Local Development]]. That note is a specification and describes components that do not exist
> yet. This one is the runbook.

---

## 1. What exists right now

Search works end to end over real content crawled from Algerian sites, and AI summaries are
generated locally from the results.

| Runs today | Not built yet |
|:---|:---|
| `xustive-api` — HTTP, search, server-rendered results | `xustive-worker` — a standing parse/enrich/index service |
| `xustive-cli` — migrate, seed, **crawl**, stats, text, search | Continuous scheduled crawling — [[Milestone 2 - Ingestion at Scale]] |
| Language detection, Arabizi ↔ Arabic expansion, sentiment | Voice and image search — [[Milestone 3 - Multimodal Input]] |
| Meilisearch index + generated synonyms | Qdrant is running but unused |
| The web UI, including the no-JavaScript path | Autocomplete, filters UI |
| **AI summaries** — local Qwen2.5 via llama.cpp ([[Summarizer]]) | Streaming summaries — see [[Summarizer]] §3 |
| **`/admin`** — switch the summariser between GPU and CPU | Redis-backed queueing |

Crawling is a command you run, not a service that runs itself: `xustive-cli crawl` fetches a
seed list, and nothing schedules it yet. Qdrant is started because later milestones need it and
the topology is cheaper to settle now than to change later.

**Summaries are slow on CPU** — 16 to 27 seconds depending on the model, against a 2.5 s budget.
They are fetched by a second request after the results render, so nothing waits for them, but the
gap is real and measured; see [[Summarizer]] §8.

---

## 2. Prerequisites

| Tool | Version | Why |
|:---|:---|:---|
| Rust | 1.85+ | `rustup toolchain install stable` |
| Docker + Compose v2 | any current | infrastructure |
| Python 3 | 3.8+ | the corpus generator |
| `make` | any | task runner |

No system libraries are needed yet. Tesseract, `libtorch` and the model files listed in
[[Deployment Topology]] only become prerequisites when `xustive-ml` arrives in
[[Milestone 3 - Multimodal Input]].

Disk: roughly 6 GB for the Rust build plus about 200 MB for the sample index.

---

## 3. First run

```sh
git clone https://github.com/ABakdi/Xustive.git
cd Xustive

make setup       # check prerequisites, install git hooks, create .env
make up          # infrastructure, corpus, index settings, and seed data
make run-api     # foreground; Ctrl-C to stop
```

`make setup` is optional — `make up` works without it — but it tells you up front if something is
missing rather than failing halfway through. It is safe to re-run and will not overwrite an
existing `.env`.

It does **not** download models. `xustive-ml` does not exist yet, so there is nothing to fetch;
that step joins `setup` in [[Milestone 3 - Multimodal Input]].

Then open <http://localhost:8080>, or run `make web`.

> **There is no separate web server.** The UI is plain HTML, CSS and JavaScript in `web/public/`,
> served by `xustive-api` itself. There is no `make run-web`, no dev server on port 3000 and no
> build step — editing a file in `web/public/` and reloading the page is the whole workflow.
>
> That changes when the Tailwind and esbuild pipeline arrives with the component library in
> M1-T13. Until then, less machinery is the point.

`make up` took **12.7 s** on a warm build. The first run also has to compile the workspace, which
takes a few minutes; everything after that is fast.

What `make up` actually does, in order:

1. `dev-up` — starts the five containers and waits for Meilisearch to report healthy.
2. `corpus` — generates ~10 000 sample documents into `tests/fixtures/corpus/`.
3. `seed` — runs `migrate` (creates indexes, applies settings and synonyms), then indexes the
   corpus.

Expected output ends with:

```
  indexed 10385/10385
✓ seeded 10385 documents into documents

Ready. Start the API with:  make run-api
Then open:                  http://localhost:8080
```

### Real content instead of the sample corpus

`make up` seeds a generated corpus so search works immediately with no network. To index the
actual Algerian web:

```sh
make crawl                                   # all seeds in data/sources/seeds.tsv
make crawl ARGS="--source aps-dz --max 20"   # one source, small
```

It is slow on purpose. One request per host at a time with the site's declared `Crawl-delay`
means roughly 150 documents takes several minutes, and that pacing is not a knob worth turning:
sites that notice us block us, and a blocked source is a permanent hole in the index.

`robots.txt` fails closed. A timeout, a 5xx, a 401 or a 403 all mean "do not crawl" — only a 404
means there are no restrictions. Some sources will report zero documents for that reason, and
that is the system working.

Seeds live in `data/sources/seeds.tsv`, one line per entry point, with a trust tier that feeds
ranking.

### Verify it worked

```sh
curl -s localhost:8080/readyz                 # -> ready
make stats                                    # -> documents: 10385 documents
./scripts/smoke.sh                            # -> 45 passed, 0 failed
```

Try these queries in the browser — they exercise different parts of the language stack:

| Query | Exercises |
|:---|:---|
| `سونلغاز` | Arabic retrieval, and reaches Latin-spelled documents through synonyms |
| `sonelgaz` | the same in reverse: Latin query reaching Arabic documents |
| `واش راك` | Darija detection in Arabic script |
| `wach rak khouya` | Arabizi detection in Latin script |
| `ch7al` | digit-as-consonant Arabizi, expanded to `شحال` and `combien` |
| `facture electricite` | French |

---

### AI summaries

The summariser needs model weights, which are not in the repository — they are gigabytes and
change independently of the code.

```bash
./scripts/fetch-models.sh          # the 2 GB default (Qwen2.5-3B-Instruct Q4_K_M)
./scripts/fetch-models.sh all      # also the 1.1 GB 1.5B, which is faster on CPU
```

Restart `xustive-api` afterwards. It loads the model in the background at startup and logs
`summariser ready`; until then, searches work normally and simply return no summary.

Search a topic with several indexed articles and the summary appears above the results a few
seconds later. Nothing on the page waits for it.

To check what the summariser is doing, open **http://localhost:8080/admin**. It shows which
device is in use, why, which models are present, and lets you switch between GPU and CPU.

### Choosing GPU or CPU

The device is a runtime setting, changed from `/admin` or with `XUSTIVE_DEVICE=cpu|gpu|auto`.
It takes effect on the next model load, so restart after changing it.

Using a GPU also requires a binary built with GPU support, which needs the CUDA toolkit:

```bash
cargo build --release -p xustive-api --features cuda
```

Without it, `/admin` reports *"this binary was built without GPU support"* even when it can see
the card — that is the expected message, not a fault. A CPU-only build is fully functional; it is
only slower.

To measure what the hardware actually delivers:

```bash
cargo run --release -p xustive-ml --example bench           # CPU
cargo run --release -p xustive-ml --example bench -- gpu    # GPU, on a cuda build
```

---

## 4. Ports

| Port | Service | Notes |
|:---|:---|:---|
| **8080** | `xustive-api` | the application |
| 7700 | Meilisearch | |
| 6333 | Qdrant | running, unused |
| **6390** | Redis | **not 6379** — see below |
| 9090 | Prometheus | |
| 3001 | Grafana | `admin` / `admin` |

Everything except the API binds to `127.0.0.1` only.

Redis defaults to **6390** because a development machine usually runs more than one stack and
6379 is very often taken. Every port is overridable through `.env`:

```sh
cp .env.example .env
# then edit, e.g.
XUSTIVE_REDIS_PORT=6395
XUSTIVE_MEILI_PORT=7701
```

### Why there are two compose files

`deploy/docker-compose.yml` is the **production** topology: it publishes no ports at all, and its
networks are `internal: true` so nothing on them can reach the internet. That is what makes the
data-sovereignty claim a property of the network rather than a promise
([[Security and Privacy]] §1). A CI lint asserts it stays that way.

`internal: true` also stops Docker routing *inwards*, so published ports do not work on those
networks. In production that does not matter because `xustive-api` runs in a container alongside
the backends. In development it runs on the host, so `deploy/docker-compose.dev.yml` adds a
separate bridge network and publishes the ports. The Makefile always uses both files.

---

## 5. Everyday commands

```sh
make help          # every target, with descriptions
```

### Running and inspecting

| Command | Does |
|:---|:---|
| `make run-api` | run the API — **this also serves the web UI** |
| `make web` | open the UI in a browser |
| `make stats` | document counts per index |
| `make search Q='وهران'` | search from the terminal |
| `make text Q='الجَزَائِر'` | show what normalisation does to a string |
| `make dev-logs` | tail the container logs |

`make text` is the first thing to reach for when a query returns nothing unexpected — "why does
this not match" is almost always a normalisation question:

```
$ make text Q='الجَزَائِر ٢٠٢٦'
input      الجَزَائِر ٢٠٢٦
normalized الجزائر 2026
folded     الجزاير 2026
changed
           U+064E  harakat, removed
           U+0650  harakat, removed
           U+0662  Arabic-Indic digit, folded to ASCII
           U+0660  Arabic-Indic digit, folded to ASCII
           U+0666  Arabic-Indic digit, folded to ASCII
script     Arabic
tokens     ["الجزائر", "2026"]
chars      15 -> 12
hash       b3:57d9743bef7de30ca58728d353be4e4731bfda424ae47827931bb703f28251dd
simhash    (text too short to be meaningful)
```

The `changed` block is the useful part: it names the characters that did not survive and why.
Those characters are invisible by definition, so listing their codepoints is the only way to see
them. Note that it distinguishes *removed* from *folded* — an Arabic-Indic digit is not deleted,
it becomes an ASCII one, and reporting that as removal would send you hunting a bug that is not
there.

`folded` appears only when it differs from `normalized`. It is the aggressive form used for the
secondary match field, which collapses orthographic variants — here `ائ` to `اي`.

### Quality checks

| Command | Does | Needs |
|:---|:---|:---|
| `make check` | everything CI runs: fmt, clippy, both lints, all tests | — |
| `make test` | the test suite (254 tests) | — |
| `make lint` | fmt check, clippy, telemetry lint, compose lint | — |
| `./scripts/smoke.sh` | 45 end-to-end checks against a running API | API running |
| `make egress-test` | proves the serving plane cannot reach the internet | `make dev-up` |
| `make audit` | dependency advisories and licences | `cargo install cargo-deny` |

`make check` is the one to run before committing. It is what CI runs, so a green local run means
a green pipeline.

### Index management

| Command | Does |
|:---|:---|
| `make crawl` | **fetch and index real Algerian sites** (respects robots.txt) |
| `make migrate` | create indexes, apply settings and synonyms — idempotent |
| `make migrate-check` | report drift between declared and live settings |
| `make corpus` | regenerate the sample corpus |
| `make seed` | migrate, then index the corpus |

Run `make migrate` after editing `data/expansion/*.tsv`. Those files generate the index synonyms,
and the index will not pick up an edit until the settings are reapplied.

---

## 6. Stopping and resetting

```sh
make dev-down     # stop containers, keep the indexed data
make dev-reset    # stop containers and DELETE all volumes
```

After `dev-reset` you need `make up` again to rebuild the index. It takes about 13 s.

The API is stopped with Ctrl-C when run in the foreground. If it was backgrounded:

```sh
kill "$(ss -tlnp | grep -oP '8080.*pid=\K[0-9]+' | head -1)"
```

> Do not use `pkill -f xustive-api` from a shell whose own command line contains that string —
> `pkill -f` matches the full command line and the shell will kill itself. This is not
> hypothetical; it happened while building this.

---

## 7. Troubleshooting

### `make run-api` and then "site can't be reached"

Almost always the build, not the server. `xustive-api` links llama.cpp for the summariser, and
**llama.cpp is compiled from source** — several minutes on a first build or after a `cargo clean`.
The port is not open until that finishes, so the browser correctly reports nothing there.

`make run-api` now prints a notice before building. Wait for:

```
  Xustive is running.

    Web UI    http://localhost:8080
```

To skip the summariser entirely and get a build in seconds:

```bash
make run-api-fast     # no AI summaries; everything else works
```

If the build is finished and the port is still closed, check that something else is not already
holding it:

```bash
ss -ltnp | grep 8080
```


Every entry here is a problem actually hit while developing, not a hypothetical.

| Symptom | Cause | Fix |
|:---|:---|:---|
| `Bind for 0.0.0.0:6379 failed: port is already allocated` | another local stack owns the port | set `XUSTIVE_REDIS_PORT` in `.env`; Redis already defaults to 6390 for this reason |
| `curl: (7) Failed to connect to localhost:7700` after `docker compose up` | you started only the base compose file, whose networks are `internal: true` | use `make dev-up`, which includes the dev override |
| Code changes have no effect | an older API process still holds port 8080 | check `ss -tlnp \| grep 8080` and kill that pid — a stale pidfile will not save you |
| `readyz` returns 503 | Meilisearch is not up | `make dev-up`, then `curl -s localhost:7700/health` |
| Arabic query returns nothing | normalisation mismatch | `make text Q='…'` and compare with what is indexed |
| A synonym edit has no effect | index settings not reapplied | `make migrate` |
| `indexing task N failed: Index already exists` | fixed — `ensure_index` now checks existence first | pull latest |
| `make audit` fails immediately | `cargo-deny` not installed | `cargo install cargo-deny` |
| Smoke tests fail on latency | the workspace was built in debug | expected; the budget still passes comfortably at ~20 ms |

### Reading the logs

```sh
RUST_LOG=debug make run-api                       # everything
RUST_LOG=xustive_search=debug make run-api        # one crate
```

Note what you will *not* find in the logs: **query text**. That is deliberate and enforced by
`scripts/lint-telemetry.sh` plus a canary check in the smoke suite
([[ADR-0008 - No Query Logging]]). If you are debugging a specific query, reproduce it with
`make search Q='…'` rather than looking for it in a log file.

---

## 8. Metrics

Prometheus scrapes the API at `host.docker.internal:8080`. Grafana is at
<http://localhost:3001> (`admin`/`admin`), though no dashboards are provisioned yet — those
arrive with [[Milestone 4 - Quality and Operations]].

The raw endpoint is often quicker:

```sh
curl -s localhost:8080/metrics | grep xustive_

xustive_http_requests_total{route="/api/v1/search",status="200"} 95
xustive_http_requests_total{route="/api/v1/search",status="400"} 16
xustive_lang_detected_total{lang="ar",script="arabic"} 78
xustive_lang_detected_total{lang="ary",script="arabic"} 1
xustive_lang_detected_total{lang="ary",script="latin"} 4
```

`xustive_lang_detected_total` is worth watching as a product signal, not just an operational one:
if the `ary` share sits near zero, either detection is broken or the assumption about who uses
this is wrong ([[Language Detector]] §9).

---

## 9. Working on one part

The whole stack is rarely needed.

| Working on | Run |
|:---|:---|
| UI | `make dev-up`, `make run-api`, then edit `web/public/` and reload. No build step, no watcher — the files are served as they are on disk. |
| Ranking or search | `make dev-up`, `make run-api`, `make search Q='…'` |
| Normalisation or language | `cargo test -p xustive-text -p xustive-lang` — no containers needed |
| Lexicons | edit `data/`, `cargo test -p xustive-lang`, then `make migrate` for synonyms |

The `xustive-text` and `xustive-lang` crates have no I/O, so their tests run in under a second
with nothing else running. That is where most language work happens.

---

## 10. Related

[[Local Development]] — the specification for the full development environment, including
components not yet built.
[[Deployment Topology]] — production sizing, networks, and backup.
[[Observability]] — the metrics and alerts this will grow into.
[[TODO]] — what is built and what is next.
