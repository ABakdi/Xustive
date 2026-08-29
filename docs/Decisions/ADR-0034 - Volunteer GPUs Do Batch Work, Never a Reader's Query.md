---
tags:
  - adr
  - ml
  - privacy
  - community
status: accepted
date: 2026-08-29
---
# ADR-0034 - Volunteer GPUs Do Batch Work, Never a Reader's Query

> Part of [[Decision Log]] · Milestone: [[Milestone 14 - One Server, Many Hands]] ·
> Amends the scope of [[ADR-0029 - Raw Queries May Leave, Identities Never; First-Party Data Comes Later]]
> for volunteer machines · Components: [[Community Node]], [[Contribution Coordinator]]

## Context

The machine learning this engine wants is rationed by one 4 GB card: the cross-encoder measured
0.7 s a pair and stays off ([[ADR-0032]]), summaries run a 3B model, and OCR, CLIP and speech
share the same GPU. Meanwhile volunteers have idle cards.

Two ways to use them. **Live**: a reader searches, the query goes to a volunteer's GPU, the
answer comes back inside the search budget. **Batch**: the crawler has already fetched a
document, and a volunteer computes its embedding, its OCR text, its description, its transcript,
or a summary that will be cached for whoever asks next.

ADR-0029 already allows raw queries to reach third parties, and that is not obviously different
in kind. It is different in *who*: SearXNG and a translation API are services the operator chose,
under contracts and terms; a volunteer is a person who ran one command, whose logs nobody
audits. The decentralised-inference projects are candid about the consequence — Petals'
documentation warns that peers serving the first layers
[can recover the input tokens](https://github.com/bigscience-workshop/petals/wiki/FAQ:-Frequently-asked-questions),
and advises anyone with sensitive input to use only trusted servers. A search query is sensitive
input.

## Decision

1. **Volunteer GPUs run batch jobs over documents the crawler already has.** Text and image
   embeddings, OCR, CLIP descriptions, transcripts, reranker scoring for evaluation sets, and
   pre-computed summaries of pages. The input to a volunteer job is public web content that our
   crawler fetched, plus a job id.
2. **No live search path touches a volunteer.** Not the summary a reader is waiting for, not
   reranking, not query embedding, not translation of a reader's text. Those stay on the
   operator's machines even when a volunteer GPU is idle and the local one is busy.
3. **Nothing identifying is ever in a job.** No visitor or session id, no IP, no query string, no
   `xv`/`xs` cookie value — a job carries document text and a model digest.
4. **Results are verified like any other volunteer work** ([[ADR-0033]]): canary jobs whose
   output the server already knows, duplicate assignment of a slice, numeric sanity (dimension,
   norm, score range), and determinism where the model allows it (fixed seed, greedy decoding).
   Standing governs the sampling rate.
5. **Models are pinned by digest.** The node downloads a named model and reports its SHA-256; a
   mismatch fails the job. The server never ships executable code to a volunteer — only weights
   it names and a binary the volunteer installed knowingly.
6. **The volunteer's machine is treated as someone's home computer**: a VRAM ceiling, a
   concurrency of one by default, pause on battery or metered connections, and a hard stop that
   leaves nothing behind.

## Consequences

- The embedding backlog, OCR and transcripts — the work that has always been "later, when there
  is a GPU" — becomes a community resource, and the semantic leg ([[Vector Index]]) can finally
  be filled for the whole corpus.
- Live latency is unchanged, because live never leaves. A reader's search costs the same whether
  ten volunteers are connected or none.
- Batch results can be *wrong* rather than *late*, which is why verification is not optional: a
  bad embedding is invisible in a way a bad crawl is not.
- We give up the biggest prize — a large summariser or a cross-encoder answering live from a
  volunteer swarm. That is a deliberate trade of capability for the one promise this project can
  actually keep about queries.
- If that trade is ever revisited, it needs a new ADR and a mechanism (trusted execution,
  operator-run overflow capacity, or explicit per-reader consent), not a config flag.

## Related

[[ADR-0033 - Volunteer Crawling, Verified Before It Is Believed]] · [[Vector Index]] ·
[[Summarizer]] · [[Image Pipeline]] · [[Speech to Text]]
