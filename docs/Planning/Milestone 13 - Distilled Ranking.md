---
tags:
  - planning
  - milestone
  - ranking
milestone: 13
status: in-progress
updated: 2026-08-28
progress: specified 2026-08-28; T01–T04 in build
---
# Milestone 13 - Distilled Ranking

> **Goal:** the web's judgement becomes a durable signal on *our* documents. Every result the
> metasearch federation returns is distilled into the local index at once and remembered as an
> **endorsement** on the document — how many times the web returned it, at what rank, from
> which engines — whether the document was born from that result or crawled on our own.
> Endorsed documents lead the page; the crawl reads them first; and the top of the page is
> optionally re-read by a small cross-encoder on the CPU, fused with the existing order by
> reciprocal rank so neither can run away with the list.
> **Exit gate:** a URL the federation returned is searchable locally on the *next* search with
> its endorsement recorded; a document that already existed and reappears on the web carries
> `web.seen ≥ 1` and outranks an unendorsed peer of equal relevance; the frontier reads
> federated URLs before organic discoveries and a test pins it; the reranker is a switch in the
> console, off by default, bounded by a timeout, and its effect is measured by the eval harness
> before it is turned on; the docs describe the ranking that exists.
> Parent: [[TODO]] · Previous: [[Milestone 12 - The Operator's Console]] · Decisions:
> [[ADR-0031 - The Web's Verdict Is a Signal on Our Own Documents]],
> [[ADR-0032 - A Cross-Encoder Reranks the Top of the Page, Fused by Reciprocal Rank]] ·
> Components: [[Ranking and Relevance]], [[Federation Gateway]], [[Search Index]],
> [[Query Pipeline]]

## Why This Milestone Exists

The index is small and its ranking is not accurate enough; that is *why* the product federates
to a self-hosted SearXNG ([[ADR-0017 - Query-Time Federation with External Metasearch]]). The
federation was built as a fallback and a discovery channel: live hits are shown, eager-indexed
as thin documents, and queued for a crawl. Reading the code on 2026-08-28, three things the
operator assumed were true are not:

1. **The web's ranking is thrown away.** SearXNG returns a `score` (the sum over engines of
   `weight / position`) and the list of engines that agreed on a URL. The client keeps the
   title, snippet, one engine name and the position, and stores none of it. A document born
   from a federated hit is indistinguishable, a day later, from one the crawler stumbled on.
2. **An existing document learns nothing from reappearing.** When a page we already crawled
   comes back from the federation, nothing is written to it. The user's expectation — "we
   either distilled it from there, or we crawled it and the web says it is good; either way it
   should rank higher" — has no field to live in.
3. **Ties are broken by date.** Meilisearch's rules (`words, typo, proximity, attribute, sort,
   exactness`) are buckets; on a small index many candidates tie, and the tie-breakers are
   `published_at:desc` then `quality_score:desc`. The re-rank then reads *position* as
   relevance. So among documents that matched equally, the newest wins — not the one the web
   agrees is good, not the one readers open.

`Weights.federated_first` (M12) already puts federated *provenance* first. It keys on
`discovery = "federation"`, which covers case 1 only, and only until the crawl overwrites the
thin document with a full one.

## What the research says

- **Metasearch scoring.** SearXNG's own merge is `score = Σ_engines weight / position`; a URL
  several engines return at high positions scores highest. That number, and the engine count,
  are the distilled judgement worth keeping ([SearXNG engine settings](https://docs.searxng.org/admin/settings/settings_engines.html),
  [scores discussion](https://github.com/searxng/searxng/discussions/1324)).
- **Fusing lists without calibration.** Reciprocal Rank Fusion (`Σ 1/(k+rank)`, k=60) fuses
  ranked lists from unrelated scorers without normalising their scores, and in the original
  evaluation beat learned fusions on LETOR
  ([OpenSearch](https://opensearch.org/blog/introducing-reciprocal-rank-fusion-hybrid-search/),
  [Azure AI Search](https://learn.microsoft.com/en-us/azure/search/hybrid-search-ranking)). It
  is the right tool for combining "our order" with "a model's order" when neither score is on
  the other's scale.
- **A small cross-encoder is affordable on CPU.** Qwen3-Reranker-0.6B ships as INT8 ONNX
  (~570 MB) and Q4 GGUF (~380 MB) and rescores query–document pairs to `[0, 1]`
  ([qwen3-embed](https://github.com/n24q02m/qwen3-embed),
  [onnx-community/Qwen3-Reranker-0.6B-ONNX](https://huggingface.co/onnx-community/Qwen3-Reranker-0.6B-ONNX)).
  Apache-2.0, Chinese open model, fits the 4 GB / CPU-only target; twenty pairs of a few
  hundred tokens is well inside a search budget.
- **Tie-breaking by a quality prior.** Meilisearch custom ranking rules (`field:desc`) apply
  only among documents equal under the text rules, which is exactly the tie we want a prior to
  settle ([ranking rules](https://www.meilisearch.com/docs/capabilities/full_text_search/relevancy/ranking_score)).
- **Behavioural priors stay bounded.** Click/dwell signals move rankings at every large
  engine, and every serious treatment (unbiased LTR) warns they are position-biased. M6 and
  M11 already hold that line — bounded weights, never able to lift an irrelevant document —
  and this milestone keeps it.

## Tasks

### T01 — The endorsement record

- **T01.1** `xustive-federation` keeps what SearXNG says about a hit: `score` and the full
  `engines` list, defaulted on the wire so mixed builds still agree.
- **T01.2** `Document.web: Option<WebEndorsement>` — `seen` (how many searches the web returned
  it for), `engines` (union, capped), `best_rank`, `score` (the best SearXNG score seen),
  `first_seen_at`, `last_seen_at` — and a flat `endorsement: f32` in `[0, 1]` derived from it
  so the index can rank on one number: `0.5·min(1, ln(1+seen)/ln 11) + 0.5/best_rank`.
- **T01.3** An `endorse` sink in the API (`endorse.rs`, the shape of `events.rs`): every
  federated response feeds it; it reads the current `web` of the ids that exist (one filtered
  fetch), folds the new sighting in, and writes partial updates (`{id, web, endorsement}`).
  URLs not yet in the index get their endorsement on the eager thin document instead, so no
  stub document is ever created.
- **T01.4** The documents console shows the badge (seen ×n · engines) and can sort by
  endorsement.

### T02 — Endorsed documents lead

- **T02.1** Meilisearch ranking rules: `endorsement:desc` before `quality_score:desc` and
  `published_at:desc`. Among equal text matches the web's verdict decides, then quality, then
  date. (A settings change; `xustive migrate` reindexes — see the note in M12.)
- **T02.2** `Weights.endorsement` (0.09) in the re-rank, with the other side weights rebalanced
  so their sum (0.47) stays under the twenty-position relevance gap (0.48); `Explain` carries
  it; the console's ranking editor shows it.
- **T02.3** The `federated_first` tier keys on *endorsement*, not provenance: a document with
  `web.seen ≥ 1` — born from the federation or crawled and later confirmed — is in the
  leading tier. Live web cards keep leading the merged page.
- **T02.4** Tests: an endorsed document outranks an unendorsed one of equal relevance; an
  endorsement cannot lift a document twenty positions; the tier admits a crawled-then-endorsed
  document.

### T03 — The crawl reads the web's picks first

- **T03.1** Confirm and pin: a federated URL is added at depth 0 with trust 40 and *promoted*
  to the front of its host queue (`ZADD XX i64::MIN`), ahead of every organic discovery on the
  host; a document already indexed that is endorsed again is re-queued the same way so the thin
  or stale copy is refreshed. A test on the frontier pins the ordering.
- **T03.2** The worker's merge (`update_documents`, M11) keeps `web`/`endorsement` when the
  full crawl replaces the thin document.

### T04 — A cross-encoder on the top of the page

- **T04.1** `services/reranker`: FastAPI, Qwen3-Reranker-0.6B INT8 ONNX via `qwen3-embed`
  (CPU; GPU when present). `POST /rerank {query, documents[]} → {scores[]}`, `GET /health`.
  Weights under `data/models/` via `scripts/fetch-models.sh`. Never logs a query or a document.
- **T04.2** `[reranker]` config (`enabled=false`, `url`, `timeout_ms=400`, `top_n=20`); a
  runtime switch in the console (persisted like the others).
- **T04.3** In the query pipeline, after the re-rank and before the federation merge: the top
  `top_n` local results (title + excerpt) go to the sidecar under the timeout; the model's
  order and ours are fused by RRF (k=60) *within each tier*; on timeout or error the page is
  unchanged and a `degraded{stage="reranker"}` metric ticks. `explain` carries the model score.
- **T04.4** Measured before it is turned on: `make eval` nDCG@10 with the switch off and on,
  recorded in the milestone; p95 search latency on the CPU-only reference machine.

### T05 — Docs and gates

- [[Ranking and Relevance]] describes the three stages (retrieval with the endorsement
  tie-break, the bounded re-rank with the endorsement weight and the endorsed tier, the
  optional fused cross-encoder); [[Federation Gateway]] §6 provenance lists what is kept;
  [[Search Index]] lists the new fields and rules; [[UI - Admin Console]] the badge, sort and
  switch; the README's feature row and milestone table; [[Decision Log]].

## Out of scope

- Learning weights from clicks (unbiased LTR): the events exist (M11), the volume does not.
- A dense-retrieval leg (embedding the corpus): a different milestone; the reranker here is a
  cross-encoder on candidates already retrieved.
- Replacing SearXNG's ranking with our own fusion of its engines: we take its merged verdict
  as given.

## Acceptance

| Check | How |
|:---|:---|
| A federated URL is searchable locally on the next search | search A (federation live) → search A again: the card is local, `discovery=federation`, `web.seen=1` |
| An existing document reappearing on the web is endorsed | crawl a page, search a query the federation returns it for → `web.seen=1`, tier 1 |
| Endorsement bounded | `Weights::check` still passes; the twenty-position test passes |
| Frontier order | unit test: federated URL pops before an organic depth-0 URL on the same host |
| Reranker off by default, bounded | config default; timeout test; `degraded{stage="reranker"}` |
| Measured | eval report before/after in `eval/reports/`, numbers in this file |

## Related

[[TODO]] · [[Milestone 7 - Federated Retrieval and External Tools]] ·
[[Milestone 11 - Learning from Readers]] · [[Ranking and Relevance]]
