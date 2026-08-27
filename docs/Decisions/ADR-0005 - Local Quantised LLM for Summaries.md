---
tags:
  - adr
adr-id: "0005"
status: implemented
date: 2026-08-06
---

# ADR-0005 - Local Quantised LLM for Summaries

## Status

Implemented, with a licence caveat on the default model. Constrains [[Summarizer]], [[Deployment Topology]], [[Performance Budgets]].

## Context

The AI summary needs a language model that handles Arabic and French competently. The obvious,
cheapest-to-build option is a hosted API — better quality, no model ops, no GPU.

It is also disqualifying. Calling a hosted model means sending the user's query **and** the retrieved
passages to a third party outside Algeria, on every search. That directly contradicts the product's
central claim ([[Security and Privacy]] P1, P2) and the data-sovereignty requirement in the
functional spec. It is not a trade-off to be weighed; it is the one thing the product exists to
avoid.

Given local inference, the remaining question is size. A 7B model gives better Arabic synthesis but
needs a GPU to hit the latency budget. A 3B quantised model runs acceptably on CPU.

## Decision

Run a **quantised instruct model locally on CPU**: Qwen2.5-3B-Instruct at Q4_K_M (~2.2 GB) via
`llama-cpp-rs`, with 2 slots per `xustive-ml` replica.

Supporting choices:
- `temperature 0.2` — this is extraction, not creativity.
- Hard output cap: 3 sentences, 400 characters, 120 tokens.
- Reply `INSUFFICIENT` when the passages do not answer the question, and show nothing.
- Reject any summary with zero citations — the main hallucination guard
  ([[Summarizer]] §4.5).
- A GPU, if one is added, upgrades to a 7B model by config, not by redesign.

## Consequences

**Good**
- No user query or crawled passage ever leaves our infrastructure. The privacy claim survives.
- No per-query cost, no rate limits, no external dependency that can change its terms or its model
  under us.
- Latency is predictable and under our control; degradation is our decision, not an upstream's.
- Runs on the same hardware as everything else, with no GPU requirement for v1.

**Bad**
- **Quality is meaningfully lower than a frontier model**, particularly for Arabic synthesis. This is
  the real cost and it should not be understated.
- ~4 GB resident per replica, and only 2 concurrent generations — [[Summarizer]] drops requests under
  load rather than queueing.
- Model files (~2.2 GB) must be distributed, checksummed, and version-managed
  ([[Deployment Topology]] §5).
- Prompt injection from crawled content is now our problem to defend against
  ([[Security and Privacy]] §5).
- Model licences must permit commercial use — a research-only licence would invalidate this decision
  ([[Legal and Compliance]] §7).

**Commits us to**
- Owning summary quality as an engineering problem: the faithfulness gate (≥ 95 %, human-sampled) in
  [[Testing Strategy]] §7 is what keeps "lower quality" from becoming "wrong".
- Accepting no summary rather than a bad one. Every failure mode in [[Summarizer]] §7 resolves to
  *omit*, never to *guess*.

## Alternatives

| Option | Why not |
|:---|:---|
| Hosted API (Claude, GPT, Gemini) | sends queries and passages abroad — disqualifying |
| 7B model on CPU | ~4× slower; misses the 2.5 s budget |
| 7B model on GPU | better quality, but adds a hardware requirement and cost to v1; kept as a config-level upgrade path |
| Extractive summarisation (no LLM) | fast, cheap, no hallucination — but reads as three disconnected sentences rather than an answer. **Worth reconsidering if the faithfulness gate fails.** |
| No summary at all | the links are the product, so this is genuinely viable — but the summary is a headline feature |

## Revisit when

- The faithfulness gate cannot be met at 3B — then either add a GPU and go to 7B, or fall back to
  extractive summarisation.
- A GPU becomes available in the deployment budget.
- A materially better small Arabic-capable open model appears (this space moves fast; re-evaluate
  each milestone).

## Related

[[Summarizer]] · [[Security and Privacy]] · [[Performance Budgets]] · [[Deployment Topology]] ·
[[Testing Strategy]] · [[Legal and Compliance]] · [[Decision Log]]

## Where it stands (2026-08-27)

- Inference is `llama-cpp-2` behind the `llama` cargo feature (`crates/xustive-ml/Cargo.toml`), linked into `xustive-api` rather than a separate `xustive-ml` replica. `slots` default 2, `max_tokens` 120, low temperature, the prompt asks for 2–3 sentences ≤ 400 characters with a citation per sentence and `INSUFFICIENT` otherwise (`crates/xustive-ml/src/engine.rs`, `prompt.rs`).
- **Licence caveat.** The registry default is `qwen2.5-3b-instruct-q4`, which is the non-commercial *Qwen-Research* licence (`crates/xustive-ml/src/registry.rs`, `commercial_use: false`; the engine warns when it loads). The Apache-2.0 sizes (1.5B, 7B) are registered; a commercial launch must pin one (`summariser_model` in `config/*.toml`).
- Device is switchable at runtime from the admin page (`crates/xustive-api/src/admin.rs`, `device = "auto"` in `config/dev.toml`); CPU-only remains supported. The "GPU upgrades to 7B by config" path exists (`config/dev.toml` comment documents the download + `summariser_model` switch).
