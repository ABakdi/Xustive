---
tags:
  - planning
  - milestone
  - deployment
  - community
milestone: 14
status: specified
updated: 2026-08-29
progress: specified 2026-08-29; nothing built
---
# Milestone 14 - One Server, Many Hands

> **Goal:** three deployments of the same system, in this order. **One server** that anyone can
> stand up with a command and that runs the whole engine. **That server plus the community** —
> a person in Oran runs one command, their machine crawls the web (or lends its GPU, or both),
> and what it produces arrives already indexable and is believed only after it has been checked.
> **Many servers** — the index sharded across machines and the volunteer fleet orchestrated,
> under Kubernetes, without changing a line of the search path.
> **Exit gate:** `xustive-node join --crawl --gpu` on a clean laptop contributes verified
> documents and completed GPU jobs to a server it has never met, and both are visible on the
> console; a poisoned submission is caught by the verifier and the node's standing falls; a
> host is never crawled by two nodes at once; the same corpus answers searches from four
> Meilisearch shards with no change to the ranking code; the whole thing is one Helm install
> and one `docker compose up`.
> Parent: [[TODO]] · Previous: [[Milestone 13 - Distilled Ranking]] · Decisions:
> [[ADR-0033 - Volunteer Crawling, Verified Before It Is Believed]],
> [[ADR-0034 - Volunteer GPUs Do Batch Work, Never a Reader's Query]],
> [[ADR-0035 - Sharding in Our Own Router]] · Components:
> [[Contribution Coordinator]], [[Community Node]], [[Deployment Topology]]

## Why This Milestone Exists

The engine is one process tree on one machine. Everything that limits it now is a resource one
machine has a fixed amount of:

- **Crawl throughput is IP-bound and politeness-bound.** [[PROB-002 - Crawl and Index Throughput]]
  measured the ceiling: `min(workers, distinct ready hosts) / per-host delay`. One server is one
  IP address crawling one host at a time; a hundred volunteers are a hundred addresses crawling a
  hundred hosts at a time, at the same politeness per host.
- **The index outgrows the box.** [[Problems#PROB-004|PROB-004]] was exactly this — a 12.5 GB
  index against a 5 GiB page cache — and the fix was to give one machine more memory. That fix
  has an end; sharding does not.
- **GPU work is rationed.** The reference machine is a 4 GB Quadro T1000, which is why the
  cross-encoder measured 0.7 s a pair and stays off ([[ADR-0032]]), why summaries are a 3B model,
  and why OCR, CLIP and speech share one card. Community GPUs are the only path to that work at
  corpus scale that does not begin with a purchase order.
- **And the point of an Algerian search engine is that Algerians can run it.** A donated laptop
  in Constantine crawling Constantine's web is a better crawler than a rented server in Frankfurt
  pretending to be one.

## What already exists, and what has to be built

| Exists | Where |
|:---|:---|
| The crawl pipeline as a library — frontier, fetcher, parser, enrichment, media | `xustive-ingest`, driven by `crawld` |
| Bounded frontier with per-host politeness, budgets and ceilings | [[PROB-001 - Bounded Frontier and Queue]] |
| An index queue with batching, bisection and a dead-letter queue | `xustive-queue`, `xustive-cli worker` |
| Sidecars with narrow HTTP contracts (CLIP, OCR, STT, text-embed, reranker) | `services/*` |
| Signals recomputed server-side (quality, spam, authority, endorsement) | `xustive-ingest::enrichment`, `xustive-search::rank`, `endorse.rs` |
| A console that shows and steers | [[Milestone 12 - The Operator's Console]] |

| Missing | Consequence |
|:---|:---|
| **Any authentication at all** on the API, including `/admin` | nothing here can be exposed to the internet until it exists (T01) |
| A coordinator: enrollment, leases, submission, verification, reputation | there is no way to give work out or to check what comes back |
| A node binary and a one-line installer | a volunteer would have to build Rust and read a config file |
| Shard routing | the API talks to exactly one Meilisearch |
| Kubernetes manifests | the only deployment is `docker compose up` on one host |

## The shape of the thing

**One binary for the volunteer, two capabilities.** `xustive-node`, installed by one command,
holding the same `xustive-ingest` pipeline the server runs. `--crawl` leases hosts and returns
documents; `--gpu` leases batch jobs and returns vectors, text and labels. Either, or both, or
neither on a given day — a node that goes away is a lease that expires, which the coordinator
already has to handle for its own workers.

**The coordinator is the only thing that trusts nobody.** It hands out work, it takes back
results, and between those two it decides what to believe: structural checks on everything, a
sampled re-fetch, canary work whose answer it already knows, duplicate assignment for a slice,
and a standing per node that sets how hard it looks. Everything a node sends is *evidence*, and
every ranking-visible number is recomputed on the server from that evidence ([[ADR-0033]]).

**Politeness stays global.** A host is leased to one node at a time, with the delay the frontier
has already learned for it. Distribution buys parallelism *across* hosts, never inside one.

**Readers' queries never leave the operator's machines.** Volunteer GPUs do batch work on
documents the crawler already fetched — embeddings, OCR, descriptions, transcripts,
pre-computed summaries. Live search stays local ([[ADR-0034]]), which is a stricter line than
[[ADR-0029]] draws for services the operator chooses, because a volunteer is not a service the
operator chose.

**Sharding is ours, in the router.** Meilisearch's own sharding is Enterprise Edition; the API
already fuses several result lists and re-ranks them, so fanning out to N shards and merging is a
smaller change than a licence ([[ADR-0035]]).

## Tasks

### T01 — Authentication, before anything is exposed *(blocking)*
- **T01.1** API keys with scopes (`admin`, `contrib`, `read`) in a `keys` index, `Authorization:
  Bearer`, constant-time compare, rotation, and a startup refusal to bind a non-loopback address
  without one. The console gets a key management page.
- **T01.2** Every `/admin` route behind `admin`; the new `/contrib` routes behind `contrib`;
  everything else public and rate-limited as today.
- **T01.3** Per-key rate limits and quotas, and an audit line per admin mutation.

### T02 — The coordinator ([[Contribution Coordinator]])
- **T02.1** `POST /contrib/enroll` — invite token + node public key (Ed25519) → node id,
  capability grant, probation standing. Sybil bounds: one node per key, quotas per node and per
  /24, invites issued by the operator or by a contributor in good standing.
- **T02.2** `POST /contrib/lease` (crawl) — an exclusive **host lease**: host, up to N URLs from
  the frontier, the robots snapshot, the per-host delay, a deadline. Released, renewed or
  expired; a host is never in two live leases.
- **T02.3** `POST /contrib/submit` — a signed batch of fetched pages (URL, status, headers
  digest, fetched-at, raw text, content hash, media list). Lands in **quarantine**, never
  directly in `documents`.
- **T02.4** `POST /contrib/gpu/lease` and `/contrib/gpu/submit` — batch jobs by capability and
  VRAM, with the model pinned by digest.
- **T02.5** Standing: credibility per node, sampling rate as a function of it, fast decay on a
  failure, slow recovery; quotas scale with it.

### T03 — The verifier
- **T03.1** Structural: schema, URL inside the lease, robots decision reproduced, content hash
  over the submitted text, size and count caps, `SafeUrl` and trap detection, MIME sanity.
- **T03.2** Server-side recomputation of every ranking-visible field — language, simhash,
  quality, spam, topics, wilaya, media — from the submitted text. Node-supplied scores are
  dropped, not trusted.
- **T03.3** Sampled re-fetch (adaptive rate by standing) comparing content hash and simhash
  distance; **canary URLs** the server holds a fresh copy of; **duplicate assignment** of a
  slice of URLs to two nodes; disagreement escalates to a server fetch.
- **T03.4** Admission: a batch that passes and comes from a node above the standing threshold is
  promoted to the index queue; otherwise it waits in quarantine for review on the console.
- **T03.5** Blast-radius caps: a node may not supply more than a share of any one host's pages,
  nor more than its daily quota, and a newly enrolled node is quarantine-only.

### T04 — The community node ([[Community Node]])
- **T04.1** `xustive-node` binary: `join`, `status`, `pause`, `leave`; config in one TOML;
  keypair generated locally and never sent.
- **T04.2** Crawl mode: lease → fetch (honouring robots and the lease's delay) → parse and enrich
  locally → submit; resumable, offline-tolerant, bandwidth-capped.
- **T04.3** GPU mode: capability probe (VRAM, CUDA/CPU), model download by digest, job loop with
  timeouts and a hard VRAM ceiling; falls back to CPU where the model allows.
- **T04.4** Manners on someone else's computer: bandwidth and CPU caps, pause on battery or a
  metered connection, nice/ionice, a visible log, `xustive-node leave` that removes everything.
- **T04.5** The installer: `curl -fsSL https://get.xustive.dz | sh` → verified download of a
  signed binary, a systemd user unit (or launchd/Windows service), then `xustive-node join`.

### T05 — Sharding in the router ([[ADR-0035]])
- **T05.1** `search.shards` — a list of Meilisearch URLs; rendezvous hashing on the document id
  picks the owner, so adding a shard moves ~1/N of the corpus and no more.
- **T05.2** Reads fan out in parallel with the existing pool limit, merge on `_rankingScore`,
  sum facets and totals, then the existing re-rank runs over the merged pool — unchanged.
- **T05.3** Writes route by id: the worker, the endorse sink, the hits counters, takedowns.
- **T05.4** A slow shard degrades like every other stage in the deadline ladder — partial
  results, `degraded{stage="shard"}` — never a failed search.
- **T05.5** `xustive shard status|add|rebalance`, and `migrate` applies settings to every shard.

### T06 — Kubernetes
- **T06.1** A Helm chart: API (HPA), coordinator, federator, worker (KEDA on the index stream),
  crawler, sidecars on a GPU node pool, Meilisearch shards as a StatefulSet with a PVC each,
  Redis, Qdrant, Prometheus/Grafana.
- **T06.2** NetworkPolicies that reproduce the compose segregation — `core` has no egress; only
  the federator and the crawler leave the cluster.
- **T06.3** The contributor ingress is separate from the public one: its own host, its own rate
  limits, its own key scope.
- **T06.4** Per-shard dumps to object storage, and a restore runbook that names its RTO.
- **T06.5** The memory rule from [[Problems#PROB-004|PROB-004]] as a chart value with a note:
  each shard's limit must exceed that shard's `usedDatabaseSize`.

### T07 — Docs, console, gates
- Console: a **Community** page — nodes, standing, work in flight, quarantine review, GPU jobs,
  contribution over time. Public `/community` page with the leaderboard and the install command.
- [[Deployment Topology]] §11–§12; [[Running a Community Node]]; [[Operating Xustive]] gains the
  coordinator's runbooks; README's feature table and milestone list.
- Gates: a poisoned-submission test fixture, a two-nodes-one-host test, a shard fan-out test with
  a shard down, and a node that lies about a GPU job.

## Out of scope

- Payment, tokens or any transferable credit. Standing is a number that decides how much work a
  node is trusted with, not an asset.
- Volunteer *serving* — a node answering searches for the public. The index is the operator's and
  stays there.
- Model training on volunteer hardware ([[Petals]]-style split inference). Batch jobs only.
- Federated indexes: several independent operators pooling corpora. Different milestone,
  different trust model.

## Acceptance

| Check | How |
|:---|:---|
| One command joins | clean VM: installer → `join --crawl` → documents admitted within a lease |
| A host is never doubly crawled | two nodes, one host in the frontier: the second lease is refused |
| Poison is caught | a fixture that alters text after fetch: the re-fetch check rejects the batch and standing drops |
| GPU jobs are verified | a node returning zero vectors fails the canary and is suspended |
| Sharding is invisible | the eval harness scores the same on 1 shard and on 4 |
| A shard down is a degradation | kill one shard: results and `degraded{stage="shard"}`, not a 5xx |
| Nothing is open | every `/admin` and `/contrib` route refuses an unkeyed request |

## Related

[[TODO]] · [[Contribution Coordinator]] · [[Community Node]] · [[Deployment Topology]] ·
[[Crawler Orchestrator]] · [[Task Queue]] · [[Performance Budgets]]
