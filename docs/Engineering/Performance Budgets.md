---
tags:
  - engineering
  - performance
type: reference
status: specified
updated: 2026-08-27
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

**Where the numbers are actually being taken today (2026-08-27):** one development box with a
Quadro T1000 (4 GB), Meilisearch indexing a crawl backlog while serving, and `make load` from
`xustive-loadgen` against it ([[Load Generator]]). Nothing below has been measured on the reference
host; the budgets stand, the measurements are a different column.

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

### 2a. Deadlines as configured (verified 2026-08-27)

The budget is what we aim for; the deadline ladder is what the code enforces when it is missed.
Each stage has its own timeout inside the request's, so the last say is always the API's.

| Knob | dev | staging / prod / ci | Where |
|:---|:---|:---|:---|
| `api.timeout_search_ms` — the request deadline | **2 500** | 1 500 | `config/*.toml` |
| `search.timeout_ms` — one Meilisearch call | **1 200** | 800 | `config/*.toml` |
| `SEARCH_GRACE_MS` — added to the request deadline for the server-level timeout layer, so the handler's own deadline fires first and can degrade instead of the layer cutting it off | 1 000 | 1 000 | `crates/xustive-api/src/lib.rs` |
| `api.timeout_suggest_ms` | 150 | 150 | `config/*.toml` |
| `ml.deadline_ms` — summary generation | 30 000 | 30 000 | `MlConfig` default |
| Next.js `proxyTimeout` — the rewrite proxy in front of the API | 90 s | 90 s | `web/next.config.ts`; sized to outlast the summary deadline, so the API bounds the wait, not the proxy |

The dev numbers went up after **BUG-041**
([[ADR-0027 - Narrow the Search Under Load Instead of Failing]]): with 800 ms the engine tripped
"That search took too long" whenever it was also indexing. Now a retrieval timeout **narrows** rather than fails — the 200-candidate query with
facets and highlighting is retried as a page's worth with nothing extra (~30 ms even under load),
`facets_degraded` is set so the UI drops the chips, and `xustive_degraded_total{stage="retrieval"}`
counts it. Worse ranking, but results — a reader cannot tell an empty search from an outage.

The alert (`SearchLatencyHigh`, [[Runbooks]]) pages at p95 > **400 ms** for 10 minutes, twice the
budget: the budget is the design target, the alert is the point at which one search in twenty is
visibly slow.

## 3. Other Endpoints

| Endpoint | Budget | Note |
|:---|:---|:---|
| `/suggest` | **40 ms** p95, 80 ms p99 | fires per keystroke; the highest-QPS route |
| `/search/summary` TTFT | ~~800 ms~~ n/a | not streamed; see [[Summarizer]] §3 |
| `/search/summary` complete | **2 500 ms** — *not met on CPU*; the hard deadline is `ml.deadline_ms` = 30 s | never blocks results |
| `/search/voice` (5 s audio) | **1 500 ms** | includes decode + inference. Measured on the T1000 (10 s Arabic clip, `int8_float16`): partial 0.35–0.5 s, final 1.0–1.5 s ([[UI - Voice Search]]) |
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
| JS, home (gz) | **185 KB** as enforced | `scripts/bundle-budget.sh` (`make ui-gates`) |
| JS, results (gz) | **195 KB** as enforced | same |
| CSS (gz) | **20 KB** | same |
| Fonts | RTL 95 KB · LTR 50 KB | same |
| LCP | 2.0 s | Lighthouse CI — ❌ not set up (2026-08-27) |
| INP | 200 ms | |
| CLS | **0.05** | the summary slot reserves *no* height and stays hidden until filled — most summaries never arrive, and a placeholder that collapses moves the results ([[UI - Results Page]] §2) |
| DOM nodes, results page | ≤ 1 500 | |
| Render 20 result cards | ≤ 16 ms | one frame |

> The original spec's 25 KB JS / 60 KB first load are gone: the Next.js runtime alone is ~152 KB
> gzipped before a line of this project's code loads ([[ADR-0010 - Next.js for the Frontend]]);
> our own components are about 19 KB of the total. The script fails the gate rather than warning —
> a budget that only warns is a number nobody reads. The no-JavaScript path
> (`scripts/no-js-check.sh`) is the other half of the same gate ([[UI - Frontend Architecture]] §7).

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

### GPU budget on the development card (2026-08-27)

There is one GPU in the picture today, a **Quadro T1000 with 4 GB**, and it is shared — with the
desktop. Everything must also run CPU-only; the device is switchable from the admin page
(`/admin/device`). What each tenant holds, measured:

| Tenant | VRAM | Note |
|:---|:---|:---|
| `xustive-api` (summariser via the `cuda` feature, `ml.gpu_layers = -1`) | **~1.6 GB** | decides layers from free memory at load |
| STT sidecar (`base` + `small`, `int8_float16`) | **~0.75 GB** (756 MB for both) | was 1.5 GB at float32, which OOM'd the final pass's beam search under the API's share — the quantisation was the fix ([[UI - Voice Search]]) |
| OCR sidecar (Unlimited-OCR, 3B VLM) | ≥ 8 GB | does **not** fit alongside; profile `ocr` is for a bigger card |
| the desktop | the rest | |

Above ~2.4 GB combined the card is full; anything new on the GPU displaces one of these.

---

## 8. Availability

| SLO | Target | Monthly budget |
|:---|:---|:---|
| Search availability | 99.5 % | 3 h 39 m |
| Search p95 ≤ 200 ms | 99 % of 5-min windows | |
| Summary delivered ≤ 2.5 s | 95 % of requests that ask | ❌ 0 % on CPU: measured 16.5 s (1.5B) to 27.1 s (3B). See [[Summarizer]] §8 — the estimate this budget was built on was an order of magnitude optimistic. GPU offload is the intended fix. |
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
- [ ] Is 2.5 s the right summary budget, or would users prefer a faster, shorter summary? Now
      pressing rather than theoretical: on CPU the choice is between a shorter summary and no
      summary, since `max_tokens` is the one knob with a linear effect on total time.
- [ ] Should the client budget assume 3G in 2026, or has the realistic floor moved to 4G?

## Related

[[Observability]] · [[Testing Strategy]] · [[Deployment Topology]] · [[System Architecture]] ·
[[UI Specification]] · [[Error Handling and Resilience]] · [[Load Generator]] ·
[[ADR-0027 - Narrow the Search Under Load Instead of Failing]]
