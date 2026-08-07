---
tags:
  - component
  - ingestion
component-id: C29
binary: xustive-toold
status: specified
updated: 2026-08-07
---

# Tool Data Plane

> **ID** C29 · **Binary** `xustive-toold` · **Upstream** external publishers ·
> **Downstream** [[Instant Answers]]

## 1. Why this exists as its own process

The serving plane **has no route to the internet**. That is enforced, not aspirational —
`scripts/test-egress.sh` fails the build if `xustive-api` can reach outside, and the `core`
network is `internal: true` ([[Deployment Topology]]).

Weather and exchange rates come from outside. Two ways to reconcile that:

1. Give the serving plane egress. **No.** That constraint is what guarantees a prompt-injected
   summariser or a compromised dependency cannot exfiltrate queries. It is worth more than any
   tool.
2. A separate fetcher on a network that *does* have egress, writing to a cache the serving plane
   reads.

This is (2). Same shape as the crawler, same reason.

The consequence is worth stating plainly: **the serving plane can only ever answer from cache.**
A tool needing data nobody has fetched yet has no answer, and that is correct rather than
unfortunate.

## 2. Responsibilities

**In scope**: scheduled fetching of external tool data; validation; writing to the shared cache;
recording provenance and measurement time.

**Out of scope**: matching, formatting, rendering (→ [[Instant Answers]]); anything per-query. It
never sees a query. It cannot — it does not share a network with anything that has one.

## 3. Interface

Writes to Redis, which both planes reach. One key per dataset:

```jsonc
tool:weather:v1:31           // wilaya code
{
  "fetched_at": 1786000000,
  "observed_at": 1785998400,   // when the *publisher* measured it, not when we asked
  "source": "open-meteo",
  "licence": "CC-BY-4.0",
  "payload": { … }
}
```

`observed_at` and `fetched_at` are separate on purpose. A rate we fetched a minute ago that the
publisher measured yesterday is a day old, and showing the fetch time would be a lie of exactly
the kind §2 of [[Instant Answers]] forbids.

## 4. Datasets

| Dataset | Cadence | Source | Staleness limit |
|:---|:---|:---|:---|
| Weather, 58 wilayas | 30 min | Open-Meteo (CC-BY, no key, no per-user calls) | 3 h |
| Currency, official | 6 h | Bank of Algeria | 48 h |
| Currency, parallel | 1 h | aggregated reporting — see §6 | 48 h |
| Fuel prices | daily | Naftal / official notices | 30 d |
| Sports fixtures & results | 15 min in season | football federation feeds | 6 h |
| Exam result portals | daily in season | official portals — **links only** | 7 d |

Fixed cadence, never per-request. 58 wilayas every 30 minutes is 116 requests an hour: a trivial
load for the publisher, and — the point — **a request pattern that reveals nothing**, because it
is identical whether one person or a million searched for weather.

## 5. Validation

Fetched data is checked before it is written, because a bad write silently poisons every
subsequent answer:

- Schema and type conformance.
- **Plausibility bounds**: a temperature of 300 °C or a dinar rate of 4 to the euro is rejected
  and the previous value kept. Publishers do occasionally emit garbage, and a tool card is a
  place where obviously-wrong output is maximally damaging.
- **Movement guard**: a rate moving more than 25 % in one interval is held and flagged rather
  than published. Real moves that large exist; so do decimal-point errors, and the cost of a
  slightly late correct rate is far below a wrong one.
- Failures leave the previous value and increment a metric. Never a partial write.

## 6. Provenance

Every dataset carries its source and licence into the rendered card. This is not legal
box-ticking — it is what lets a reader judge a number we cannot independently verify.

The parallel exchange rate has **no authoritative publisher**. Whatever aggregation is used, the
card must describe what it is in plain language rather than presenting it with the same authority
as the Bank of Algeria figure. If no honest sourcing can be found, **the parallel rate ships
disabled** and the tool shows only the official rate with a note. Shipping a made-up number
because the feature looks better with two is not an option.

## 7. Failure

| Failure | Response |
|:---|:---|
| Publisher down | Keep last good value; it ages and is eventually withheld by [[Instant Answers]] |
| Publisher returns nonsense | Rejected by §5; previous value kept; metric |
| Redis down | Serving plane finds no cache and renders no card. Search is unaffected |
| `xustive-toold` down | Data ages out and cards disappear over hours. **Never fatal** |

Nothing here can degrade search. That is the test for whether this component is correctly
separated.

## 8. Security

- Runs on the `ingest` network. **No route to `core`** beyond the Redis it writes.
- Receives no user input, ever. Its inputs are a schedule and a fixed list of URLs.
- Outbound requests go through the same `SafeUrl` guard as the crawler — a publisher that starts
  redirecting to a private address gets refused.
- Holds no credentials for anything the serving plane can reach.

## 9. Observability

`xustive_toold_fetch_total{dataset,outcome}` · `xustive_toold_data_age_seconds{dataset}` ·
`xustive_toold_rejected_total{dataset,reason}`.

**`data_age_seconds` is the one to alert on.** A fetcher that stops silently is invisible until a
user sees a stale rate, and by then trust is already spent.

## 10. Open questions

- [ ] Parallel-rate sourcing — the open question that gates the currency tool (§6).
- [ ] Do sports feeds need a licence review before use?
- [ ] Should this share the crawler's binary rather than being its own? Same shape, very
      different cadence and failure tolerance. Leaning separate.

## Related

[[Instant Answers]] · [[Deployment Topology]] · [[Security and Privacy]] · [[Web Fetcher]] ·
[[Observability]] · [[Legal and Compliance]]
