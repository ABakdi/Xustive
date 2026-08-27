---
tags:
  - adr
adr-id: "0018"
status: partly implemented; amended by ADR-0029 on 2026-08-27 — first-party collection, including identifiable data, is decided and will get its own ADR; the k-anonymous counts stay until then
date: 2026-08-25
---
# ADR-0018 - Anonymous Search History

## Status

Accepted; **implemented with one deployment gap** (signals land in the durable queue Redis outside dev). **Amends [[ADR-0008 - No Query Logging]]** — specifically its "Query text never written
to durable storage" row — and **extends [[ADR-0015 - Anonymous Interaction Signals for Ranking]]**,
which had already amended 0008's "No click tracking" and "Aggregate counters … default off" rows.
Constrains [[Security and Privacy]], [[Observability]], [[Interaction Signals]], and the
`/admin/interaction` console. Built on the k-anonymity escape hatch [[ADR-0008 - No Query Logging]]
itself named and [[Interaction Signals|weak_coverage]] first walked through.

## Context

[[ADR-0015 - Anonymous Interaction Signals for Ranking]] introduced windowed, identifier-free Redis
counters keyed by the **normalised query term** — per-query counts, per-`(query, doc)` clicks — to
feed ranking and re-crawl, and it named the operator dashboard as a consumer. M7-T10 turns that
latent capability into an operator-facing feature: a **browsable Search history** at
`/admin/interaction` — every term searched, its result count, and the clicks it drew.

That crosses a line 0015 left standing. ADR-0015 amended the *click-tracking* rows of ADR-0008 but
not this one:

> Query text never written to durable storage — code review + CI telemetry lint.

A per-term counter that survives its window *is* durable retention of query text. It is
identifier-free — there is no IP, user-agent, session, cookie, or account anywhere in the key or the
value, exactly as 0015 requires — but "we never store your searches" is no longer literally true,
and a privacy claim of that kind is only worth anything if it is precise. This ADR states plainly
what is now stored, why it does not reintroduce the harm ADR-0008 existed to prevent, and where the
one genuine residual risk (query *content* self-identification) is contained.

The user's framing, which this ADR adopts: **anonymity is a property of what is *stored*, not of how
many people share the box.** Storing no identifier is anonymity by construction; a k-threshold is a
separate, multi-user *surfacing* control against content self-identification. Conflating the two —
"you need many users for this to be private" — is the AOL-2006 mistake in reverse.

## Decision

**Persist an identifier-free search history — normalised term, result count, click counts — and
decouple *storage* (always identifier-free) from *surfacing* (`k` is a multi-user-only floor).**

| Rule | Enforcement |
|---|---|
| **No identifier is ever in the key or the value** — no IP, UA, session, cookie, account, device fingerprint, or per-event timestamp that could order events into a session | The history writes read none and store none; the same structural assertion M6/0015 apply to interaction keys covers these keys. This is *the* anonymity guarantee — it does not depend on `k`. |
| **What is stored**: per normalised term — a search count, a coarse category, the **result count**; per `(term-hash, doc)` — a click count | All under the `interaction:` Redis namespace, all windowed with a sliding TTL, so a term not repeated within the window decays to nothing |
| **Storage is always identifier-free; surfacing has a `k`-floor** | The `surfaceable(count, k)` predicate from [[Interaction Signals|weak_coverage]] gates the dashboard *and* the ranker on multi-user deployments. `k` never changes what is stored — only what is shown. |
| **Single-operator (`k = 1`) sees full history; this is "no anonymity, single operator", not "anonymised"** | The dev/single-box config lowers `k` to 1, as it already does for weak-coverage and interaction. On your own machine your own history in your own Redis is yours to read. |
| **Multi-user deployments threshold and blunt** — `k ≥ 20`, sliding window, **no session grouping, no fine-grained per-event timestamps** | Config validation keeps `k ≥ 20` unless `environment = dev` (as M6 enforces); no field exists to chain events or timestamp them precisely — chaining and precise times are what re-identify anonymous logs (AOL 2006) |
| **The query text still never reaches a log, metric label, or span** | The [[Observability|telemetry lint]] is unchanged and still runs; this history is a Redis ranking/console input, never observability, and `token` remains a forbidden field name |
| **Default off** | The `[interaction] enabled` flag, default `false`; off means no store connects and nothing is written |

The click path is unchanged from 0015: a click carries `{token, doc_id}`, the server resolves
`token → term hash` from an in-process, single-search, TTL'd token, and the query text never rides in
the click request. The history's per-`(term, doc)` click detail is built from that hash, so no
durable structure ever pairs a click with plaintext query at write time beyond the term counter the
search itself wrote.

## Consequences

**Good**
- The operator can finally see what the engine is being asked and where it comes up short — the
  feedback loop ADR-0008 gave up — **without any query dataset that can be tied to a person**, because
  there is no identifier to tie it by.
- It sharpens rather than abandons the privacy claim: the honest line is "we store no identifiers and
  never link a search to you", not "we store nothing", and the privacy page says exactly that.
- Weak-coverage, re-crawl prioritisation, and the history view are now one store read three ways.

**Bad**
- It amends a headline promise a second time. "We don't store your searches" becomes "we store search
  *terms and counts* with no identifier, and never link them to you." The privacy page must carry the
  precise version, per deployment mode, or the amendment is dishonest by omission.
- The residual risk is **content self-identification**: a term can be self-identifying regardless of
  identifiers (someone's full name, a rare medical query). On a single-operator box this is the
  operator's own history and moot; on a shared deployment the `k`-floor + window + no-chaining +
  no-fine-timestamps contain it, but do not eliminate it — a sufficiently unique single term can still
  stand out. This is the AOL-2006 failure mode, and thresholding is a mitigation, not a cure.
- `k = 1` must be understood and documented as *no anonymity, single operator* — acceptable only
  because the data lives on that operator's own machine.

**Commits us to**
- `k ≥ 20` on any multi-user deployment, enforced in config validation, not convention (inherited
  from 0015, re-audited here for the history view).
- **The signal stores live in an ephemeral Redis** (`redis-signals`: no RDB, no AOF, no volume,
  excluded from backups; `queue.signals_url`). The persistent queue Redis's AOF is an *ordered*
  command log — it would chain hash↔plaintext writes back into sessions, and backups of it would
  make terms outlive the sliding window — so "windowed and unchainable" is only true if the store's
  persistence layer is as forgetful as its keys. Losing the instance loses ranking hints, never
  data of record.
- No session-grouping field and no fine-grained per-event timestamp in the history store, ever — the
  moment either exists, thresholding stops protecting content.
- A privacy page that states, per mode, what is stored (terms, counts, clicks — no identifiers) and
  what is not.

## Alternatives

| Option | Why not |
|---|---|
| Keep only the ranking counters (0015), no browsable history | Forgoes the operator visibility M7-T10 exists to deliver; the counters already retain the term, so the marginal privacy cost of *reading* them is small and the honest move is to disclose it, not hide the capability |
| Threshold *storage* too (never write a term seen fewer than k times) | Cannot — you must count from the first occurrence to reach k. Thresholding belongs at read time; pretending it can gate writes is the error 0015's `surfaceable` predicate already avoids |
| Differential-privacy aggregate history | The route ADR-0008 names; at this corpus/traffic scale the noise swamps an already-sparse signal. Revisit at scale, as 0015 says |
| Per-account saved history (opt-in) | Builds the exact profile the engine promises not to build; against the product's reason to exist |

## Revisit when

- A multi-user deployment appears — re-audit end to end that `k ≥ 20` holds, that no path lowers it,
  and that no session-grouping or fine-timestamp field has crept into the history store.
- Traffic is high enough that differential privacy beats bare k-anonymity — switch the counters to a
  DP mechanism, per 0015.
- Content self-identification proves to bite in practice despite thresholding — tighten the window,
  raise `k`, or move to DP; do **not** quietly keep the plaintext.

## Related

[[ADR-0008 - No Query Logging]] · [[ADR-0015 - Anonymous Interaction Signals for Ranking]] ·
[[ADR-0001 - Two-Plane Architecture]] · [[Security and Privacy]] · [[Observability]] ·
[[Interaction Signals]] · [[Interaction Signals|weak_coverage]] · [[Decision Log]] ·
[[Milestone 7 - Federated Retrieval and External Tools]]

## Where it stands (2026-08-27)

- `/admin/interaction` exists (`crates/xustive-api/src/lib.rs`, `admin.rs`). `redis-signals` is defined in `deploy/docker-compose.yml` with no volume, and `scripts/backup.sh` deliberately excludes it.
- **Gap:** `QueueConfig::signals_url()` falls back to `queue.url` when unset (`crates/xustive-core/src/config.rs`), and only `config/dev.toml` sets `signals_url`; `config/prod.toml`, `staging.toml` and `ci.toml` do not, and nothing in compose references `redis-signals`. Outside dev, every signal namespace — including the default-on weak-coverage plaintext terms — is written to the persistent, backed-up queue Redis. Fix: set `queue.signals_url` in the non-dev configs and point it at the ephemeral instance.
- The privacy page (`web/app/[lang]/privacy/page.tsx`) does not mention that the connection address is read for approximate location ([[ADR-0020 - Approximate Location from a Local Database]]).
