# Xustive

**Xustive is a self-hosted search engine for the Algerian web.** It crawls and indexes public
content in **Arabic, Darija, French and English** — including Darija written in Latin script
("Arabizi": `wach rak`, `ch7al`) — and answers with ranked results, a locally generated AI summary,
knowledge panels, instant tools (weather, currency, calculator, prayer times…), image and video
search, and voice input. It runs on one machine with a small GPU, or on CPU alone.

Two things are not features but rules the build enforces: **no query is ever logged**, and **the
serving plane has no egress** — a search never leaves the machine that answers it.

Everything below links into [`docs/`](docs/), an Obsidian vault that is the project's actual
specification: every component, decision and screen has a note, and the notes were audited
against the code on 2026-08-27.

## Table of contents

1. [What it does](#what-it-does)
2. [How it works](#how-it-works)
3. [Repository layout](#repository-layout)
4. [Quick start](#quick-start)
5. [Status](#status)
6. [Guarantees enforced by the build](#guarantees-enforced-by-the-build)
7. [Documentation map](#documentation-map)
   - [Start here](#start-here)
   - [Architecture](#architecture)
   - [Components](#components)
   - [Decisions (ADRs)](#decisions-adrs)
   - [User interface](#user-interface)
   - [Engineering and operations](#engineering-and-operations)
   - [Planning and problems](#planning-and-problems)
8. [Licence](#licence)

## What it does

| Area | What you get | Specified in |
|:---|:---|:---|
| **Search** | Full-text search over crawled Algerian pages and public posts, with cross-script query expansion (MSA ↔ Darija ↔ Arabizi ↔ French), typo tolerance, facets (language, source, date, sentiment), and a ranking that puts the reader's interface language first. Works without JavaScript. | [Ranking and Relevance](docs/Architecture/Ranking%20and%20Relevance.md) · [Query Pipeline](docs/Components/Query%20Pipeline.md) · [UI - Results Page](docs/UI/UI%20-%20Results%20Page.md) |
| **AI summary** | A short answer above the results, generated locally by a quantised Qwen model with citations into the results, streamed separately so it never delays the list. Written in the reader's language. | [Summarizer](docs/Components/Summarizer.md) · [ADR-0005 - Local Quantised LLM for Summaries](docs/Decisions/ADR-0005%20-%20Local%20Quantised%20LLM%20for%20Summaries.md) |
| **Knowledge panel** | A side panel for the entity a query means — person, film, place, team — from a locally harvested Wikidata store, with a live fallback judged by the same resolver; facts with icons, scores from IMDb/Rotten Tomatoes/Metacritic, a Wikipedia extract. | [Knowledge Store](docs/Components/Knowledge%20Store.md) · [UI - Knowledge Panel](docs/UI/UI%20-%20Knowledge%20Panel.md) · [ADR-0019 - The Knowledge Layer](docs/Decisions/ADR-0019%20-%20The%20Knowledge%20Layer.md) |
| **List answers** | "cast of the matrix", "livres de kateb yacine", "albums by cheb khaled": a row of cards with photos or covers, linking to Wikipedia, IMDb, Open Library, Goodreads, Google Books; a *See also* row for the films of a series or the seasons of a show that swaps the row and the panel in place. | [Milestone 8 - The Answer Layer](docs/Planning/Milestone%208%20-%20The%20Answer%20Layer.md) · [ADR-0022 - Entity Resolution Prefers Silence to a Wrong Panel](docs/Decisions/ADR-0022%20-%20Entity%20Resolution%20Prefers%20Silence%20to%20a%20Wrong%20Panel.md) |
| **Instant tools** | Weather for your wilaya (located from your connection with a local database — never sent anywhere), currency at the official rate, a calculator with units and currencies, dates, prayer times, fuel prices, exam dates, translation and transliteration. | [Instant Answers](docs/Components/Instant%20Answers.md) · [UI - Tool Cards](docs/UI/UI%20-%20Tool%20Cards.md) · [ADR-0020 - Approximate Location from a Local Database](docs/Decisions/ADR-0020%20-%20Approximate%20Location%20from%20a%20Local%20Database.md) |
| **Images and videos** | Separate *Images* and *Videos* tabs over media extracted from crawled pages (videos are metadata only — the watch page, never the stream), thumbnails served through a signed proxy so the reader's address never reaches the image host; optionally enriched by a self-hosted SearXNG. | [Milestone 9 - Images and Videos](docs/Planning/Milestone%209%20-%20Images%20and%20Videos.md) · [Media Extraction](docs/Components/Media%20Extraction.md) · [Thumbnail Proxy](docs/Components/Thumbnail%20Proxy.md) |
| **Voice and image input** | Tap the microphone and the words appear in the search box as you speak (Whisper on our own server — a fast model for the live reading, a careful one for the final pass; stop searches). Search by image via OCR. |
| **Reverse image search** | Drop, paste or photograph a picture: where it appears on the Algerian web (the same picture, then similar ones, from a local CLIP index), then the web by *description* — the picture never leaves; SearXNG receives words. Chips for kind (photo, illustration, screenshot…) and format (png, jpg…) computed from the results. | [Milestone 10 - Reverse Image Search](docs/Planning/Milestone%2010%20-%20Reverse%20Image%20Search.md) · [UI - Image Search](docs/UI/UI%20-%20Image%20Search.md) · [ADR-0028](docs/Decisions/ADR-0028%20-%20Reverse%20Image%20Search%20Sends%20Words%20to%20the%20Web%2C%20Never%20the%20Picture.md) | [UI - Voice Search](docs/UI/UI%20-%20Voice%20Search.md) · [Speech to Text](docs/Components/Speech%20to%20Text.md) · [UI - Image Search](docs/UI/UI%20-%20Image%20Search.md) |
| **Federation** | Optional query-time enrichment from a self-hosted SearXNG through a single gateway — the one allow-listed egress, off by a runtime switch. | [Federation Gateway](docs/Components/Federation%20Gateway.md) · [ADR-0017 - Query-Time Federation with External Metasearch](docs/Decisions/ADR-0017%20-%20Query-Time%20Federation%20with%20External%20Metasearch.md) |
| **Anonymous ranking signals** | Clicks are counted per query hash under a k-anonymity floor and feed a small, bounded ranking term. Off by default. | [Interaction Signals](docs/Components/Interaction%20Signals.md) · [ADR-0015 - Anonymous Interaction Signals for Ranking](docs/Decisions/ADR-0015%20-%20Anonymous%20Interaction%20Signals%20for%20Ranking.md) |
| **Operator console** | Crawler live view (pages, images and videos counted separately), documents browser with media facet, sources registry and health, queue and dead-letter, integrations switches, compute device, evaluation, maintenance. | [UI - Admin Console](docs/UI/UI%20-%20Admin%20Console.md) · [Crawler Console](docs/Components/Crawler%20Console.md) |
| **Four languages, RTL first** | The interface is translated into Arabic, Darija, French and English; Arabic is the default and the layout is designed right-to-left first. | [UI - RTL and Localization](docs/UI/UI%20-%20RTL%20and%20Localization.md) · [UI - Accessibility](docs/UI/UI%20-%20Accessibility.md) |

## How it works

Two planes that share nothing but the index ([ADR-0001 - Two-Plane Architecture](docs/Decisions/ADR-0001%20-%20Two-Plane%20Architecture.md)):

```
                       ┌──────────────────────── serving plane (no egress) ─────────────────────────┐
  browser ──▶ Next.js  │  xustive-api ──▶ Meilisearch (documents, comments, knowledge)               │
   (web/)   ◀── SSR    │      │       ──▶ Redis (tool cache, queue) · Redis-signals (ephemeral)       │
                       │      ├──▶ xustive-ml (llama.cpp summariser, CPU or GPU)                      │
                       │      ├──▶ sidecars: stt · ocr · clip-embed · text-embed (localhost only)     │
                       │      └──▶ federator ──▶ SearXNG        ← the one allow-listed hop           │
                       └────────────────────────────────────────────────────────────────────────────┘
                       ┌──────────────────────── ingestion plane ───────────────────────────────────┐
  the web ──▶ crawld (frontier, politeness, robots) ──▶ Redis Streams ──▶ worker (parse, dedup,     │
             toold (rates, weather, Wikidata harvest)                    enrich, media, index)      │
                       └────────────────────────────────────────────────────────────────────────────┘
```

- **A search** goes browser → Next.js (server-rendered) → `xustive-api` → Meilisearch, then through
  the ranker (relevance, authority, freshness, interface language, bounded click signal) and back;
  the summary streams on a separate connection; the knowledge panel and list answers arrive after
  paint. Under load the search narrows (fewer facets, a page-sized query) rather than failing
  ([ADR-0027 - Narrow the Search Under Load Instead of Failing](docs/Decisions/ADR-0027%20-%20Narrow%20the%20Search%20Under%20Load%20Instead%20of%20Failing.md)).
- **Ingestion** never touches the serving plane's process: `crawld` fetches politely (robots.txt,
  per-host budgets, adaptive re-crawl), the queue carries pages to the `worker`, which parses,
  de-duplicates, detects language and sentiment, extracts images and videos, and writes to the
  index. `toold` refreshes the data the tools read on a schedule.
- **Nothing identifying is kept.** No query text in any log or metric (a lint fails the build), no
  IP in any store, the rate limiter keys on a salted hash of the network, the location lookup is a
  local database read on the request's stack. Read [Security and Privacy](docs/Architecture/Security%20and%20Privacy.md)
  and [ADR-0008 - No Query Logging](docs/Decisions/ADR-0008%20-%20No%20Query%20Logging.md).

The long form: [System Architecture](docs/Architecture/System%20Architecture.md), then
[Component Map](docs/Architecture/Component%20Map.md) and [Deployment Topology](docs/Architecture/Deployment%20Topology.md).

## Repository layout

```
crates/        17 Rust crates (workspace)         web/         Next.js app: pages, API routes, components
services/      5 sidecars (Python/Docker)         deploy/      docker-compose (prod-shaped) + dev override
config/        dev / staging / prod / ci .toml    scripts/     setup, dev, lints, backup, smoke, eval helpers
data/          lexicons (TSV), authority, seeds   eval/        golden set and SERP-comparison harness
models/        summariser weights (gitignored)    tests/       fixtures (corpus, hostile pages)
docs/          the Obsidian vault — the specification
```

| Crate | Role |
|:---|:---|
| `xustive-text` | shared normalisation, called at **both** query time and index time — the crate that holds Arabic search together |
| `xustive-core` | canonical types, error taxonomy, config, `SafeUrl` SSRF guard, circuit breaker |
| `xustive-lang` | language detection and query expansion for Arabic, Darija, Arabizi, French, English |
| `xustive-search` | Meilisearch client, index settings, ranking weights, the search request/response contract |
| `xustive-api` | the HTTP surface (`/api/v1/*`): search, summary, knowledge, tools, transcribe, admin; rate limits |
| `xustive-ingest` | polite fetching, robots.txt, HTML → document extraction, frontier, dedup, interaction store |
| `xustive-queue` | Redis Streams task queue: produce, consume-group, ack, reclaim, dead-letter |
| `xustive-ml` | local model inference (llama.cpp): device selection, model registry, summarisation, translation |
| `xustive-media` | image OCR and image/video extraction from crawled pages |
| `xustive-vector` | Qdrant client for CLIP image-embedding similarity search |
| `xustive-tools` | instant answers: calculator, currency, weather, dates, prayer times, fuel, exams, translation |
| `xustive-toold` | scheduled fetch of external tool data (rates, weather, Wikidata entities) into a cache the serving plane reads |
| `xustive-knowledge` | the entity store: model, kind table, Wikidata parser, resolver, Meilisearch `knowledge` index |
| `xustive-federation` | SearXNG federation client (a leaf crate) |
| `xustive-federator` | the federation gateway — the one allow-listed egress hop from the serving plane |
| `xustive-cli` | operator tooling: migrate, seed, crawl daemon, index worker, eval, media re-pass |
| `xustive-loadgen` | open-loop HTTP load generator for the serving plane |

Sidecars under `services/`: `stt-sidecar` (faster-whisper; two models, GPU when present),
`ocr-sidecar` (Unlimited-OCR), `clip-embed`, `text-embed` (bge-m3), `searxng`. Each has its own
README. The web app (`web/`) is Next.js with server-rendered search, four locales, and a few API
routes of its own for the live knowledge fallback and the thumbnail proxy.

## Quick start

Needs Rust 1.85+, Docker, Node 20+, Python 3 (for the corpus generator).

```sh
make setup            # checks prerequisites, installs the pre-commit hooks, writes .env
make dev              # infrastructure + API + web + crawler + worker + toold, logs interleaved
make dev ARGS=--fast  # same, without the summariser (the first build compiles llama.cpp)
```

Open **<http://localhost:3000>** (not :8080 — the API serves JSON) and search for `سونلغاز`,
`wach rak`, or `facture`. `make help` lists every target; `make check` runs what CI runs.

Optional pieces, each one command and documented where linked:

| Want | Do | Doc |
|:---|:---|:---|
| Voice search | `cd services/stt-sidecar && ./run.sh` (weights under `data/models/`) | [`services/stt-sidecar/README.md`](services/stt-sidecar/README.md) |
| Weather located from the connection | `scripts/fetch-geoip.sh` (DB-IP City Lite, CC BY) | [Instant Answers](docs/Components/Instant%20Answers.md) |
| The summariser | `scripts/fetch-models.sh` — default model is non-commercial, see licence note | [Legal and Compliance](docs/Engineering/Legal%20and%20Compliance.md) |
| Images/videos from SearXNG | `deploy/` starts it; switch on in *Admin → Integrations* | [Federation Gateway](docs/Components/Federation%20Gateway.md) |

The full procedure, ports, troubleshooting and the honest dev caveats (host binaries need a
restart to pick up new code): [Running Xustive](docs/Engineering/Running%20Xustive.md). Day-two operations:
[Operating Xustive](docs/Operations/Operating%20Xustive.md) and [Runbooks](docs/Operations/Runbooks.md).

## Status

Seven of ten milestones are closed and verified against the code; the rest are honest about what
they lack.

| Milestone | State |
|:---|:---|
| 0 Foundations · 1 Text search · 1B Frontend and instant answers · 2 Ingestion at scale | closed (Aug 2026) |
| 3 Multimodal input | built — voice (live, GPU), OCR, image similarity; the accuracy gates (WER/CER, recall) are not yet measured |
| 4 Quality and operations | tooling built — breakers, load generator, backup/restore, alerts, runbooks; the 10M-document/chaos gate not run |
| 5 Beta launch | not started — only `/privacy` and `/bot` exist; no about/terms/submit-a-site |
| 6 Adaptive ranking · 7 Federated retrieval and tools · 8 The answer layer · 9 Images and videos | closed 2026-08-25/26 |
| 10 Reverse image search | built 2026-08-27 — a picture in, pictures out; two gate items open ([spec](docs/Planning/Milestone%2010%20-%20Reverse%20Image%20Search.md)) |

The current, verified picture — open items, what is deliberately not built, and what is next —
is kept in **[TODO](docs/Planning/TODO.md)**; the problem register is
[Problems](docs/BugReports/Problems.md).

## Guarantees enforced by the build

- **No query logging.** `scripts/lint-telemetry.sh` fails CI if a query or credential field appears
  in a `tracing` call; the metrics registry accepts only static label names; the smoke suite runs
  a canary query and greps the logs for it.
- **No exposed databases, no egress.** `scripts/lint-compose.sh` asserts the production compose
  publishes no ports and keeps its networks `internal: true`; `scripts/test-egress.sh` proves a
  container on the core network cannot reach the internet.
- **Crawled markup is text, never markup.** Rendering escapes everything and re-admits only the
  `<em>` the engine inserts; tested with live hostile documents.
- **Search works without JavaScript.** `/search` is server-rendered; scripts are enhancements.
- **Documented commands exist.** `scripts/lint-docs.sh` fails the commit if a doc names a `make`
  target or script that does not exist.

## Documentation map

Open [`docs/`](docs/) in Obsidian and start at [Home](docs/Home.md) for the graph view and
backlinks; the same files read fine on GitHub.

### Start here

- [Xustive Search Engine – Technical Specification](docs/Xustive%20Search%20Engine%20%E2%80%93%20Technical%20Specification.md) — the system specification: goals, scope, requirements, and how the parts meet them
- [Home](docs/Home.md) — the vault's map of content, with a crate → note table
- [Glossary](docs/Glossary.md) — Darija, MSA, Arabizi, wilaya, and the project's own words

### Architecture

- [API Contract](docs/Architecture/API%20Contract.md)
- [Component Map](docs/Architecture/Component%20Map.md)
- [Data Model](docs/Architecture/Data%20Model.md)
- [Deployment Topology](docs/Architecture/Deployment%20Topology.md)
- [Error Handling and Resilience](docs/Architecture/Error%20Handling%20and%20Resilience.md)
- [Observability](docs/Architecture/Observability.md)
- [Ranking and Relevance](docs/Architecture/Ranking%20and%20Relevance.md)
- [Security and Privacy](docs/Architecture/Security%20and%20Privacy.md)
- [System Architecture](docs/Architecture/System%20Architecture.md)

### Components

One note per component, each with its crate or service, its contract, and what it must never do.

- [Admin and Source Submission](docs/Components/Admin%20and%20Source%20Submission.md)
- [API Gateway](docs/Components/API%20Gateway.md)
- [Autocomplete Service](docs/Components/Autocomplete%20Service.md)
- [Content Parser](docs/Components/Content%20Parser.md)
- [Crawler Console](docs/Components/Crawler%20Console.md)
- [Crawler Orchestrator](docs/Components/Crawler%20Orchestrator.md)
- [Deduplication Service](docs/Components/Deduplication%20Service.md)
- [Enrichment Pipeline](docs/Components/Enrichment%20Pipeline.md)
- [Federation Gateway](docs/Components/Federation%20Gateway.md)
- [Fingerprint Engine](docs/Components/Fingerprint%20Engine.md)
- [Image Pipeline](docs/Components/Image%20Pipeline.md)
- [Indexer Worker](docs/Components/Indexer%20Worker.md)
- [Instant Answers](docs/Components/Instant%20Answers.md)
- [Interaction Signals](docs/Components/Interaction%20Signals.md)
- [Knowledge Store](docs/Components/Knowledge%20Store.md)
- [Language Detector](docs/Components/Language%20Detector.md)
- [Load Generator](docs/Components/Load%20Generator.md)
- [Media Extraction](docs/Components/Media%20Extraction.md)
- [Politeness and Robots](docs/Components/Politeness%20and%20Robots.md)
- [Proxy Manager](docs/Components/Proxy%20Manager.md)
- [Query Expander](docs/Components/Query%20Expander.md)
- [Query Pipeline](docs/Components/Query%20Pipeline.md)
- [Search Index](docs/Components/Search%20Index.md)
- [Sentiment Engine](docs/Components/Sentiment%20Engine.md)
- [Session Manager](docs/Components/Session%20Manager.md)
- [Signature Service](docs/Components/Signature%20Service.md)
- [Social Connector - Facebook](docs/Components/Social%20Connector%20-%20Facebook.md)
- [Social Connector - Instagram](docs/Components/Social%20Connector%20-%20Instagram.md)
- [Social Connector - TikTok](docs/Components/Social%20Connector%20-%20TikTok.md)
- [Speech to Text](docs/Components/Speech%20to%20Text.md)
- [Summarizer](docs/Components/Summarizer.md)
- [Task Queue](docs/Components/Task%20Queue.md)
- [Thumbnail Proxy](docs/Components/Thumbnail%20Proxy.md)
- [Tool Data Plane](docs/Components/Tool%20Data%20Plane.md)
- [Vector Index](docs/Components/Vector%20Index.md)
- [Web Fetcher](docs/Components/Web%20Fetcher.md)
- [Web Upstream Client](docs/Components/Web%20Upstream%20Client.md)

### Decisions (ADRs)

Why the system is the way it is. Index: [Decision Log](docs/Decisions/Decision%20Log.md).

- [ADR-0001 - Two-Plane Architecture](docs/Decisions/ADR-0001%20-%20Two-Plane%20Architecture.md)
- [ADR-0002 - Meilisearch as System of Record](docs/Decisions/ADR-0002%20-%20Meilisearch%20as%20System%20of%20Record.md)
- [ADR-0003 - Comments in a Separate Index](docs/Decisions/ADR-0003%20-%20Comments%20in%20a%20Separate%20Index.md)
- [ADR-0004 - Stream Summary Separately from Results](docs/Decisions/ADR-0004%20-%20Stream%20Summary%20Separately%20from%20Results.md)
- [ADR-0005 - Local Quantised LLM for Summaries](docs/Decisions/ADR-0005%20-%20Local%20Quantised%20LLM%20for%20Summaries.md)
- [ADR-0006 - Redis Streams for the Ingestion Pipeline](docs/Decisions/ADR-0006%20-%20Redis%20Streams%20for%20the%20Ingestion%20Pipeline.md)
- [ADR-0007 - API-First Social Access](docs/Decisions/ADR-0007%20-%20API-First%20Social%20Access.md)
- [ADR-0008 - No Query Logging](docs/Decisions/ADR-0008%20-%20No%20Query%20Logging.md)
- [ADR-0009 - Direct Collection for Social Platforms](docs/Decisions/ADR-0009%20-%20Direct%20Collection%20for%20Social%20Platforms.md)
- [ADR-0010 - Next.js for the Frontend](docs/Decisions/ADR-0010%20-%20Next.js%20for%20the%20Frontend.md)
- [ADR-0011 - Adaptive Recrawl over Static Crawling](docs/Decisions/ADR-0011%20-%20Adaptive%20Recrawl%20over%20Static%20Crawling.md)
- [ADR-0012 - Discovery-Only Aggregation](docs/Decisions/ADR-0012%20-%20Discovery-Only%20Aggregation.md)
- [ADR-0013 - Direct SERP Collection for Discovery](docs/Decisions/ADR-0013%20-%20Direct%20SERP%20Collection%20for%20Discovery.md)
- [ADR-0014 - Knowledge Panel from Wikipedia via the Web Tier](docs/Decisions/ADR-0014%20-%20Knowledge%20Panel%20from%20Wikipedia%20via%20the%20Web%20Tier.md)
- [ADR-0015 - Anonymous Interaction Signals for Ranking](docs/Decisions/ADR-0015%20-%20Anonymous%20Interaction%20Signals%20for%20Ranking.md)
- [ADR-0016 - Two OCR Engines with an Optional Unlimited-OCR Sidecar](docs/Decisions/ADR-0016%20-%20Two%20OCR%20Engines%20with%20an%20Optional%20Unlimited-OCR%20Sidecar.md)
- [ADR-0017 - Query-Time Federation with External Metasearch](docs/Decisions/ADR-0017%20-%20Query-Time%20Federation%20with%20External%20Metasearch.md)
- [ADR-0018 - Anonymous Search History](docs/Decisions/ADR-0018%20-%20Anonymous%20Search%20History.md)
- [ADR-0019 - The Knowledge Layer](docs/Decisions/ADR-0019%20-%20The%20Knowledge%20Layer.md)
- [ADR-0020 - Approximate Location from a Local Database](docs/Decisions/ADR-0020%20-%20Approximate%20Location%20from%20a%20Local%20Database.md)
- [ADR-0021 - Proxied Thumbnails with Signed URLs](docs/Decisions/ADR-0021%20-%20Proxied%20Thumbnails%20with%20Signed%20URLs.md)
- [ADR-0022 - Entity Resolution Prefers Silence to a Wrong Panel](docs/Decisions/ADR-0022%20-%20Entity%20Resolution%20Prefers%20Silence%20to%20a%20Wrong%20Panel.md)
- [ADR-0023 - Live Wikidata Fallback Judged by the Local Resolver](docs/Decisions/ADR-0023%20-%20Live%20Wikidata%20Fallback%20Judged%20by%20the%20Local%20Resolver.md)
- [ADR-0024 - Two-Model Voice Transcription that Submits on Stop](docs/Decisions/ADR-0024%20-%20Two-Model%20Voice%20Transcription%20that%20Submits%20on%20Stop.md)
- [ADR-0025 - Official Exchange Rate Only](docs/Decisions/ADR-0025%20-%20Official%20Exchange%20Rate%20Only.md)
- [ADR-0026 - The Reader's Language as a Bounded Ranking Signal](docs/Decisions/ADR-0026%20-%20The%20Reader%27s%20Language%20as%20a%20Bounded%20Ranking%20Signal.md)
- [ADR-0027 - Narrow the Search Under Load Instead of Failing](docs/Decisions/ADR-0027%20-%20Narrow%20the%20Search%20Under%20Load%20Instead%20of%20Failing.md)
- [ADR-0028 - Reverse Image Search Sends Words to the Web, Never the Picture](docs/Decisions/ADR-0028%20-%20Reverse%20Image%20Search%20Sends%20Words%20to%20the%20Web%2C%20Never%20the%20Picture.md)

### User interface

Screens, states, the design language, and the rules for right-to-left and accessibility.

- [UI - Accessibility](docs/UI/UI%20-%20Accessibility.md)
- [UI - Admin Console](docs/UI/UI%20-%20Admin%20Console.md)
- [UI - Component Library](docs/UI/UI%20-%20Component%20Library.md)
- [UI - Design Language](docs/UI/UI%20-%20Design%20Language.md)
- [UI - Filters and Facets](docs/UI/UI%20-%20Filters%20and%20Facets.md)
- [UI - Frontend Architecture](docs/UI/UI%20-%20Frontend%20Architecture.md)
- [UI - Home Page](docs/UI/UI%20-%20Home%20Page.md)
- [UI - Image Search](docs/UI/UI%20-%20Image%20Search.md)
- [UI - Knowledge Panel](docs/UI/UI%20-%20Knowledge%20Panel.md)
- [UI - Results Page](docs/UI/UI%20-%20Results%20Page.md)
- [UI - RTL and Localization](docs/UI/UI%20-%20RTL%20and%20Localization.md)
- [UI - Search Verticals](docs/UI/UI%20-%20Search%20Verticals.md)
- [UI - States and Errors](docs/UI/UI%20-%20States%20and%20Errors.md)
- [UI - Tool Cards](docs/UI/UI%20-%20Tool%20Cards.md)
- [UI - Voice Search](docs/UI/UI%20-%20Voice%20Search.md)
- [UI Specification](docs/UI/UI%20Specification.md)

### Engineering and operations

- [Data Sources Registry](docs/Engineering/Data%20Sources%20Registry.md)
- [Legal and Compliance](docs/Engineering/Legal%20and%20Compliance.md)
- [Performance Budgets](docs/Engineering/Performance%20Budgets.md)
- [Running Xustive](docs/Engineering/Running%20Xustive.md)
- [Testing Strategy](docs/Engineering/Testing%20Strategy.md)
- [Operating Xustive](docs/Operations/Operating%20Xustive.md)
- [Runbooks](docs/Operations/Runbooks.md)

### Planning and problems

Milestones are living records: each task is ticked against the code, and what was settled
differently says so.

- [Milestone 0 - Foundations](docs/Planning/Milestone%200%20-%20Foundations.md)
- [Milestone 1 - Text Search MVP](docs/Planning/Milestone%201%20-%20Text%20Search%20MVP.md)
- [Milestone 1B - Frontend and Instant Answers](docs/Planning/Milestone%201B%20-%20Frontend%20and%20Instant%20Answers.md)
- [Milestone 2 - Ingestion at Scale](docs/Planning/Milestone%202%20-%20Ingestion%20at%20Scale.md)
- [Milestone 3 - Multimodal Input](docs/Planning/Milestone%203%20-%20Multimodal%20Input.md)
- [Milestone 4 - Quality and Operations](docs/Planning/Milestone%204%20-%20Quality%20and%20Operations.md)
- [Milestone 5 - Beta Launch](docs/Planning/Milestone%205%20-%20Beta%20Launch.md)
- [Milestone 6 - Adaptive Ranking from Interaction Signals](docs/Planning/Milestone%206%20-%20Adaptive%20Ranking%20from%20Interaction%20Signals.md)
- [Milestone 7 - Federated Retrieval and External Tools](docs/Planning/Milestone%207%20-%20Federated%20Retrieval%20and%20External%20Tools.md)
- [Milestone 8 - The Answer Layer](docs/Planning/Milestone%208%20-%20The%20Answer%20Layer.md)
- [Milestone 9 - Images and Videos](docs/Planning/Milestone%209%20-%20Images%20and%20Videos.md)
- [Milestone 10 - Reverse Image Search](docs/Planning/Milestone%2010%20-%20Reverse%20Image%20Search.md)
- [TODO](docs/Planning/TODO.md) — the verified current picture
- [Problems](docs/BugReports/Problems.md) — the problem register
- [2026-08-25 - Code Audit Findings](docs/BugReports/2026-08-25%20-%20Code%20Audit%20Findings.md) — the audit findings (BUG-0xx)
- [PROB-001 - Bounded Frontier and Queue](docs/BugReports/Solutions/PROB-001%20-%20Bounded%20Frontier%20and%20Queue.md) · [PROB-002 - Crawl and Index Throughput](docs/BugReports/Solutions/PROB-002%20-%20Crawl%20and%20Index%20Throughput.md) · [PROB-003 - Admin Console Coverage](docs/BugReports/Solutions/PROB-003%20-%20Admin%20Console%20Coverage.md)

## Licence

The code is intended to be MIT-licensed; a `LICENSE` file has not yet been added to the
repository (tracked in [TODO](docs/Planning/TODO.md)). Runtime dependencies are MIT, Apache-2.0
or BSD. Two data/model caveats apply and are documented in
[Legal and Compliance](docs/Engineering/Legal%20and%20Compliance.md): the default summariser model (Qwen2.5-3B,
"qwen-research") is licensed for non-commercial use — the 1.5B and 7B sizes are Apache-2.0 — and the
DB-IP City Lite location database is CC BY 4.0 and requires attribution.
