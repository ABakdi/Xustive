---
tags:
  - component
  - tooling
component-id: C36
binary: xustive-loadgen
status: built
updated: 2026-08-27
---

# Load Generator

> **ID** C36 · **Binary** `xustive-loadgen` · **Upstream** none · **Downstream** [[API Gateway]]
> (whatever `--target` points at)

## 1. Purpose

Measure the serving plane under a realistic query stream and say pass or fail against the
[[Performance Budgets]]. Built in M4-T03 ([[Milestone 4 - Quality and Operations]]) and
Rust-native rather than a `k6`/`oha` dependency: one `cargo run`, no toolchain to install, and the
query mix and statistics are unit-tested in the same language as the thing under test.

## 2. Open loop, on purpose

Requests are dispatched on a fixed schedule for the target rate, **independently** of whether
prior requests have returned. A closed loop (N workers each firing the next request after the
last completes) measures throughput but hides latency under load — the coordinated-omission
error, where a stalled server simply receives fewer requests and reports a rosy p95. Here the
schedule does not slow down when the server does, so a stall shows up as rising latency and, past
the in-flight cap, as **shed** requests, which is what a real overload looks like.

## 3. Where it lives today

| Piece | Path |
|:---|:---|
| CLI, scenarios, scheduler | `crates/xustive-loadgen/src/main.rs` |
| Weighted query mix (deterministic LCG) | `crates/xustive-loadgen/src/mix.rs` |
| Percentiles and verdict | `crates/xustive-loadgen/src/stats.rs` |
| Make target | `make load S=<scenario> [RPS=…] [DUR=…]` |

## 4. Interface

```
xustive-loadgen [--target URL | XUSTIVE_API_URL] [--scenario search|suggest|summary|mixed]
                [--rps N] [--duration 30] [--max-inflight N] [--p95-ms N]
                [--max-error-rate 0.01] [--report path] [--seed 42]
```

| Scenario | Hits | Default rps | Default p95 budget |
|:---|:---|---:|---:|
| `search` | `GET /api/v1/search` with whole queries | 100 | 200 ms |
| `suggest` | `GET /api/v1/suggest` with prefixes (per-keystroke traffic) | 300 | 40 ms |
| `summary` | a search, then `POST /api/v1/summary` for its token | 2 | 2 500 ms |
| `mixed` | ~70 % suggest, ~30 % search | 150 | 200 ms |

`--max-inflight` defaults to 2 × rps. The exit code is non-zero on a budget miss:
`p95 ≤ budget && (errors + shed) / requests ≤ max_error_rate`. `--report` writes the JSON
`Report` (requests, ok, errors, shed, error_rate, throughput_rps, p50/p95/p99/max ms).

## 5. Internal Design

**The mix.** A load test is only as honest as its inputs: one query a million times measures a
cache, random strings measure the empty-result path. `DEFAULT_QUERIES` is a weighted, mostly
Algerian mix across ar/ary/fr/en — wilaya names, public services (Sonelgaz, CNAS), health and
how-to queries, French and Darija phrasings of the same intents, and a few long-tail misses a real
stream always contains. Selection is a small LCG seeded by `--seed`, so a run is reproducible and
two runs are comparable. `pick_prefix` cuts a query to `n` characters for the suggest scenario.

**Statistics.** Nearest-rank percentiles on a sorted copy of the raw microsecond samples. A run
collects at most a few hundred thousand samples, so an exact sort is simplest and correct — no
approximate histogram, no dependency. ok / error / shed are counted separately because they mean
different things.

**The summary scenario** times search + summary together; a two-step timing is a noted
refinement.

## 6. What it does not do

It does not assert the budgets at 10 M documents — that needs the corpus and the target
hardware ([[Performance Budgets|Quadro T1000 4 GB]] is not a load-test host). It measures
whatever stack it is pointed at, so the same harness serves a laptop smoke test and a staging
run unchanged. It sends no interaction beacons and does not exercise the web tier.

## 7. Security and privacy

The mix is synthetic; nothing a real person typed is in it. Pointed at production it is a
self-inflicted load and is rate-limited like any other client — run it against staging.

## 8. Testing

Unit tests cover the LCG picker (same seed, same sequence), prefix cutting, and the percentile
and verdict arithmetic.

## 9. Open Questions

- [ ] Add `knowledge` and `tools` scenarios now that those endpoints exist (M8).
- [ ] Per-step timing for the summary scenario.

## Related

[[Performance Budgets]] · [[Milestone 4 - Quality and Operations]] · [[API Gateway]] ·
[[Observability]]
