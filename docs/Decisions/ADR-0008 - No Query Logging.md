---
tags:
  - adr
adr-id: "0008"
status: partly implemented
date: 2026-08-06
---

# ADR-0008 - No Query Logging

## Status

Accepted, **amended twice**, **partly implemented** (see Where it stands). [[ADR-0015 - Anonymous Interaction Signals for Ranking]] amended the
"No click tracking" and "Aggregate counters … default off" rows; [[ADR-0018 - Anonymous Search History]] amends the "Query text never written to durable storage" row (identifier-free, windowed
retention of the normalised term, for the operator console and ranking). The rows below stand except
where those ADRs supersede them. Constrains [[Security and Privacy]], [[Observability]],
[[Autocomplete Service]], [[API Gateway]], [[Error Handling and Resilience]].

## Context

Every search engine logs queries. It is how you measure relevance, build autocomplete, find zero-
result gaps, debug complaints, and detect abuse. Giving that up removes the primary feedback loop for
the thing the product is judged on.

The counter-argument is that "we don't store your searches" is Xustive's central claim, and a claim
of that kind is only worth anything if it is structurally true rather than policy-true. A policy can
change, be misconfigured, be compelled, or be quietly violated by a well-meaning debug log. A system
that never has the data cannot do any of those things.

The decision is not "privacy vs. quality" in the abstract. It is: do we accept a harder engineering
problem in exchange for a claim we can actually defend?

## Decision

**Zero query retention, enforced by architecture and tested in CI.**

| Rule | Enforcement |
|:---|:---|
| Query text never written to durable storage | code review + CI telemetry lint — **amended by [[ADR-0018 - Anonymous Search History]]**: the normalised term (no identifier) may be retained in a windowed counter for the operator console and ranking |
| Query text never in a log line, metric label, or span attribute | lint greps `tracing::` call sites (CI); `test-egress.sh` scans live containers' logs for query-bearing URLs on every run; `scan-logs.sh` for log files; transport errors scrub request URLs at the type boundary |
| Query text never leaves the `core` network | `xustive-api`/`xustive-ml` have **no egress route** ([[Deployment Topology]] §3); an egress test passes only if the connection fails |
| No result caching keyed by query | a query-keyed cache is a query log with extra steps |
| No click tracking, no redirect interstitials, no `ping` | the `href` is the destination |
| Client IPs not stored | rate limiter keys on `HMAC(ip/24, daily-rotating salt)`, memory-only, 60 s TTL |
| Audio and images never written to disk | in-memory buffers, zeroised; disk-scan test |
| Aggregate counters (if ever enabled) must be k-anonymous, k ≥ 20 | [[Autocomplete Service]] §4, default off |

What we *may* record: query **length bucket**, detected **language**, **result count**, and
**latency** — none of which identify a query or a person, and all of which are enough to detect that
something is broken ([[Observability]] §2).

## Consequences

**Good**
- The product's headline claim is true by construction, not by promise.
- There is no query dataset to leak, subpoena, sell, or accidentally ship to a third party.
- Compliance under [[Legal and Compliance]] §5 is dramatically simplified: the strongest position on
  personal data is not holding any.
- It forces relevance work to be done properly — with judged golden sets that are stable and
  reproducible, rather than by chasing production logs.

**Bad**
- **No production relevance feedback.** We cannot see what real users search or where we fail them,
  except in aggregate by language.
- Autocomplete cannot learn from popular queries; it is built from the corpus instead
  ([[Autocomplete Service]] §4).
- Debugging a user complaint means asking them to reproduce it — we have no record.
- Abuse detection is limited to rate and shape, not content.
- A/B testing ranking changes on live traffic is essentially impossible; evaluation must be offline
  ([[Ranking and Relevance]] §6).

**Commits us to**
- Investing in offline evaluation as the *only* quality signal — the golden sets in
  [[Testing Strategy]] §7 are not a nice-to-have, they are the entire feedback loop.
- Growing those sets from user-reported complaints, which becomes the substitute for query logs.

## Alternatives

| Option | Why not |
|:---|:---|
| Log queries with short retention (e.g. 24 h) | still a query log; still leakable and compellable; the claim becomes a caveat |
| Log with a hashed/tokenised query | reversible by dictionary attack for short queries; false comfort |
| Differential-privacy aggregate logging | genuinely interesting; complexity and a correct implementation are beyond v1 scope — reconsider later |
| Opt-in logging | a consent banner on a privacy-first search engine undermines the proposition, and opt-in populations are unrepresentative anyway |

## Revisit when

- Offline evaluation demonstrably fails to catch real relevance problems users report — at which
  point the honest options are differential privacy or explicit, granular opt-in, **not** quiet
  logging.
- A rigorous, reviewable DP aggregation design exists and someone owns it.

## Related

[[Security and Privacy]] · [[Observability]] · [[Autocomplete Service]] · [[Ranking and Relevance]] ·
[[Testing Strategy]] · [[Legal and Compliance]] · [[Decision Log]]

## Where it stands (2026-08-27)

Row by row, against the code:

- **Telemetry lint** — `scripts/lint-telemetry.sh` exists and runs in CI (`.github/workflows/ci.yml`). Holds.
- **No egress** — `scripts/test-egress.sh` proves SearXNG is unreachable from the `core` network, but never asserts the ADR-0017 form "the API reaches the gateway and nothing else". `deploy/docker-compose.yml` has no `api` service (the API runs on the host), so the network-level no-egress claim for `xustive-api` is not enforced by compose at all.
- **Live-log scan** — in CI compose is brought up with `up -d --no-start` (`ci.yml`), so the live-container log scan in `test-egress.sh` runs against nothing and is vacuous. `scripts/scan-logs.sh` is nightly-only and its pattern list omits `token`, `password`, `cookie` and `secret`.
- **No query-keyed cache** — holds: the tool cache is keyed by wilaya, the knowledge cache by entity id (`crates/xustive-api/src/knowledge.rs`); there is no result cache keyed by query.
- **Client IPs** — `crates/xustive-api/src/ratelimit.rs` keys on keyed BLAKE3 over `/24` (v4) and `/48` (v6) with a **24 h** rotating salt, memory-only, never reading `X-Forwarded-For`. Divergences: the salt rotates daily as written but the `SOURCES` bucket window is **3600 s**, not 60 s.
- **Media never on disk** — diverged for the OCR sidecar: `services/ocr-sidecar/app.py` writes the image to a `tempfile` and deletes it in `finally` (as [[ADR-0016 - Two OCR Engines with an Optional Unlimited-OCR Sidecar]] records). No zeroisation of in-memory media buffers and no disk-scan test exists.
- **Click tracking / counters** — amended by ADR-0015 and ADR-0018; see their Where it stands.
