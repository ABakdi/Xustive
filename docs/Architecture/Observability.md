---
tags:
  - architecture
  - ops
type: architecture
status: implemented
updated: 2026-08-27
---

# Observability

> Metrics, logs, traces, dashboards, and alerts. Constrained hard by [[Security and Privacy]]:
> **no query text, no user identifiers, ever** — not in a log line, not in a metric label, not in a
> span attribute.
>
> Audited against the code on 2026-08-27. Metric and alert names below are the ones that exist;
> the 2026-08-06 design lists are kept where they describe things not yet built, marked as such.

---

## 1. The Privacy Constraint First

| Forbidden | Allowed instead |
|:---|:---|
| the raw query string | `query_len_bucket` (`telemetry::query_len_bucket`), `lang` |
| client IP | truncated salted hash, in-memory only, for rate limiting; a wilaya, never a coordinate, for weather ([[ADR-0020 - Approximate Location from a Local Database]]) |
| `user_id`, cookie, session | nothing — there are none |
| result URLs clicked | k-anonymous per-document counters only ([[Interaction Signals]]) |
| collection credentials, cookies, TOTP seeds | `identity_id` only ([[Session Manager]] §4.8 — design; not built) |

Two checks, at either end of the pipe:

- `scripts/lint-telemetry.sh` (CI, `make lint`) fails the build if a `tracing::` call site names
  a forbidden field: `q`, `query`, `raw_query`, `normalized_query`, `transcript`, `ocr_text`,
  `user_query`, `search_term`, `password`, `passwd`, `cookie(s)`, `credentials`, `totp`, `secret`,
  `api_key`, `token`. Deliberately dumb and slightly over-eager: a false positive costs one rename.
- `scripts/scan-logs.sh` (`make scan-logs`, nightly against the day's logs) checks the other end —
  whether anything query-shaped actually reached a log line. The lint cannot see a query that
  arrives inside a struct someone made `Debug`; the scan cannot see a leak on a path that never
  ran.

Note the two distinct reasons: query fields are forbidden because of the promise to users
([[ADR-0008 - No Query Logging]]); credential fields because identity secrets are the highest-value
target in the system. Both land in the same lint. See [[Testing Strategy]].

Metric labels are `&'static str` keys by construction (`xustive_api::metrics`), which makes "log
the query as a label" a compile error rather than a code-review catch. The registry is a small
hand-rolled one (counters, gauges, histograms rendered in the text exposition format), not the
`prometheus` crate.

---

## 2. Metrics (Prometheus)

Naming: `xustive_<subsystem>_<metric>_<unit>`. Labels are **bounded** — never a URL, never a query.
Scraped from `GET /metrics` on the API (`deploy/prometheus/prometheus.yml`: jobs `xustive-api` and
`meilisearch`).

### Serving (exist)

| Metric | Type | Labels | Purpose |
|:---|:---|:---|:---|
| `xustive_http_requests_total` | counter | `route`, `status` | traffic + error rate |
| `xustive_http_duration_seconds` | histogram | `route` | latency SLO |
| `xustive_search_duration_seconds` | histogram | `stage` (`detect`, `retrieve`, `rerank`) | where time goes |
| `xustive_search_results_total` | histogram | `lang` | result-count distribution |
| `xustive_search_zero_results_total` | counter | `lang` | relevance health |
| `xustive_lang_detected_total` | counter | `lang` | query-language mix |
| `xustive_query_expansion_total` | counter | `lang` | how often the expansion leg ran |
| `xustive_semantic_fused_total` | counter | `kind` | dense-recall fusion (M7-T02) |
| `xustive_degraded_total` | counter | `stage` | deadline ladder skips ([[Error Handling and Resilience]] §6) |
| `xustive_instant_answers_total` | counter | `tool` | which [[Instant Answers]] fire |
| `xustive_suggest_total` | counter | `empty` | autocomplete hit rate |
| `xustive_rate_limited_total` | counter | `route` | abuse |
| `xustive_summary_duration_seconds` | histogram | — | [[Summarizer]] |
| `xustive_summary_withheld_total` | counter | `reason` | validator rejections and timeouts |
| `xustive_summary_external_total` | counter | `outcome` | external summariser attempts |
| `xustive_translate_total` | counter | `outcome` | translation tool |
| `xustive_federation_duration_seconds` | histogram | — | [[Federation Gateway]] round trip |
| `xustive_federation_searches_total` | counter | `outcome` | federated strip hits / empty |
| `xustive_federation_urls_fed_total` | counter | — | URLs handed to the crawler from federation |
| `xustive_federation_blend_cards_total` | counter | `source` (`local`, `web`) | blend composition |
| `xustive_data_age_seconds` | gauge | `dataset` (`weather`, `knowledge`) | [[Tool Data Plane]] freshness |
| `xustive_data_entries` | gauge | `dataset` | coverage (58 wilayas for weather) |
| `xustive_build_info` | gauge | version labels | which build is serving |

### Ingestion (exist)

| Metric | Type | Labels | Purpose |
|:---|:---|:---|:---|
| `xustive_queue_depth` | gauge | — | consumer-group lag on `q:index` — **the** backpressure signal |
| `xustive_queue_pending` | gauge | — | delivered, unacknowledged |
| `xustive_queue_dead_letters` | gauge | — | `q:index:dead` length |
| `xustive_crawl_fetched_total` | counter | — | crawl throughput |
| `xustive_crawl_revisited_total` | counter | — | adaptive re-crawl ([[ADR-0011 - Adaptive Recrawl over Static Crawling]]) |

Not yet emitted (design, 2026-08-06): `xustive_fetch_total{source_type,outcome}`,
`xustive_fetch_duration_seconds`, `xustive_queue_lag_seconds`, `xustive_stage_duration_seconds`,
`xustive_docs_indexed_total`, `xustive_dedup_rejected_total{kind}`, `xustive_proxy_healthy`,
`xustive_proxy_banned_total`, `xustive_robots_blocked_total`, and the STT / image-pipeline
histograms. Crawl health is read today from the [[Crawler Console]] (`/api/v1/admin/crawler/*`),
which reports per-channel and per-host counters from Redis rather than through Prometheus.

### Collection (design only)

Added by [[ADR-0009 - Direct Collection for Social Platforms]]; none of it is built. These are
**tier-1 operational signals** — detection damage compounds silently, so they page rather than
ticket.

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

Meilisearch's own `/metrics` is scraped. `xustive_model_load_seconds`,
`xustive_model_memory_bytes` and `node_exporter` are design; not wired.

---

## 3. Logs

`tracing` + `tracing-subscriber`. `telemetry.log_json = true` (prod, staging) gives one flattened
JSON line per event, never multi-line; dev uses the compact human format. The filter comes from
`telemetry.log_filter` (e.g. `info,xustive_api=debug,xustive_search=debug`), with llama.cpp's
chatter quietened unless asked for.

```json
{"ts":"2026-08-06T10:12:03.481Z","level":"WARN","target":"xustive_ingest::fetch",
 "request_id":null,"source_id":"elkhabar-dz","host":"elkhabar.com",
 "event":"fetch_retry","attempt":2,"status":503,"msg":"upstream 503, backing off"}
```

| Level | Use |
|:---|:---|
| `ERROR` | needs a human; always paired with an alert or a DLQ entry |
| `WARN` | self-healing (retry, circuit open, job dead-lettered) |
| `INFO` | lifecycle: startup, config loaded, batch indexed, source run completed |
| `DEBUG` | per-message detail; off in prod, enable per-target at runtime |
| `TRACE` | developer-only |

Runtime level control: `POST /api/v1/admin/log-level {"filter": "<EnvFilter>"}` (with
`X-Admin-Key` when `api.admin_key` is set) raises or lowers the filter for at most
`OVERRIDE_TTL` = 15 minutes, after which a background ticker reverts to the configured baseline so
nobody leaves prod on `DEBUG`. Sending `{"filter": null}` reverts immediately. The
[[Crawler Console]] shows the active filter and the time left.

Every request carries an `x-request-id` (ULID, set by the API; a client-supplied one is stripped
so nobody can inject an identifier that outlives the request).

Retention: 15 days hot. Log volume budget ≤ 2 GB/day at 1M queries/day — if a component exceeds it,
that is a bug, not a capacity request. (Targets; no retention is configured in `deploy/`.)

---

## 4. Tracing

Spans are `tracing` spans. The design (2026-08-06) called for OTLP export, **local-only** (no SaaS
collector — see [[Security and Privacy]]) with 100 % of errors, 1 % of successful searches and one
ingestion chain per source per hour sampled. No exporter is wired as of 2026-08-27 — there is no
OpenTelemetry dependency — so spans exist only as structured log context.

```
span: http.request {request_id, route, status}
 └ span: search {lang, expanded_terms_count, results_count}
    ├ span: detect_language {confidence}
    ├ span: expand_query {variants}
    ├ span: meili.search {hits, took_ms}
    └ span: rerank {candidates, capped_by_domain}
 └ span: summarize {tokens_out, model, ttft_ms}   ← separate request, linked by request_id
```

The design's per-document ingestion `trace_id` on the [[Task Queue]] envelope is not implemented;
the crawl daemon's per-URL outcome is recorded in the admin event log instead
(`/api/v1/admin/crawler/events`).

---

## 5. Dashboards (Grafana)

Grafana is in `deploy/docker-compose.yml` (on the `obs` network, `internal: true`, with Prometheus
as its datasource), but `deploy/grafana/provisioning/` is empty: **no dashboards are provisioned
from git yet** (2026-08-27). The intended set:

| Dashboard | Panels |
|:---|:---|
| **Search Health** | QPS, p50/p95/p99 latency by route, error rate, zero-result rate by language, summary duration and withheld rate |
| **Index Health** | doc count by `source_type`, indexing rate, index size on disk, Meilisearch task queue, freshness (median `crawled_at` age) |
| **Ingestion** | queue depth + pending, DLQ length, fetched / revisited rate |
| **Crawl Politeness** | requests/min per host, robots blocks, 429/403 rate by host |
| **Answer Data** | `data_age_seconds` and `data_entries` per dataset |
| **Models** | inference latency + memory for STT / CLIP / LLM, queue wait for `xustive-ml` |
| **Host** | CPU, RAM, disk, IO, network, container restarts |

Until then the [[Crawler Console]] (`/admin/*`) is the operator's live view.

---

## 6. Alerts

`deploy/prometheus/alerts.yml`. Severity is a label (`warning` = ticket, `critical` = page).

| Alert | Condition | Severity | Runbook |
|:---|:---|:---|:---|
| `SearchDown` | `up{job="xustive-api"} == 0` for 2 min | **critical** | restart api; check meili health |
| `SearchLatencyHigh` | p95 `/search` > 400 ms for 10 min | **critical** | check meili CPU, index size |
| `ZeroResultsSpike` | zero-result share > 25 % over 15 min | **critical** | likely a bad index/settings deploy |
| `SummaryDropHigh` | `summary_withheld_total` > 20 % of generations over 15 min | warning | scale `xustive-ml` / check validator |
| `QueueBacklog` | `xustive_queue_depth` > 50k for 30 min | warning | scale indexer workers |
| `DLQGrowth` | `increase(xustive_queue_dead_letters[15m]) > 10` | warning | poison payload class |
| `ToolDataAgeing` | `data_age_seconds{dataset="weather"}` > 90 min for 10 min | warning | [[Tool Data Plane]] — fetcher stalled, cards still shown |
| `ToolDataStale` | same > 3 h for 5 min | **critical** | past the staleness limit; cards now withheld |
| `ToolDataMissing` | `absent(data_age_seconds{dataset="weather"})` for 15 min | **critical** | cache flushed or fetcher never completed a pass |
| `ToolDataCoverageDropped` | `data_entries{dataset="weather"}` < 55 for 30 min | warning | partial fetch failure the age gauge cannot see |
| `KnowledgeAgeing` | `data_age_seconds{dataset="knowledge"}` > 10 days for 1 h | warning | harvester behind its 7-day cadence ([[ADR-0019 - The Knowledge Layer]]) |
| `KnowledgeStale` | same > 21 days for 1 h | **critical** | panels render unchecked facts; nothing is withheld |
| `KnowledgeMissing` | `absent(...{dataset="knowledge"})` for 30 min | warning | index dropped or harvester never ran |

Design-only alerts (not in `alerts.yml`, their metrics do not exist yet): `IndexStale`,
`QueueLagHigh`, `ProxyPoolDegraded`, `PlatformBlocking`, `PoolExhausted`, `ChallengeSpike`,
`CanaryDown`, `SignerFailure`, `EgressMismatch`, `WebRTCLeak`, `IdentityLifespanDrop`,
`BandwidthBudget80`, `DiskPressure`, `TelemetryLeak` (covered procedurally by
`scripts/scan-logs.sh`).

Every alert must link to a runbook section ([[Runbooks]]). An alert without a runbook gets
deleted, not ignored.

### 6.1 Alert rules are unit-tested

A rule file that parses is not a rule file that fires. `deploy/prometheus/alerts_test.yml` replays
synthetic series through the real rules and asserts which alerts appear, so a threshold typo, a
`for:` that never elapses, or a label that fails to propagate is caught by
`scripts/check-alerts.sh` (`promtool test rules`, run from the Prometheus image if `promtool` is
not installed) rather than during the incident the alert was written for. The API's `dataage`
tests additionally assert that the weather thresholds in `alerts.yml` match the staleness limits
the serving plane enforces, so the two cannot drift apart.

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
note gets corrected. The hard request ceiling is `api.timeout_search_ms` (1 500 ms in prod).

---

## 8. Open Questions

- [ ] Do we adopt Loki for log search, or is `docker logs` + grep enough at this scale?
- [ ] Wire an OTLP exporter (local collector only) or keep spans as log context?
- [x] Can we measure relevance in prod at all without logging queries? — Yes, as built: zero-result
      rate by language (metrics), k-anonymous interaction counters ([[Interaction Signals]]), and
      the offline golden set ([[Ranking and Relevance]] §6).

## Related

[[Deployment Topology]] · [[Error Handling and Resilience]] · [[Security and Privacy]] ·
[[Performance Budgets]] · [[Task Queue]] · [[Runbooks]] · [[Crawler Console]]
