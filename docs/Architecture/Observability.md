---
tags:
  - architecture
  - ops
type: architecture
status: specified
updated: 2026-08-06
---

# Observability

> Metrics, logs, traces, dashboards, and alerts. Constrained hard by [[Security and Privacy]]:
> **no query text, no user identifiers, ever** — not in a log line, not in a metric label, not in a
> span attribute.

---

## 1. The Privacy Constraint First

| Forbidden | Allowed instead |
|:---|:---|
| the raw query string | `query_len_bucket`, `query_lang`, `token_count` |
| client IP | truncated salted hash, in-memory only, for rate limiting |
| `user_id`, cookie, session | nothing — there are none |
| result URLs clicked | nothing — there is no click tracking |
| collection credentials, cookies, TOTP seeds | `identity_id` only ([[Session Manager]] §4.8) |

A CI lint (`scripts/lint-telemetry.sh`) greps for `query`, `q =`, `transcript`, `ocr_text`,
`password`, `cookie`, `credentials`, and `totp` inside `tracing::` macro arguments and fails the
build. See [[Testing Strategy]].

Note the two distinct reasons: query fields are forbidden because of the promise to users
([[ADR-0008 - No Query Logging]]); credential fields because identity secrets are the highest-value
target in the system. Both land in the same lint.

---

## 2. Metrics (Prometheus)

Naming: `xustive_<subsystem>_<metric>_<unit>`. Labels are **bounded** — never a URL, never a query.

### Serving

| Metric | Type | Labels | Purpose |
|:---|:---|:---|:---|
| `xustive_http_requests_total` | counter | `route`, `method`, `status` | traffic + error rate |
| `xustive_http_duration_seconds` | histogram | `route` | latency SLO |
| `xustive_search_duration_seconds` | histogram | `stage` (`detect`,`expand`,`retrieve`,`rerank`) | where time goes |
| `xustive_search_results_total` | histogram | `lang` | zero-result detection |
| `xustive_search_zero_results_total` | counter | `lang` | relevance health |
| `xustive_summary_duration_seconds` | histogram | — | [[Summarizer]] |
| `xustive_summary_dropped_total` | counter | `reason` | load shedding |
| `xustive_ratelimit_rejected_total` | counter | `route` | abuse |
| `xustive_stt_duration_seconds` | histogram | — | [[Speech to Text]] |
| `xustive_image_duration_seconds` | histogram | `mode` | [[Image Pipeline]] |

### Ingestion

| Metric | Type | Labels | Purpose |
|:---|:---|:---|:---|
| `xustive_fetch_total` | counter | `source_type`, `outcome` | crawl health |
| `xustive_fetch_duration_seconds` | histogram | `source_type`, `method` | |
| `xustive_queue_depth` | gauge | `stream` | **the** backpressure signal |
| `xustive_queue_lag_seconds` | gauge | `stream` | oldest unacked message age |
| `xustive_stage_duration_seconds` | histogram | `stage` | pipeline throughput |
| `xustive_docs_indexed_total` | counter | `source_type` | growth |
| `xustive_dedup_rejected_total` | counter | `kind` (`exact`,`near`,`phash`) | [[Deduplication Service]] |
| `xustive_dlq_total` | counter | `stage`, `error_class` | poison messages |
| `xustive_proxy_healthy` | gauge | `pool` | [[Proxy Manager]] |
| `xustive_proxy_banned_total` | counter | `pool`, `platform` | detection pressure |
| `xustive_robots_blocked_total` | counter | `reason` | [[Politeness and Robots]] |

### Collection

Added by [[ADR-0009 - Direct Collection for Social Platforms]]. These are **tier-1 operational
signals** — detection damage compounds silently, so they page rather than ticket.

| Metric | Type | Labels | Purpose |
|:---|:---|:---|:---|
| `xustive_identity_pool_size` | gauge | `platform`, `tier` | [[Session Manager]] |
| `xustive_identity_lifespan_days` | histogram | `platform` | **the health metric that matters** |
| `xustive_challenge_total` | counter | `platform`, `kind` | detection pressure |
| `xustive_ban_total` | counter | `platform` | |
| `xustive_empty_response_total` | counter | `platform`, `source` | silent cloaking |
| `xustive_canary_status` | gauge | `platform` | ground truth vs cloaking |
| `xustive_signer_failure_rate` | gauge | `platform` | [[Signature Service]] |
| `xustive_signer_version_age_days` | gauge | `platform` | drift warning |
| `xustive_fp_drift_total` | counter | `layer` | [[Fingerprint Engine]] self-verify |
| `xustive_fp_webrtc_leak_total` | counter | `profile` | **must be 0** |
| `xustive_proxy_egress_mismatch_total` | counter | `pool` | **must be 0** |
| `xustive_proxy_bandwidth_bytes` | counter | `pool` | residential cost driver |
| `xustive_proxy_cost_per_1k_docs` | gauge | `source` | source viability |
| `xustive_access_path_total` | counter | `platform`, `path` | path-ladder health |

### Resource

`xustive_model_load_seconds`, `xustive_model_memory_bytes`, plus standard process/Go/Rust collectors
and `node_exporter` for the host.

---

## 3. Logs

Structured JSON via `tracing` + `tracing-subscriber`. One line per event, never multi-line.

```json
{"ts":"2026-08-06T10:12:03.481Z","level":"WARN","target":"xustive_crawler::fetch",
 "request_id":null,"trace_id":"01J8ZK…","source_id":"elkhabar-dz","host":"elkhabar.com",
 "event":"fetch_retry","attempt":2,"status":503,"proxy_pool":"dz-res","msg":"upstream 503, backing off"}
```

| Level | Use |
|:---|:---|
| `ERROR` | needs a human; always paired with an alert or a DLQ entry |
| `WARN` | self-healing (retry, proxy rotation, circuit open) |
| `INFO` | lifecycle: startup, config loaded, batch indexed, source run completed |
| `DEBUG` | per-message detail; off in prod, enable per-target at runtime |
| `TRACE` | developer-only |

Runtime level control: `RUST_LOG` + an admin endpoint `POST /admin/log-level {target, level}` with a
15-minute auto-revert so nobody leaves prod on `DEBUG`.

Retention: 15 days hot. Log volume budget ≤ 2 GB/day at 1M queries/day — if a component exceeds it,
that is a bug, not a capacity request.

---

## 4. Tracing

Spans are `tracing` spans; export is OTLP-compatible but **local-only** (no SaaS collector — see
[[Security and Privacy]]). Sampling: 100 % of errors, 1 % of successful searches, 100 % of ingestion
chains for one source per hour.

```
span: http.request {request_id, route, status}
 └ span: search {lang, expanded_terms_count, results_count}
    ├ span: detect_language {confidence}
    ├ span: expand_query {variants}
    ├ span: meili.multi_search {index_count, hits, took_ms}
    └ span: rerank {candidates, capped_by_domain}
 └ span: summarize {tokens_out, model, ttft_ms}   ← separate root, linked by request_id
```

Ingestion chains are correlated by `trace_id` on the [[Task Queue]] envelope, so
`fetch → parse → dedup → enrich → index` for one document is a single searchable trace.

---

## 5. Dashboards (Grafana, provisioned from git)

| Dashboard | Panels |
|:---|:---|
| **Search Health** | QPS, p50/p95/p99 latency by route, error rate, zero-result rate by language, summary TTFT and drop rate |
| **Index Health** | doc count by `source_type`, indexing rate, index size on disk, Meilisearch task queue, freshness (median `crawled_at` age) |
| **Ingestion** | queue depth + lag per stream, fetch success rate by platform, DLQ rate, dedup ratio, docs/min per worker |
| **Crawl Politeness** | requests/min per host, robots blocks, 429/403 rate by platform, proxy pool health |
| **Collection Health** | identity pool by tier, challenge/ban rate, canary status, median identity lifespan, signer version age, access-path mix, bandwidth and cost per 1 000 docs |
| **Models** | inference latency + memory for STT / CLIP / LLM / sentiment, queue wait for `xustive-ml` |
| **Host** | CPU, RAM, disk, IO, network, container restarts |

---

## 6. Alerts

| Alert | Condition | Severity | Runbook |
|:---|:---|:---|:---|
| `SearchDown` | `readyz` failing 2 min | **page** | restart api; check meili health |
| `SearchLatencyHigh` | p95 `/search` > 400 ms for 10 min | page | check meili CPU, index size |
| `ZeroResultsSpike` | zero-result rate > 25 % for 15 min | page | likely a bad index/settings deploy |
| `IndexStale` | median `crawled_at` age > 24 h | ticket | ingestion stalled |
| `QueueBacklog` | `queue_depth{stream="q:enrich"}` > 50k for 30 min | ticket | scale workers |
| `QueueLagHigh` | `queue_lag_seconds` > 3600 | ticket | stuck consumer |
| `DLQGrowth` | `rate(dlq_total[15m]) > 10/min` | ticket | poison payload class |
| `ProxyPoolDegraded` | `proxy_healthy` < 20 % of pool | ticket | [[Proxy Manager]] |
| `PlatformBlocking` | 403/429 rate > 40 % for one platform, 30 min | ticket | back off; check path ladder |
| `PoolExhausted` | all identities quarantined for a platform | **page** | halt platform; [[Session Manager]] §7 |
| `ChallengeSpike` | challenge rate > 30 % in 15 min | **page** | halt platform — likely a defence rollout |
| `CanaryDown` | canary returns empty for > 30 min | **page** | distinguishes cloaking from a code break |
| `SignerFailure` | `signer_failure_rate` > 0.30 for 5 min | **page** | [[Signature Service]] re-extraction |
| `EgressMismatch` | `proxy_egress_mismatch_total` > 0 | **page** | proxy leaking real IP |
| `WebRTCLeak` | `fp_webrtc_leak_total` > 0 | **page** | quarantine profile |
| `IdentityLifespanDrop` | median lifespan falls > 30 % week-over-week | ticket | **leading indicator that pacing is too aggressive** |
| `BandwidthBudget80` | residential spend > 80 % of monthly budget | ticket | check for a misrouted source |
| `SummaryDropHigh` | drop rate > 20 % for 15 min | ticket | scale `xustive-ml` |
| `DiskPressure` | volume > 85 % | page | expand or prune raw blobs |
| `TelemetryLeak` | log line matches query-shaped field | **page** | privacy incident, see [[Security and Privacy]] |
| `ToolDataAgeing` | `xustive_data_age_seconds` > 90 min for 10 min | ticket | [[Tool Data Plane]] — fetcher stalled, cards still shown |
| `ToolDataStale` | `xustive_data_age_seconds` > 3 h for 5 min | **page** | past the staleness limit; cards now withheld |
| `ToolDataMissing` | `absent(xustive_data_age_seconds)` for 15 min | **page** | cache flushed or fetcher never completed a pass |
| `ToolDataCoverageDropped` | `xustive_data_entries{dataset="weather"}` < 55 for 30 min | ticket | partial fetch failure the age gauge cannot see |

Every alert must link to a runbook section. An alert without a runbook gets deleted, not ignored.

### 6.1 Alert rules are unit-tested

A rule file that parses is not a rule file that fires. `deploy/prometheus/alerts_test.yml` replays
synthetic series through the real rules and asserts which alerts appear, so a threshold typo, a
`for:` that never elapses, or a label that fails to propagate is caught by `make check-alerts`
rather than during the incident the alert was written for.

The tool-data rules exercise five scenarios: healthy, a stalled fetcher, data past the staleness
limit, a flushed cache, and a partial fetch failure. The last two are the reason there is more than
one rule — a flushed cache publishes **no series at all**, so every threshold rule goes silent
exactly when it matters, and a partial failure keeps the age healthy while coverage shrinks.

---

## 7. SLOs

| SLO | Target | Error budget |
|:---|:---|:---|
| Search availability | 99.5 % monthly | 3 h 39 m |
| Search p95 latency ≤ 200 ms | 99 % of 5-min windows | |
| Summary delivered within 2.5 s | 95 % of searches that request it | |
| Index freshness < 6 h for tier-A sources | 95 % of days | |

Numbers derive from [[Performance Budgets]]; if the two disagree, Performance Budgets wins and this
note gets corrected.

---

## 8. Open Questions

- [ ] Do we adopt Loki for log search, or is `docker logs` + grep enough at this scale?
- [ ] Is 1 % trace sampling enough to debug rare Darija ranking complaints?
- [ ] Can we measure relevance in prod at all without logging queries? (proposal: aggregate
      zero-result rate by language only — no query text, k-anonymous counters)

## Related

[[Deployment Topology]] · [[Error Handling and Resilience]] · [[Security and Privacy]] ·
[[Performance Budgets]] · [[Task Queue]]
