---
tags:
  - solutions
  - index
date: 2026-08-26
---
# Solutions

One document per problem in the [[Problems|problems register]]
([../Problems.md](../Problems.md)): what was built, which knobs it introduced, what was
deliberately left undone and why, and how it was verified. The register keeps the analysis of what
was wrong; these keep the design of what replaced it.

| Problem | Solution | Solved | In one line |
|---|---|---|---|
| PROB-001 | [[PROB-001 - Bounded Frontier and Queue]] ([file](<PROB-001 - Bounded Frontier and Queue.md>)) | 2026-08-25 | The crawl can no longer fill Redis: linear branching, enforced ceilings with worst-tail eviction, per-host lifetime budgets, generational seen-sets, and an 85% memory backstop. |
| PROB-002 | [[PROB-002 - Crawl and Index Throughput]] ([file](<PROB-002 - Crawl and Index Throughput.md>)) | 2026-08-25 | The per-page overhead is gone — a page enqueues in ~3 round trips, probes throttle, the indexer batches, healthy hosts earn 1 rps. The remaining ceiling is host diversity, an operator decision. |
| PROB-003 | [[PROB-003 - Admin Console Coverage]] ([file](<PROB-003 - Admin Console Coverage.md>)) | 2026-08-26 | The console shows what the system is running with and what it is worth: effective config, capacity alarm, evaluation trail — and the controls to pause, curate, and triage from the browser. |
| PROB-004 | [[PROB-004 - Index Throughput Decay]] ([file](<PROB-004 - Index Throughput Decay.md>)) | 2026-08-29 | Indexing fell from 260 to 10 documents a minute as the index outgrew the container's 5 GiB page cache and the `byWord` proximity database grew with it; 16 GB and `byAttribute` restore it. |

Bugs, as distinct from problems, live in the tracker:
[2026-08-25 - Code Audit Findings.md](<../2026-08-25 - Code Audit Findings.md>).
