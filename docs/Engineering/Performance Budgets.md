---
tags:
  - engineering
  - performance
type: reference
status: specified
updated: 2026-08-06
---

# Performance Budgets

> The single source of truth for every latency, throughput, and size number in this vault. Where a
> component note disagrees with this table, **this note wins** and the component note is corrected.
> Parent: [[Home]] · Measured by [[Testing Strategy]] §10 · Alerted on by [[Observability]] §7

---

## 1. Reference Conditions

Budgets are meaningless without stated conditions. All numbers below assume:

| Condition | Value |
|:---|:---|
| Corpus | 10M documents, 50M comments, 5M image vectors |
| Hardware | 32 vCPU / 128 GB RAM / NVMe, no GPU |
| Load | 100 rps search sustained, 500 rps peak |
| Client | mid-range Android (≈ Snapdragon 6-series), 3G/HSPA, 200 ms RTT |
| Percentile | p95 unless stated |

Change any of these and the numbers change. A budget met on a developer laptop with 10k documents
proves nothing.

---

## 2. Search Latency

| Stage | Budget | Owner |
|:---|:---|:---|
| Gateway middleware | 2 ms | [[API Gateway]] |
| Query normalisation | 1 ms | [[Query Pipeline]] |
| Language detection | 3 ms | [[Language Detector]] |
| Query expansion | 8 ms | [[Query Expander]] |
| Meilisearch multi-search | 50 ms | [[Search Index]] |
| Facet computation | 20 ms | [[Search Index]] |
| Merge + re-rank (200 candidates) | 15 ms | [[Query Pipeline]] |
| Serialisation | 5 ms | [[API Gateway]] |
| **Total server-side `/search`** | **≤ 200 ms** | |
| Network (3G, 200 ms RTT) | ~250 ms | — |
| **Perceived time to results** | **≤ 500 ms** | |

Headroom: the stages sum to ~104 ms against a 200 ms budget. That slack absorbs GC pauses, cold
caches, and contention with indexing — it is not spare capacity to spend on new features without a
measurement.

## 3. Other Endpoints

| Endpoint | Budget | Note |
|:---|:---|:---|
| `/suggest` | **40 ms** p95, 80 ms p99 | fires per keystroke; the highest-QPS route |
| `/search/summary` TTFT | **800 ms** | perceived summary responsiveness |
| `/search/summary` complete | **2 500 ms** | never blocks results |
| `/search/voice` (5 s audio) | **1 500 ms** | includes decode + inference |
| `/search/image` | **500 ms** | OCR or ANN path |
| `/healthz` | 5 ms | |

---

## 4. Ingestion Throughput

| Stage | Budget | Owner |
|:---|:---|:---|
| Static fetch | ≥ 100 pages/min/worker | [[Web Fetcher]] |
| Headless render | ≤ 12 s each, ≤ 10 % of fetches | [[Web Fetcher]] |
| Social API fetch | ≥ 200–300 posts/min/worker | connectors |
| Parse | ≥ 200 docs/s/worker | [[Content Parser]] |
| Dedup verdict | ≥ 300 docs/s/worker | [[Deduplication Service]] |
| Enrich (text only) | ≥ 100 docs/s/worker | [[Enrichment Pipeline]] |
| Enrich (with 1 image) | ≥ 10 docs/s/worker | [[Enrichment Pipeline]] |
| Index | ≥ 2 000 docs/s/worker | [[Indexer Worker]] |
| **End-to-end: fetched → searchable** | **≤ 5 min p95** | |

The end-to-end number is the one users feel, and it is dominated by queue wait, not by processing.
At green queue pressure it should be well under a minute; 5 minutes is the budget under load.

---

## 5. Index Freshness

| Source tier | Target staleness |
|:---|:---|
| Tier A (major news, high-value social) | ≤ 6 h |
| Tier B | ≤ 24 h |
| Tier C | ≤ 7 days |
| Median across the corpus | ≤ 24 h |

Alert `IndexStale` fires when the median `crawled_at` age exceeds 24 h
([[Observability]] §6).

---

## 6. Client Budgets

| Metric | Budget | Enforced by |
|:---|:---|:---|
| HTML (home, gz) | 12 KB | `bundlesize` |
| CSS (gz) | 18 KB | `bundlesize` |
| JS (gz) | 25 KB | `bundlesize` |
| Total first load | 60 KB | |
| Requests, first load | ≤ 6 | |
| LCP | 2.0 s | Lighthouse CI |
| INP | 200 ms | |
| CLS | **0.05** | the summary block reserves height ([[UI - Results Page]] §2) |
| DOM nodes, results page | ≤ 1 500 | |
| Render 20 result cards | ≤ 16 ms | one frame |

---

## 7. Resource Budgets

| Service | RAM | CPU | Disk |
|:---|:---|:---|:---|
| `xustive-api` (per replica) | ≤ 512 MB | 2 vCPU | — |
| `xustive-ml` (per replica) | ≤ 8 GB | 6 vCPU | models 4 GB (ro) |
| `xustive-crawler` (per replica) | ≤ 1 GB | 1 vCPU | — |
| `xustive-worker` (per replica) | ≤ 3 GB | 2 vCPU | — |
| `meilisearch` | ≤ 32 GB | 8 vCPU | 180–260 GB @ 10M docs |
| `qdrant` | ≤ 8 GB | 4 vCPU | ~11 GB @ 5M vectors |
| `redis` | ≤ 6 GB | 2 vCPU | AOF ~2 GB |

Model memory: Whisper small ~1.5 GB · Summariser 3B Q4 ~4 GB · CLIP ~400 MB · DziriBERT ~800 MB.
Loading all four in one `xustive-ml` replica is why its ceiling is 8 GB and its floor is not much
lower.

---

## 8. Availability

| SLO | Target | Monthly budget |
|:---|:---|:---|
| Search availability | 99.5 % | 3 h 39 m |
| Search p95 ≤ 200 ms | 99 % of 5-min windows | |
| Summary delivered ≤ 2.5 s | 95 % of requests that ask | |
| Ingestion availability | 95 % | ingestion downtime does not affect search |

Ingestion has a deliberately loose SLO: it is the fragile plane, and [[System Architecture]] §1 is
built so that its failures cost freshness, never availability.

---

## 9. When a Budget Is Missed

1. **Measure before optimising.** `xustive_search_duration_seconds{stage}` says which stage grew;
   guessing wastes days.
2. **Check the interaction cases first** — most regressions are contention (indexing vs search), not
   an algorithm getting slower in isolation.
3. **Widening a budget is a decision, not a fix.** It requires a [[Decision Log]] entry saying what
   was traded for it.
4. **Regressions block release.** A PR that pushes p95 past budget does not merge because it is
   "only 10 ms" — ten of those is the whole budget.

---

## 10. Open Questions

- [ ] Are these numbers achievable at 10M documents on one Meilisearch node, or does the search
      budget force a read replica? **Must be answered with a real load test during
      [[Milestone 4 - Quality and Operations]], not assumed.**
- [ ] Is 2.5 s the right summary budget, or would users prefer a faster, shorter summary?
- [ ] Should the client budget assume 3G in 2026, or has the realistic floor moved to 4G?

## Related

[[Observability]] · [[Testing Strategy]] · [[Deployment Topology]] · [[System Architecture]] ·
[[UI Specification]] · [[Error Handling and Resilience]]
