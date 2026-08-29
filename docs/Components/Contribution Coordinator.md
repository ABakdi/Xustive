---
tags:
  - component
  - community
  - crawling
  - security
status: specified
date: 2026-08-29
milestone: 14
---
# Contribution Coordinator

> Component C25 · Milestone: [[Milestone 14 - One Server, Many Hands]] · Decisions:
> [[ADR-0033 - Volunteer Crawling, Verified Before It Is Believed]],
> [[ADR-0034 - Volunteer GPUs Do Batch Work, Never a Reader's Query]] · Peers:
> [[Community Node]], [[Crawler Orchestrator]], [[Task Queue]], [[Indexer Worker]]

## 0. Where it lives

A module in the API process (`crates/xustive-api/src/contrib/`) behind its own route group,
its own key scope and its own rate limits — not a separate service. It needs the frontier
(Redis), the index queue (Redis) and Meilisearch, which the API already holds, and splitting it
out would duplicate all three connections to save nothing. In Kubernetes it is the same image
with `--role coordinator`, scaled separately and behind a different ingress
([[Deployment Topology]] §12).

## 1. Purpose

Give work to machines nobody controls, and decide what to believe when it comes back. Everything
here follows from one sentence in [[ADR-0033]]: *a node supplies evidence, never conclusions.*

## 2. Responsibilities

| Does | Does not |
|:---|:---|
| Enrol nodes, issue and revoke credentials | Vet people; standing is earned by work, not by identity |
| Lease hosts exclusively and enforce global politeness | Trust a node to pace itself |
| Accept batches into quarantine | Write to `documents` directly |
| Verify: structure, recomputation, re-fetch, canaries, duplicates | Re-fetch everything |
| Keep standing, quotas and blast-radius caps | Punish slowness — a lease simply expires |
| Hand out GPU batch jobs and check their results | Send anything a reader typed ([[ADR-0034]]) |

## 3. Interface

All routes take `Authorization: Bearer <node-key>` (scope `contrib`) **and** an
`X-Xustive-Signature` over the canonical request body, made with the node's Ed25519 key. The key
is the identity; the bearer token is revocable per node.

```
POST /api/v1/contrib/enroll        {invite, pubkey, agent, capabilities}
                                   -> {node_id, token, standing, quotas, probation: true}
POST /api/v1/contrib/lease         {kinds: ["crawl"], budget: {urls, seconds}}
                                   -> {lease_id, host, delay_ms, robots, urls[], expires_at}
POST /api/v1/contrib/renew         {lease_id}            -> {expires_at}
POST /api/v1/contrib/submit        {lease_id, pages[]}   -> {accepted, quarantined, rejected[]}
POST /api/v1/contrib/gpu/lease     {vram_mb, models[]}   -> {job_id, kind, model, digest, items[]}
POST /api/v1/contrib/gpu/submit    {job_id, results[]}   -> {accepted, rejected[]}
GET  /api/v1/contrib/me                                  -> standing, quotas, totals, notices
POST /api/v1/contrib/leave                               -> credentials revoked
```

A submitted page — the evidence, and nothing else:

```json
{ "url": "https://example.dz/a", "canonical": "https://example.dz/a",
  "status": 200, "fetched_at": 1787990000, "elapsed_ms": 412,
  "headers_digest": "sha256:…", "content_type": "text/html; charset=utf-8",
  "content_hash": "sha256:…", "text": "…", "title": "…",
  "links": ["https://example.dz/b"], "media": [{"kind":"image","url":"…"}],
  "robots": {"allowed": true, "source": "https://example.dz/robots.txt", "fetched_at": … } }
```

Rejected, silently dropped, or recomputed: `quality_score`, `spam_score`, `language`, `simhash`,
`topics`, `geo`, `endorsement`, `discovery`, `trust`. A node that sends them is not an error —
they are simply ignored, because a field that is never read cannot be attacked.

## 4. The lease, and why it is per host

Politeness is a property of a *host*, and the frontier has already learned each host's delay,
its robots rules and its budgets ([[PROB-001 - Bounded Frontier and Queue]]). A lease is
therefore: **one host, to one node, for a deadline**, with the URLs and the delay attached. While
it is out, no other node — and not the operator's own crawler — is given that host.

```
available ──lease──▶ leased ──submit──▶ verifying ──▶ released (next_allowed = now + delay·n)
     ▲                  │                    │
     └──── expire ──────┘                    └──▶ rejected → released, standing↓
```

Expiry is generous (a laptop closes) and cheap (the host returns to the pool). The URLs in an
expired lease are not lost; they were never removed from the frontier, only hidden.

## 5. Verification

Four checks, in cost order. A batch must pass 1 and 2; 3 and 4 are sampled by standing.

1. **Structural** — schema; every URL was in the lease; `SafeUrl` parses and no trap
   ([[Crawler Orchestrator]]); the robots decision reproduced from our own snapshot; the content
   hash equals the hash of the submitted text; size, count and MIME caps; `fetched_at` inside
   the lease window.
2. **Recomputation** — the server runs the enrichment pipeline over the submitted text and keeps
   *its* numbers. Cost is the same as a local crawl minus the fetch, which is the saving that
   makes volunteering worthwhile in the first place.
3. **Sampled re-fetch** — the server fetches a random `p` of the pages itself and compares
   content hash, then simhash distance (a page can legitimately change between two fetches; an
   identical URL returning unrelated text cannot). `p = clamp(0.25 / (1 + standing), 0.02, 1.0)`
   — everything for a new node, a fiftieth for a proven one.
4. **Canaries and duplicates** — leases carry a few URLs the server holds a fresh copy of, and a
   slice of URLs is leased to two nodes independently. A canary mismatch is decisive. A
   disagreement between two nodes is not: the server fetches and decides, and only then is
   someone wrong. This is the credibility-based scheme from the volunteer-computing literature —
   sample rather than replicate, and let history set the sample.

**Standing** is a scalar in `[0, 1]`: `+ε` per clean verified batch, `× 0.25` on any failed
check, floor 0. It sets the sampling rate, the daily quota, and whether admission is automatic or
waits for review. Fast down, slow up, on purpose.

**Blast radius**, independent of standing: a node may supply at most a fixed share of any one
host's pages, at most its daily quota of documents, and nothing at all outside a lease.

## 6. GPU jobs

Job kinds, all over content the crawler already fetched: `embed.text`, `embed.image`,
`describe.image`, `ocr`, `transcribe`, `rerank.eval`, `summarise.page`. A job carries item ids,
the text or a signed URL for the media, the model name and its SHA-256, and a deadline. It never
carries a query, a visitor id or an IP ([[ADR-0034]]).

Verification: canary items with known outputs mixed into a job; a duplicated slice; shape checks
(dimension, finite values, unit norm for embeddings, score range for reranking); and for
deterministic decoding, an exact match against the server's own occasional recomputation. Failed
jobs are re-queued to another node and cost standing.

## 7. Data

| Where | What |
|:---|:---|
| Meilisearch `contrib_nodes` | node id, pubkey, agent, capabilities, standing, quotas, totals, notices, enrolled/last-seen |
| Meilisearch `contrib_batches` | batch id, node, lease, counts, verdicts, verifier notes — the audit trail behind the console's quarantine page |
| Redis `contrib:lease:*` | live leases with TTLs; the exclusivity lock is the lease key itself |
| Redis `contrib:quota:*` | rolling daily counters |
| The index queue | where an admitted batch goes — the same `q:index` the local crawler produces to, so the worker, its batching, its bisection and its dead-letter queue are unchanged |

Quarantined text is kept for the review window (14 days) and then dropped; it is web content, not
personal data, and [[ADR-0030]]'s retention rules do not apply to it.

## 8. Failure modes

| Failure | Behaviour |
|:---|:---|
| Node vanishes mid-lease | lease expires, host returns to the pool, no penalty |
| Node floods submissions | per-key rate limit, then quota, then suspension |
| Node lies about content | re-fetch or canary catches it; batch rejected; standing × 0.25 |
| Two nodes disagree | server fetches and decides |
| Coordinator down | volunteers idle and retry with backoff; the operator's own crawler is unaffected |
| Meilisearch busy | quarantine writes retry like the endorse sink does (M13); leases keep flowing |
| A node is compromised | revoke the key; its unadmitted batches are dropped; admitted documents are re-verifiable from `contrib_batches` |

## 9. Observability

`xustive_contrib_nodes{state}`, `xustive_contrib_leases{state}`,
`xustive_contrib_pages_total{verdict}`, `xustive_contrib_verify_total{check,result}`,
`xustive_contrib_gpu_jobs_total{kind,verdict}`, standing histogram, and quarantine depth. The
console's Community page draws them; an alert fires when the rejected share crosses a threshold —
that is either an attack or a bug in a released node, and both need an operator.

## 10. Security

- No code ever travels from server to node. Work is data; models are named and digest-pinned.
- Signed requests bind a submission to a key; replay is bounded by the lease id and its window.
- Invite tokens are single-use, rate-limited per issuer, and revocable in bulk.
- The contributor ingress is separate from the public one and may be turned off entirely without
  affecting search ([[Deployment Topology]] §12).
- Nothing about a reader crosses this boundary: no query text, no visitor id, no IP.

## 11. Test plan

Unit: lease exclusivity under concurrency; standing arithmetic; sampling rate by standing; the
structural checks against a corpus of malformed and hostile submissions. Integration: a fake node
that alters text after fetching (must be caught), one that returns canaries wrong (caught at
once), one that submits URLs it was not leased (rejected), two nodes racing for one host (one
lease), a node that disappears (expiry). Load: a thousand nodes' worth of leases against one
coordinator.

## 12. Open questions

- Should a node be allowed to *choose* hosts (a volunteer who wants their region crawled), and
  what does that do to frontier priority?
- Does a browser-extension node ([[Milestone 14 - One Server, Many Hands]] out-of-scope) deserve
  a lower-trust tier rather than none?
- How is standing carried across a reinstall, given the keypair is local and disposable?

## Related

[[Community Node]] · [[Crawler Orchestrator]] · [[Task Queue]] · [[Indexer Worker]] ·
[[UI - Admin Console]] · [[Deployment Topology]]
