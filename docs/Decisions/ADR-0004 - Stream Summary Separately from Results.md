---
tags:
  - adr
adr-id: "0004"
status: accepted
date: 2026-08-06
---

# ADR-0004 - Stream Summary Separately from Results

## Status

Accepted. Constrains [[API Contract]], [[UI - Results Page]], [[Summarizer]], [[Query Pipeline]].

## Context

The product shows an AI summary above the result links. The retrieval budget is **200 ms**; the
summary budget is **2 500 ms** ([[Performance Budgets]]). If both travel in one response, every
search feels like a 2.5-second search — a **12× regression** on the number that defines whether a
search engine feels fast.

The summary is also the least reliable part of the system. It depends on a local LLM under
contention, it can be dropped under load, and its output can fail validation
([[Summarizer]] §4.5). Coupling the most reliable output (links) to the least reliable one (summary)
means the whole page fails when only the optional part failed.

## Decision

**Two requests.**

1. `GET /search` returns results, facets, pagination, and a single-use `summary_token`. This
   response is not blocked by summary generation in any way.
2. `GET /search/summary?token=…` opens an SSE stream that delivers the summary in `delta` events
   ([[API Contract]] §3).

The client renders results immediately, reserves a fixed-height block above them, and fills it as
tokens arrive ([[UI - Results Page]] §2). If `summary_token` is null, or the stream errors, the block
is removed and nothing is shown — no error, no spinner, no apology.

The token — not the query — is what travels in the second request, so the query string is never sent
twice and never lands in a second server-side context.

## Consequences

**Good**
- Time to usable results stays at ~70 ms server-side regardless of summary latency.
- Summary failure degrades to "no summary" instead of "no page" ([[Error Handling and Resilience]] §6).
- The summary can be dropped under load as the first degradation step, invisibly.
- Streaming makes the perceived summary latency the time-to-*first*-token (800 ms), not the total.
- Client disconnect aborts generation immediately, so a closed tab stops burning CPU.

**Bad**
- Two round trips instead of one — but the second is not on the critical path.
- SSE requires JavaScript; no-JS users get results without a summary
  ([[UI Specification]] §8). Acceptable, since the links are the product.
- The gateway must hold token→candidate state for 60 s, which is in-memory and therefore
  replica-affine ([[API Gateway]] §12 — sticky routing).
- Layout must reserve height for content that has not arrived, or CLS suffers. This is a real
  constraint the UI has to honour.

**Commits us to**
- The principle that the summary is an accelerator, never the answer
  ([[UI Specification]] §2). Any future change that makes results wait on the summary reverses this
  ADR.

## Alternatives

| Option | Why not |
|:---|:---|
| One response, wait for summary | every search feels 12× slower; summary failure fails the page |
| One response, summary optional and often absent | non-deterministic content in a cacheable response; no streaming |
| Generate summaries asynchronously and cache by query | a query-keyed cache is a query log ([[Security and Privacy]] P1) |
| WebSocket instead of SSE | bidirectional transport for a unidirectional stream; more proxy trouble |

## Revisit when

- Summary generation gets fast enough (< 150 ms TTFT) that a single response is viable — e.g. with a
  GPU and a smaller model.
- The token-state requirement becomes a real operational problem across replicas.

## Related

[[API Contract]] · [[Summarizer]] · [[UI - Results Page]] · [[Performance Budgets]] ·
[[Error Handling and Resilience]] · [[Decision Log]]
