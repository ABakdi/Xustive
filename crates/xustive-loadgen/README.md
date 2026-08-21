# xustive-loadgen

An **open-loop** HTTP load generator for the Xustive serving plane — the load-testing harness for
[[Milestone 4 - Quality and Operations]] (M4-T03). Rust-native, so there is no `k6`/`oha` toolchain
to install and the query mix and statistics are unit-tested in the same language as the engine.

## Why open-loop

Requests are dispatched on a **fixed schedule** for the target rate, regardless of whether earlier
requests have returned. A closed loop (N workers each firing the next request only after the last
finishes) hides latency under load — the *coordinated-omission* error, where a stalled server simply
receives fewer requests and reports a flattering p95. Here the schedule does not slow down when the
server does: a stall shows up as rising latency and, past the in-flight cap, as **shed** requests —
what a real overload looks like.

## Run it

The API must be running (`make run-api`, port 8080). Then:

```bash
cargo run -p xustive-loadgen -- --scenario search --rps 100 --duration 30
# or via the Makefile:
make load S=search RPS=100 DUR=30
```

Scenarios and their [[Performance Budgets]] p95 targets:

| `--scenario` | Hits | Default rps | p95 budget |
|:---|:---|:---|:---|
| `search`  | `GET /api/v1/search` (whole queries)      | 100 | 200 ms |
| `suggest` | `GET /api/v1/suggest` (query prefixes)    | 300 | 40 ms |
| `summary` | search → `POST /api/v1/summary`           | 2   | 2 500 ms |
| `mixed`   | ~70 % suggest, ~30 % search               | 150 | 200 ms |

It prints a report and **exits non-zero if the p95 budget or error rate is missed**, so it drops into
CI or a Makefile gate. `--report out.json` also writes the numbers as JSON.

## Reading the output

```
ok / err / shed   18 / 0 / 2
```

- **ok** — 2xx responses, timed and fed into the percentiles.
- **err** — 5xx or transport failures. This is the count that means the server *broke*.
- **shed** — 429/503: the server correctly rejected excess load (rate limiter, load-shed layer).
  Not a failure of the server, but it counts against the error rate for the verdict.

## Rate limits and single-IP load

The API rate-limits per client IP (60/min search, 300/min suggest). A single-machine load test trips
that immediately and reports mostly **shed**. To measure real throughput either raise the limits for
the load-test environment, or drive load from multiple source IPs. For a quick latency check, run
*under* the limit (e.g. `--scenario search --rps 1`).

## The query mix

Not one query repeated (that measures a cache) nor random strings (that measures the empty-result
path): a weighted, mostly Algerian mix across `ar`/`ary`/`fr`/`en` — wilaya names, public services
(Sonelgaz, CNAS), health and how-to queries, and a few long-tail misses. Selection is deterministic
given `--seed`, so two runs are comparable. See `src/mix.rs`.

## What it does not do

It does not, by itself, prove the budgets **at 10M documents** — that needs the real corpus and the
target hardware. It measures whatever stack it is pointed at, so the same harness serves a laptop
smoke test and a staging load test unchanged. The 10M-scale run is the M4 exit-gate exercise; this is
the instrument for it.
