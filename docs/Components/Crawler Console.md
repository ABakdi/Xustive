---
tags:
  - component
  - operations
binary: xustive-api (JSON) + web (pages)
status: built
updated: 2026-08-27
---

# Crawler Console

> The operator's window into the crawler: what it is doing right now, what it has collected, and
> the controls to change either. Driven by the [[Crawler Orchestrator]].
>
> **Behaviour and endpoints live here; the interface is [[UI - Admin Console]]**, which covers the
> sidebar shell, the sections, and the performance budgets. Split because the endpoints are worth
> specifying independently of how they are drawn.

---

## 1. Purpose

A crawler that runs unattended is a crawler nobody can see. The failure that matters is not a
crash — a crash is loud — it is the crawl that keeps running while quietly collecting nothing, or
collecting the same page four hundred times, or stuck behind one host that stopped answering.

None of that shows up in a document count alone. This is the surface that makes it visible and
gives someone a way to intervene without a deploy.

## 2. Where it lives today

| Piece | Path |
|:---|:---|
| JSON endpoints | `crates/xustive-api/src/admin_crawler.rs`, mounted under `/api/v1/admin/crawler/*` |
| Queue endpoints (backlog, DLQ replay/drop) | `crates/xustive-api/src/admin_queue.rs` — [[Task Queue]] |
| Counters the endpoints read | `crates/xustive-ingest/src/crawl_stats.rs` (`CrawlStats`, `Snapshot`) |
| Pages | `web/app/(operator)/admin/{live,documents,sources,sources/health,discovery,weak-coverage,queue}` |

The Rust side is **JSON only**. The first version rendered HTML from the API; the console is now a
set of Next.js pages that reach these endpoints through the same `/api/v1/*` proxy as the search
UI, so the frontend lives in one place. `/admin/crawler` is kept as a redirect to `/admin/live`
because that was the old live-view URL.

## 3. Responsibilities

| Does | Does not |
|:---|:---|
| Pause and resume the crawl | Start or stop the `crawld` process — that is the host's job |
| Show live throughput: fetched, revisited, parsed, indexed, failed, images, videos | Replace metrics; Prometheus stays the record |
| List and inspect what has been collected, by provenance and media kind | Serve as a public search interface |
| Push a URL into the frontier, optionally to the front | Let arbitrary URLs be fetched off-registry |
| Bypass politeness — a separate, guarded flag (`POST /politeness`) | Edit document content by hand |

## 4. Interface

All under `/api/v1/admin`, same authorisation as the rest of the admin surface
([[Admin and Source Submission]]).

| Method | Path | Purpose |
|:---|:---|:---|
| `GET` | `/crawler/status` | One `Snapshot`: state, pause flag, counters, recent URLs, per-host last-fetch, frontier depth |
| `GET` | `/crawler/events` | **SSE** — the same snapshot, one frame per second, keep-alive every 15 s |
| `GET` | `/crawler/documents` | Paged list from the index; filters `q`, `host`, `lang`, `channel`, `media=image\|video`, `page` |
| `POST` | `/crawler/enqueue` | `{"url": …, "front": bool}` — queue as a trusted seed; `front` promotes it to the head of its host queue |
| `POST` | `/crawler/pause` | `{"paused": bool}` — hold or release the crawl (PROB-003) |
| `GET/POST` | `/crawler/sources`, `POST /crawler/sources/remove` | The seed list, read and edited in place |
| `GET` | `/crawler/sources/health` | Per-source quality counters ([[Data Sources Registry]] §7) |
| `POST` | `/crawler/registry` | Approve / activate / disable a registry row |
| `GET` | `/crawler/channels` | Per-discovery-channel yield (seed, link, sitemap, federation, SERP…) |
| `GET` | `/crawler/weak-coverage`, `POST …/forget` | The query-driven discovery queue and its dismiss button |

### 4.1 Why the live view is SSE and not polling

The number people watch is the document count climbing, and a count that jumps in five-second
steps reads as a stalled crawler. One frame per second over a single connection costs less than
polling and looks like what is actually happening. One connection carries every live number on the
page; several would reconnect in a storm whenever the API restarts and drift apart between frames.

The same stream carries the per-host activity, so a host that has stopped answering is visible as
the row that stops moving rather than as an absence nobody notices.

### 4.2 Pause means hold, not lose

The console has no start/stop/restart. `crawld` is a process the host supervises; what the console
controls is whether the running workers *claim*. `pause` sets a flag in Redis beside the crawl
state; every worker polls it in its guard probe (effect within seconds) and it survives restarts of
both the console and the crawler. Pausing holds claims only — in-flight fetches finish, nothing is
dropped, the frontier is untouched, so resuming costs nothing. The snapshot's `paused` is shown
apart from `state` because a deliberately held crawl must not read as idle or broken.

Rebuilding a frontier from seeds costs hours of re-discovery and re-fetches every site from
scratch — polite crawling makes that expensive for the *sites*, not just for us — which is why the
only thing that clears it is `crawld --reset`, on purpose, from a terminal.

## 5. Internal Design

### 5.1 The counters are derived, not accumulated

Throughput is computed from the orchestrator's own counters in Redis (`CrawlStats`), which are the
same ones the Prometheus metrics read. A separate tally maintained for the console would drift from
the metrics, and the two disagreeing is worse than having only one — an operator cannot tell which
is lying. Frames carry **absolute** values, never deltas, so a client that misses a frame loses
nothing.

`fetched` is split into fresh discovery and `revisited`, so the page shows whether the crawl is
growing the corpus or keeping it current. `images` and `videos` are counted apart from pages (M9).
`waiting`, `inflight` and `deferred` (pages booked for a revisit) come from the frontier itself.

### 5.2 Enqueueing is bounded by the same rules as discovery

A URL pushed from the console still goes through `SafeUrl`, the trap detectors and URL
canonicalisation. The console changes *ordering*, not *permission*. An admin form that could fetch
any URL would be an SSRF hole with a login page in front of it, and the login is not the part that
stops it. An operator-typed URL is queued as a seed with trust 100 and depth 0; `front` promotes it
to the head of its host's queue but does not skip the host's crawl-delay — one host cannot be made
to answer faster by an operator being impatient.

### 5.3 Document list from the product index

The list is a Meilisearch query against the documents index, sorted `crawled_at:desc`, so it is the
same engine and fast at any corpus size. Two facets are computed over the scope (`host`/`lang`)
*without* the drill-in filters: `composition` by `discovery` channel — the "crawler vs external
tools" breakdown — and `media` by `media.type`. Keeping the facets scope-only means the breakdown
always shows every channel and kind to pick from. Filters use the index's own field names
(`domain`, `language`); the first version guessed `host` and `lang`, and Meilisearch silently
matched nothing rather than erroring. `www.` is stripped and the host lower-cased before filtering
for the same reason.

### 5.4 Refetch versus reindex

Two different repairs, and conflating them wastes somebody's bandwidth:

- **Refetch** goes back to the site. For a page that has genuinely changed. Today: enqueue it.
- **Reindex** re-runs extraction on the raw body we already hold. For when *our* parser was wrong.
  This needs no network, which is the reason raw bodies are stored at all ([[Web Fetcher]] §4.6).

Neither is a per-document console button. What exists is CLI: `xustive-cli media-repass`
re-extracts `media[]` from the raw store without refetching (M9), and `xustive-cli reindex`
rebuilds the index into a staging copy and swaps it atomically. A per-document `refetch`/`reindex`
endpoint is **not built** (2026-08-27) — see §9.

## 6. Configuration

| Key | Default | Notes |
|:---|:---|:---|
| `crawl.documents_page_size` | `50` | Rows per page in the document list |
| `crawl.raw_ttl_days` | `0` (off) | How long raw bodies are kept for reindexing; 0 disables the store |

There is no `console_enabled` and no frame-rate knob: the frame interval is a one-second constant
in `admin_crawler.rs`, and the console is gated by the admin authorisation, not a switch.

## 7. Failure Modes

| Failure | Behaviour |
|:---|:---|
| `crawld` not running | Console loads, shows the last stored state; pause still toggles the flag |
| Redis unavailable | `Snapshot { unavailable: true, state: "unknown" }` — the page says "cannot read crawler state" rather than zeroes |
| Redis was down at API start | The stats connection is retried once on the next request and cached, so the Live page self-heals |
| SSE connection drops | Client reconnects; counters are absolute, so nothing is lost or double-counted |
| Index unavailable for the document list | 503 `index_unavailable`; a facet failure alone is logged and the list still renders |
| Enqueue with an unsafe or trap URL | 400 `unsafe_url` / `trap`, nothing queued |

The Redis case is the one worth stating twice. **Rendering zero for "unknown" is the failure this
whole surface exists to prevent** — an operator who sees `0 fetched, 0 failed` reasonably concludes
the crawl is idle, when the truth is that we have no idea what it is doing.

## 8. Observability and security

The console reads metrics; it does not replace them. Every number shown has a Prometheus
counterpart ([[Observability]]). Pause toggles log at `warn`, enqueues at `info` with the peer, so
they can be attributed afterwards. Document contents are shown as **text**, never rendered as
HTML — a crawled page is untrusted input, and an admin console that renders it is a stored-XSS
hole aimed at the one account with the most authority.

## 9. Open Questions

- [ ] Per-document refetch/reindex buttons: the raw store makes reindex free, but a bulk reindex
      is also the easiest way to saturate the index queue by accident. CLI-only for now.
- [ ] How much history does the document list need? The useful view is "recent, plus search", which
      is what the index-backed list already is; an explicit retention rule is still open.

## Related

[[UI - Admin Console]] · [[Crawler Orchestrator]] · [[Web Fetcher]] · [[Admin and Source Submission]] ·
[[Politeness and Robots]] · [[Indexer Worker]] · [[Task Queue]] · [[Observability]] · [[Problems]]
