---
tags:
  - adr
  - ranking
  - ml
status: accepted
date: 2026-08-28
updated: 2026-08-28
---
# ADR-0032 - A Cross-Encoder Reranks the Top of the Page, Fused by Reciprocal Rank

> Part of [[Decision Log]] · Milestone: [[Milestone 13 - Distilled Ranking]] · Hardware:
> the CPU-only reference machine ([[Performance Budgets]])

## Context

Stage 1 is lexical (Meilisearch), stage 2 is a linear blend of bounded priors over the engine's
*position*. Neither reads the query and the document together. On a small, uneven corpus that
shows: a page that mentions every query word in passing outranks one that is about the topic.
The remedies the field offers are a dense-retrieval leg (embedding the corpus — a milestone of
its own) and a cross-encoder that rescores a short list of candidates with the query in
context. The second is cheap on the candidates we already have.

## Decision

1. **A cross-encoder sidecar, off by default.** Qwen3-Reranker-0.6B (Apache-2.0, INT8 ONNX,
   CPU-capable, GPU when present) behind `services/reranker`, the shape of the CLIP and STT
   sidecars. The Rust side sends the top `top_n` local candidates (title + excerpt) and gets a
   score in `[0, 1]` per candidate, under a timeout.
2. **Fused, never substituted.** The model's order and the stage-2 order are combined by
   reciprocal rank fusion (`1/(60+rank)` each), within each tier. RRF needs no calibration
   between two scorers on different scales, and neither list can move a document more than
   the other allows. The tiers (endorsed first) and the per-domain cap are applied after.
3. **Bounded and measured.** Timeout or error leaves the page as stage 2 produced it and
   ticks a degraded metric. The switch lives in the console and persists. It is turned on
   only after the eval harness shows nDCG@10 does not regress and p95 latency stays inside
   the search budget on the reference machine.
4. **Nothing is logged.** The sidecar sees raw queries and excerpts (allowed since
   [[ADR-0029 - Raw Queries May Leave, Identities Never; First-Party Data Comes Later]] —
   and this one is first-party anyway) and logs only latency and status.

## Consequences

- +200–400 ms on a CPU search when on; zero when off. The timeout caps the worst case.
- A ~600 MB model under `data/models/`, fetched by script, not committed.
- The cross-encoder reads the *snippet*, so a document with a poor excerpt is judged on it —
  one more reason the excerpt extractor matters.
- Alternatives considered: a dense leg (later); replacing stage 2 with the model (loses the
  bounded priors and the tier); a bigger reranker (4B/8B do not fit the machine).

## Related

[[Ranking and Relevance]] · [[Query Pipeline]] ·
[[ADR-0031 - The Web's Verdict Is a Signal on Our Own Documents]]
