---
tags:
  - component
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

## 2. Responsibilities

| Does | Does not |
|:---|:---|
| Start, stop and restart the crawl | Bypass politeness — that is a separate, guarded flag |
| Show live throughput: fetched, indexed, failed, per minute | Replace metrics; Prometheus stays the record |
| List and inspect what has been collected | Serve as a public search interface |
| Push a URL to the front of the frontier | Let arbitrary URLs be fetched off-registry |
| Force a re-fetch or re-index of a document | Edit document content by hand |

## 3. Interface

All under `/admin`, same authorisation as the rest of the admin surface.

| Method | Path | Purpose |
|:---|:---|:---|
| `GET` | `/admin/crawler` | The console page |
| `GET` | `/admin/crawler/status` | State, throughput, per-host activity |
| `POST` | `/admin/crawler/control` | `{"action": "start" \| "stop" \| "restart"}` |
| `GET` | `/admin/crawler/events` | **SSE** — live counters, one frame per second |
| `GET` | `/admin/crawler/documents` | Paged list of what has been collected |
| `GET` | `/admin/crawler/documents/{id}` | One document: extracted text, metadata, raw fetch record |
| `POST` | `/admin/crawler/enqueue` | `{"url": …, "priority": "front" \| "normal"}` |
| `POST` | `/admin/crawler/documents/{id}/refetch` | Fetch again now, ignoring the revisit schedule |
| `POST` | `/admin/crawler/documents/{id}/reindex` | Re-run extraction and indexing on the stored raw blob |

### 3.1 Why the live view is SSE and not polling

The number people watch is the document count climbing, and a count that jumps in five-second
steps reads as a stalled crawler. One frame per second over a single connection costs less than
polling and looks like what is actually happening.

The same stream carries the per-host activity, so a host that has stopped answering is visible as
the row that stops moving rather than as an absence nobody notices.

### 3.2 Stop means stop, not pause-and-lose

`stop` drains: in-flight fetches finish, their results are indexed, and the frontier is left
intact. A stop that discarded partial work would make the control something operators avoid using,
which defeats the point of having it.

`restart` is stop-then-start with the frontier preserved. Rebuilding a frontier from seeds costs
hours of re-discovery and re-fetches every site from scratch — polite crawling makes that
expensive for the *sites*, not just for us.

## 4. Internal Design

### 4.1 The counters are derived, not accumulated

Throughput is computed from the orchestrator's own counters in Redis, which are the same ones the
Prometheus metrics read. A separate tally maintained for the console would drift from the metrics,
and the two disagreeing is worse than having only one — an operator cannot tell which is lying.

### 4.2 Enqueueing is bounded by the registry

A URL pushed to the front still goes through the same checks as any other: `SafeUrl`, the
blocklists, `robots.txt`, and the source registry. The console changes *ordering*, not
*permission*. An admin form that could fetch any URL would be an SSRF hole with a login page in
front of it, and the login is not the part that stops it.

`priority: "front"` places the URL at the head of its host's queue. It does not skip the host's
crawl-delay — one host cannot be made to answer faster by an operator being impatient.

### 4.3 Refetch versus reindex

Two different repairs, and conflating them wastes somebody's bandwidth:

- **Refetch** goes back to the site. For a page that has genuinely changed.
- **Reindex** re-runs extraction on the raw blob we already hold. For when *our* parser was wrong —
  a boilerplate rule that ate the article body, a date format we did not recognise. This is the
  common case after a parser fix, and it needs no network at all.

A bulk reindex after a parser change is therefore free to the sites we crawl, which is the reason
raw blobs are stored at all ([[Web Fetcher]] §4.7).

## 5. Configuration

| Key | Default | Notes |
|:---|:---|:---|
| `crawler.console_enabled` | `true` in dev, `false` in prod | The console is an operator tool; production access goes through the same guard as the rest of `/admin` |
| `crawler.events_interval_ms` | `1000` | Frame rate of the live stream |
| `crawler.documents_page_size` | `50` | |

## 6. Failure Modes

| Failure | Behaviour |
|:---|:---|
| Orchestrator not running | Console loads, shows `stopped`, `start` is the only enabled control |
| Redis unavailable | Console shows an explicit "cannot read crawler state" rather than zeroes — a zero looks like a healthy idle crawler |
| SSE connection drops | Client reconnects; counters are absolute, not deltas, so nothing is lost or double-counted |
| A document's raw blob has expired | Reindex is refused with a reason, refetch is offered instead |

The Redis case is the one worth stating twice. **Rendering zero for "unknown" is the failure this
whole surface exists to prevent** — an operator who sees `0 fetched, 0 failed` reasonably concludes
the crawl is idle, when the truth is that we have no idea what it is doing.

## 7. Observability

The console reads metrics; it does not replace them. Every number shown has a Prometheus
counterpart, and the console is the thing you look at when you want to *act*, not when you want to
know what happened last Tuesday.

Actions taken from the console are logged at `info` with the peer that took them — start, stop,
enqueue and refetch are all things worth being able to attribute afterwards.

## 8. Security

- Same authorisation as `/admin`, which is not exposed publicly.
- Enqueued URLs pass every check a discovered URL passes. The console reorders; it does not grant.
- Document contents are shown as **text**, never rendered as HTML. A crawled page is untrusted
  input by definition, and an admin console that renders it is a stored-XSS hole aimed at the one
  account with the most authority.

## 9. Testing

- Start, stop, restart, and assert the frontier survives a restart.
- Enqueue a blocked URL and assert it is refused, not merely deprioritised.
- Enqueue a private-address URL and assert `SafeUrl` refuses it through this path too.
- Kill Redis mid-stream and assert the console says so rather than showing zeroes.
- Assert a document's body is escaped in the detail view, with a crawled page containing a
  `<script>` tag.
- Reindex with an expired raw blob is refused with a reason.

## 10. Open Questions

- [ ] Should bulk reindex be exposed here, or stay a CLI operation? It is the natural follow-up to
      a parser fix and it is also the easiest way to saturate the index queue by accident.
- [ ] How much history does the document list need? A crawler at target volume outgrows any list a
      person can page through, and the useful view is probably "recent, plus search", not "all".

## 11. Related

[[UI - Admin Console]] · [[Crawler Orchestrator]] · [[Web Fetcher]] · [[Admin and Source Submission]] ·
[[Politeness and Robots]] · [[Indexer Worker]] · [[Observability]]
