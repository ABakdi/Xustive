---
tags:
  - adr
  - ranking
  - federation
status: accepted
date: 2026-08-28
updated: 2026-08-28
follows: ADR-0017
---
# ADR-0031 - The Web's Verdict Is a Signal on Our Own Documents

> Part of [[Decision Log]] · Follows [[ADR-0017 - Query-Time Federation with External
> Metasearch]] · Milestone: [[Milestone 13 - Distilled Ranking]]

## Context

The federation exists because the local index is small and ranks poorly. ADR-0017 made it a
fallback and a discovery channel: live hits are shown, thin documents are eager-indexed, the
URLs are crawled. But the one thing the metasearch engine is *good* at — deciding which page
is worth returning for a query, with several engines voting — was discarded at the door. A
federated document carried only its provenance (`discovery = "federation"`), and a document we
had crawled ourselves learned nothing when the web returned it too.

## Decision

1. **Every federated sighting of a URL is recorded on the document**, whether the document was
   born from the federation or crawled on our own: `web.seen`, the engines that returned it,
   its best rank, the best SearXNG score, first and last seen. A flat `endorsement ∈ [0, 1]`
   summarises it for the index.
2. **Endorsement ranks.** In retrieval it breaks ties among equal text matches before quality
   and date; in the re-rank it is a bounded side weight like the others; and the leading tier
   of the page (`federated_first`) is defined by endorsement, not provenance.
3. **Distillation is immediate and takes priority in the crawl.** A federated URL is indexed
   from its snippet at once and queued at the front of its host, ahead of organic discoveries;
   an endorsed existing document is re-queued so the copy is refreshed.
4. **The relevance rule still governs.** The endorsement weight is inside the side-signal
   budget that cannot bridge a twenty-position relevance gap. The tier applies within the
   matched pool only. An endorsed irrelevant page does not appear for a query it does not
   match.

## Consequences

- The index learns from every search: the more people search, the more of the web's ranking
  the local index holds, and the faster the same query is answered locally.
- Two fields on every document; a settings change (`endorsement` sortable, in the ranking
  rules) that reindexes once.
- The signal is only as good as the metasearch engines behind SearXNG; a wrong verdict is
  distilled too. It is bounded and it decays with nothing — `seen` only grows — which is
  accepted for now; a time decay is noted as an open question.
- Privacy: an endorsement is about a *page*, not a person. No query text is written to the
  document ([[ADR-0030 - First-Party Search Data, Kept to Learn From]] keeps queries in the
  events index).

## Related

[[Ranking and Relevance]] · [[Federation Gateway]] · [[Search Index]] ·
[[ADR-0032 - A Cross-Encoder Reranks the Top of the Page, Fused by Reciprocal Rank]]
