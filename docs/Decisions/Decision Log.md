---
tags:
  - adr
  - moc
type: index
status: living
updated: 2026-08-27
---

# Decision Log

> Architecture Decision Records. One note per decision that was **hard to make and expensive to
> reverse**. Easy or reversible choices belong in the component note, not here.
> Parent: [[Home]]

---

## 1. Index

| ID | Decision | Status | Affects |
|:---|:---|:---|:---|
| [[ADR-0001 - Two-Plane Architecture]] | Split serving and ingestion, coupled only through the index | implemented (amended by 0017; API runs on the host, not in compose) | [[System Architecture]] |
| [[ADR-0002 - Meilisearch as System of Record]] | No separate database; the index is the store | implemented (snapshots via `scripts/backup.sh`, not scheduled) | [[Search Index]], [[Data Model]] |
| [[ADR-0003 - Comments in a Separate Index]] | Comments are their own index, folded in at query time | **partly implemented** — index exists, no query-time fold | [[Data Model]], [[Query Pipeline]] |
| [[ADR-0004 - Stream Summary Separately from Results]] | Two requests: results first, summary after | **implemented with divergence** — token kept, second request is a JSON POST, not SSE | [[API Contract]], [[UI - Results Page]] |
| [[ADR-0005 - Local Quantised LLM for Summaries]] | 3B quantised model on CPU, no external API | implemented — default `qwen2.5-3b` is non-commercial (Qwen-Research); device switchable from admin | [[Summarizer]] |
| [[ADR-0006 - Redis Streams for the Ingestion Pipeline]] | Streams + consumer groups, not lists or a broker | **partly implemented** — one stream (`q:index`), fetch/parse/enrich in-process | [[Task Queue]] |
| [[ADR-0007 - API-First Social Access]] | No scraping fallback exists in the code | **superseded by 0009** | social connectors, [[Legal and Compliance]] |
| [[ADR-0008 - No Query Logging]] | Zero query retention, enforced structurally | accepted, amended by 0015/0017/0018; **partly implemented** (egress test, log scan, media-on-disk gaps) | [[Security and Privacy]], [[Observability]] |
| [[ADR-0009 - Direct Collection for Social Platforms]] | Direct collection is a first-class path; adds the collection layer | **partly implemented** — session/fingerprint/proxy built; no Signature Service, no social connectors | social connectors, [[Session Manager]], [[Fingerprint Engine]], [[Signature Service]], [[Proxy Manager]] |
| [[ADR-0010 - Next.js for the Frontend]] | React Server Components; `xustive-api` stops rendering HTML | implemented (Next 16, shadcn source-copied) | [[UI - Frontend Architecture]], [[UI - Design Language]] |
| [[ADR-0011 - Adaptive Recrawl over Static Crawling]] | recrawl on content longevity, not change rate; abandon volatile pages | **implemented with divergence** — additive interval growth; frontier priority is depth/trust/article, not `trust × p(change) × age` | [[Crawler Orchestrator]], [[Web Fetcher]], [[Deduplication Service]] |
| [[ADR-0012 - Discovery-Only Aggregation]] | external engines discover URLs; never called on the serving path | **superseded by 0013** (its Common Crawl / sitemap / Brave inputs are what runs today) | [[Crawler Orchestrator]], [[Query Pipeline]], [[API Gateway]] |
| [[ADR-0013 - Direct SERP Collection for Discovery]] | query Google directly for discovery, in the ingestion plane only, as the narrowest channel | **partly implemented** — channel coded, off by default, blocked on residential egress | [[Crawler Orchestrator]], [[Proxy Manager]], [[Session Manager]], [[Fingerprint Engine]] |
| [[ADR-0014 - Knowledge Panel from Wikipedia via the Web Tier]] | knowledge panel fetched by the Next server from Wikipedia; no API endpoint | **superseded by 0019 / 0023** — panel now from the local entity store, Wikipedia extract folded in | [[UI - Results Page]], [[API Contract]] |
| [[ADR-0015 - Anonymous Interaction Signals for Ranking]] | k-anonymous interaction counters feed ranking and re-crawl; default off | implemented (M6; TTL re-armed below half-window per BUG-039) | [[Ranking and Relevance]], [[Query Pipeline]], [[Interaction Signals]], [[Observability]] |
| [[ADR-0016 - Two OCR Engines with an Optional Unlimited-OCR Sidecar]] | dual OCR backends, optional Unlimited-OCR sidecar | implemented | OCR / multimodal ingest |
| [[ADR-0017 - Query-Time Federation with External Metasearch]] | live web federation through one allowlisted gateway; serving-plane no-egress kept as one hop; converge to standalone | implemented (M7; runtime switch `POST /admin/integrations`; images/videos categories in M9) — egress test does not yet assert "gateway only" | [[Federation Gateway]], [[Query Pipeline]], [[Crawler Orchestrator]], [[Ranking and Relevance]] |
| [[ADR-0018 - Anonymous Search History]] | identifier-free retention of the normalised term + counts; storage always identifier-free, `k` a multi-user surfacing floor; amends 0008's durable-storage row | **implemented with a deployment gap** — `queue.signals_url` unset outside dev, so signals land in the durable queue Redis | [[Security and Privacy]], [[Observability]], [[Interaction Signals]] |
| [[ADR-0019 - The Knowledge Layer]] | entity-keyed local knowledge store harvested on the ingestion plane; authorities linked by identifier, never scraped | implemented (M8) — follow-ups: demand queue, live fallback (0023), precision floor (0022), list answers | [[Knowledge Store|Knowledge Layer]], [[Instant Answers]], [[Tool Data Plane]], [[UI - Results Page]] |
| [[ADR-0020 - Approximate Location from a Local Database]] | client address resolved to a wilaya in-process against a bundled database, used for one request, never stored or sent | **partly implemented** — works as decided; DB-IP CC BY attribution missing; no mmdb staleness gauge | [[Instant Answers]], [[API Gateway]], [[Security and Privacy]] |
| [[ADR-0021 - Proxied Thumbnails with Signed URLs]] | thumbnails for the Images/Videos tabs served through a same-origin proxy that accepts only server-signed URLs; video linked, never embedded or fetched | implemented (M9) | [[UI - Search Verticals]], [[UI - Results Page]], [[Security and Privacy]] |
| [[ADR-0022 - Entity Resolution Prefers Silence to a Wrong Panel]] | resolver applies a cheap gate, a precision floor and a fixed rule order; when not confident it answers `204` rather than guessing | accepted, implemented | [[Knowledge Store|Knowledge Layer]], [[Instant Answers]] |
| [[ADR-0023 - Live Wikidata Fallback Judged by the Local Resolver]] | on a store miss the web tier gathers Wikidata candidates; the Rust resolver judges and renders them; the miss feeds the harvester's demand queue; amends 0014 | accepted, implemented | [[Knowledge Store|Knowledge Layer]], [[Instant Answers]], [[UI - Results Page]] |
| [[ADR-0024 - Two-Model Voice Transcription that Submits on Stop]] | `base` model for live partials, `small` for the final pass; compute type per model from measurement; audio in memory only; submits on stop | accepted, implemented | [[Speech to Text]], [[UI - Voice Search]] |
| [[ADR-0025 - Official Exchange Rate Only]] | currency card shows the official reference rate from one keyless publisher; the parallel rate is deliberately absent and the card says so | accepted, implemented | [[Instant Answers]], [[Tool Data Plane]] |
| [[ADR-0026 - The Reader's Language as a Bounded Ranking Signal]] | binary `ui_language` re-ranking term, weight 0.10, reorders equals only; Darija and Arabic count as each other; summary in the reader's language | accepted, implemented | [[Ranking and Relevance]], [[Query Pipeline]], [[Summarizer]] |
| [[ADR-0027 - Narrow the Search Under Load Instead of Failing]] | on a retrieval timeout retry once page-sized without facets or highlighting, mark `facets_degraded`, count it (BUG-041) | accepted, implemented | [[API Gateway]], [[Query Pipeline]], [[Error Handling and Resilience]], [[Performance Budgets]] |
| [[ADR-0028 - Reverse Image Search Sends Words to the Web, Never the Picture]] | accepted, in progress | The picture is read locally; SearXNG gets labels, never the image; visual ranking local only, the crawl closes the loop |

---

## 2. Open Decisions

Decisions we know we must make, with the milestone that forces them.

| Question | Forced by | Notes |
|:---|:---|:---|
| Which residential/mobile proxy provider? DZ coverage, ≥ 4 ASNs, exit-node consent | [[Milestone 2 - Ingestion at Scale]] | [[Proxy Manager]] §12 |
| Which JS runtime for signer execution — `deno_core`, `rusty_v8`, `boa`? | [[Milestone 2 - Ingestion at Scale]] | [[Signature Service]] §12 — prototype against a real bundle first |
| Which impersonation library — `rquest` or alternatives? | [[Milestone 2 - Ingestion at Scale]] | [[Fingerprint Engine]] §12 — validate JA4 accuracy before committing |
| Do we join closed groups with our identities? | [[Milestone 2 - Ingestion at Scale]] | default no; per-source, operator-performed ([[Social Connector - Facebook]] §4.3) |
| Is the embedded-JSON path alone enough for IG/TikTok monitoring? | [[Milestone 2 - Ingestion at Scale]] | would make collection near signature-independent — measure early |
| Store `translit_body` (index +35 %) or transliterate at query time? | [[Milestone 1 - Text Search MVP]] | [[Data Model]] §9 |
| One Meilisearch node at 10M docs, or a read replica? | [[Milestone 4 - Quality and Operations]] | [[Performance Budgets]] §10 — needs a real load test |
| Is a 3B model good enough for Arabic synthesis? | [[Milestone 1 - Text Search MVP]] | [[Summarizer]] §12 — decided by the faithfulness gate |
| Proxy remote thumbnails through our own server? | [[Milestone 5 - Beta Launch]] | [[Security and Privacy]] §9 — leaning yes |
| Enable aggregate popularity for autocomplete? | [[Milestone 3 - Multimodal Input]] | [[Autocomplete Service]] §12 — leaning no |
| Sentiment transformer mode: where does labelled data come from? | [[Milestone 4 - Quality and Operations]] | [[Sentiment Engine]] §12 |
| What happens if Facebook group access is unobtainable? | [[Milestone 2 - Ingestion at Scale]] | [[Legal and Compliance]] §9 — a product-strategy question |
| Service worker for static assets only? | [[Milestone 5 - Beta Launch]] | [[UI - States and Errors]] §8 |

---

## 3. Template

```markdown
---
tags: [adr]
adr-id: NNNN
status: proposed | accepted | implemented | partly implemented | superseded | rejected
date: YYYY-MM-DD
---
# ADR-NNNN - <Title>

## Status
## Context          — the forces, constraints, and what we knew at the time
## Decision         — what we chose, stated plainly
## Consequences     — good, bad, and what this now commits us to
## Alternatives     — what else we considered, and why not
## Revisit when     — the concrete signal that should reopen this
```

The **Revisit when** section is the one that keeps a decision log useful. A decision without a
stated trip-wire quietly becomes an assumption nobody remembers making.

---

## 4. Conventions

- ADRs are **immutable once accepted**. Changing your mind means writing a new ADR that supersedes
  the old one, and updating the old one's `status` to `superseded` with a link.
- Number sequentially, never reuse.
- Link the ADR from the components it constrains, and link back from the ADR.
- A decision that a component note describes as "deliberate", "on purpose", or "we chose not to"
  should either have an ADR or be demoted to a plain implementation detail.

## Related

[[Home]] · [[System Architecture]] · [[Component Map]] · [[TODO]]
