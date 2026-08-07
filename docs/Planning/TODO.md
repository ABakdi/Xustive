---
tags:
  - planning
  - moc
type: index
status: living
updated: 2026-08-06
---

# TODO — Master Implementation Plan

> The single place to see where the project is. Each milestone has its own note with tasks broken
> into subtasks; this note holds the shape, the gates, and the cross-cutting work.
> Parent: [[Home]]

---

## 1. How to Use This

- **Milestone notes** hold the actual checkboxes. Tick them there; this note tracks completion at the
  task level.
- A milestone is **done** when every task is ticked **and** its exit gate passes. A gate is not
  negotiable-by-vibes — it is a measurement.
- Tasks are labelled with the component id they touch (`C12`) so commits, notes, and work items line
  up ([[Component Map]]).
- Anything blocked lands in §5 with a named owner. A blocker without an owner is a wish.

Task id format: `M<milestone>-T<task>` → `M1-T04`. Subtasks are `M1-T04.3`.

---

## 2. Milestones

| # | Milestone | Focus | Exit gate |
|:--|:---|:---|:---|
| **M0** | [[Milestone 0 - Foundations]] | repo, infra, index, a search box that works | 10k docs searchable end-to-end |
| **M1** | [[Milestone 1 - Text Search MVP]] | the full text search product | nDCG@10 ≥ 0.60 on the golden set; p95 ≤ 200 ms |
| **M1B** | [[Milestone 1B - Frontend and Instant Answers]] | Next.js UI + instant-answer tools | Rust renderer deleted; no-JS path passes; zero false tool activations |
| **M2** | [[Milestone 2 - Multimodal Input]] | voice and image | WER/OCR targets met; image search ≤ 500 ms |
| **M3** | [[Milestone 3 - Ingestion at Scale]] | real crawling + direct social collection | 1M documents; identity lifespan ≥ 60 d; cloaking detected |
| **M4** | [[Milestone 4 - Quality and Operations]] | make it survivable | load test passes at 10M docs; restore drill green |
| **M5** | [[Milestone 5 - Beta Launch]] | legal, a11y, launch | legal checklist clear; a11y AA; public beta |

```
M0 ──► M1 ──► M2 ──┐
        │          ├──► M4 ──► M5
        └──► M3 ───┘
```

M2 and M3 are independent after M1 and can run in parallel with enough people. **M3 has a wall-clock
critical path**: identity warm-up takes 10+ days per account and cannot be parallelised away, so
account acquisition and warm-up start during M1 ([[Session Manager]] §4.4).

---

## 3. Progress

Tick as tasks complete in the milestone notes.

### M0 — Foundations
- [x] M0-T01 Repository and workspace skeleton
- [x] M0-T02 Docker Compose infrastructure
- [x] M0-T03 `xustive-core` types from [[Data Model]]
- [x] M0-T04 `xustive-text` normalisation + symmetry test
- [x] M0-T05 Meilisearch index settings and migration job
- [x] M0-T06 Minimal `xustive-api` with `/search`
- [x] M0-T07 Sample corpus + `make seed`
- [x] M0-T08 Minimal UI: search box and result list
- [ ] M0-T09 CI pipeline skeleton
- [ ] M0-T10 Fixture site for offline development

### M1 — Text Search MVP
- [ ] M1-T01 `xustive-telemetry` with privacy guards
- [ ] M1-T02 [[API Gateway]] middleware stack
- [x] M1-T03 [[Language Detector]] + lexicons
- [x] M1-T04 [[Query Expander]] + transliteration + lexicons
- [ ] M1-T05 [[Query Pipeline]] orchestration and re-rank
- [ ] M1-T06 [[Ranking and Relevance]] implementation and tuning
- [ ] M1-T07 [[Sentiment Engine]] lexicon mode
- [x] M1-T08 [[Summarizer]] with validation — grounded summaries from local Qwen2.5, runtime GPU/CPU switching. Streaming dropped deliberately; faithfulness evaluation still blocked on B7
- [ ] M1-T09 [[Autocomplete Service]]
- [ ] M1-T10 [[Content Parser]] HTML cascade
- [ ] M1-T11 [[Indexer Worker]] batching
- [ ] M1-T12 [[Task Queue]] abstraction
- [ ] M1-T13 Full UI: [[UI - Results Page]], [[UI - Home Page]], [[UI - Filters and Facets]]
- [ ] M1-T14 [[UI - RTL and Localization]] and four UI languages
- [ ] M1-T15 Relevance evaluation harness + golden set v1

### M1B — Frontend and Instant Answers

Breakdown in [[Milestone 1B - Frontend and Instant Answers]].

- [ ] M1B-T01 Frontend foundation — Next.js, `[lang]` routing, typed client, budgets in CI
- [ ] M1B-T02 Design language — oklch tokens, IBM Plex, shadcn primitives rewritten
- [ ] M1B-T03 Port the existing UI, then **delete the Rust renderer**
- [ ] M1B-T04 Tool framework — matching, arbitration, the shared card
- [ ] M1B-T05 Tier 1 tools — calculator, units, currency, translate, weather, prayer, time
- [ ] M1B-T06 `xustive-toold` — scheduled fetch into a cache the no-egress serving plane reads
- [ ] M1B-T07 Tier 2 and 3 tools
- [ ] M1B-T08 Localisation — catalogues, plurals, numerals, visual regression

### M2 — Multimodal Input
- [ ] M2-T01 `xustive-ml` service scaffold and model management
- [ ] M2-T02 [[Speech to Text]] pipeline
- [ ] M2-T03 [[UI - Voice Search]]
- [ ] M2-T04 [[Image Pipeline]] OCR path
- [ ] M2-T05 CLIP embedding + [[Vector Index]]
- [ ] M2-T06 [[UI - Image Search]]
- [ ] M2-T07 Index-side media enrichment
- [ ] M2-T08 Quality suites: WER, CER, ANN recall

### M3 — Ingestion at Scale
- [ ] M3-T01a [[Session Manager]] ← **start in M1**, warm-up is wall-clock
- [ ] M3-T01b [[Fingerprint Engine]]
- [ ] M3-T01c [[Signature Service]]
- [ ] M3-T01d Legal entity, Law 18-07, provider due diligence *(reduced scope)*
- [ ] M3-T02 [[Politeness and Robots]] — incl. the two crawl profiles
- [ ] M3-T03 [[Crawler Orchestrator]] frontier and scheduling
- [ ] M3-T04 [[Web Fetcher]] plain + impersonated + stealth headless
- [ ] M3-T05 [[Deduplication Service]]
- [ ] M3-T06 [[Enrichment Pipeline]] step framework
- [ ] M3-T07 [[Proxy Manager]] residential/mobile pools *(now required)*
- [ ] M3-T08 [[Social Connector - Facebook]] (gated on T01a/b/c)
- [ ] M3-T09 [[Social Connector - Instagram]] (gated on T01a/b/c)
- [ ] M3-T10 [[Social Connector - TikTok]] (gated on T01a/b/c)
- [ ] M3-T11 [[Data Sources Registry]] seeding to ~500 sources
- [ ] M3-T12 [[Admin and Source Submission]] admin surface + takedown path

### M4 — Quality and Operations
- [ ] M4-T01 [[Observability]] metrics, dashboards, alerts
- [ ] M4-T02 [[Error Handling and Resilience]] retries, breakers, DLQ tooling
- [ ] M4-T03 Load testing to [[Performance Budgets]]
- [ ] M4-T04 Backup and restore drills
- [ ] M4-T05 Scale the index to 10M documents
- [ ] M4-T06 [[Sentiment Engine]] evaluation and possible transformer mode
- [ ] M4-T07 Spam and quality scoring tuning
- [ ] M4-T08 Security review and penetration testing
- [ ] M4-T09 Runbooks for every alert

### M5 — Beta Launch
- [ ] M5-T01 [[Legal and Compliance]] pre-launch checklist cleared
- [ ] M5-T02 [[UI - Accessibility]] full AA pass
- [ ] M5-T03 Public "Submit a Source" flow
- [ ] M5-T04 Static pages: about, privacy, `/bot`, terms
- [ ] M5-T05 Takedown process staffed and rehearsed
- [ ] M5-T06 Native-speaker review of all UI strings
- [ ] M5-T07 Beta programme and feedback loop
- [ ] M5-T08 Launch runbook and rollback plan

---

## 4. Cross-Cutting Tracks

Work that does not belong to one milestone and must not be deferred into a "polish" phase that never
arrives.

### Data curation (continuous from M1)
- [ ] Darija marker lexicon → 1 500 terms ([[Language Detector]])
- [ ] Expansion lexicon → entities, synonyms, question words ([[Query Expander]])
- [ ] Sentiment lexicons ×4, Darija hand-built ~2 000 terms ([[Sentiment Engine]])
- [ ] Per-domain parser rules for the top 50 sources ([[Content Parser]])
- [ ] Wilaya/commune gazetteer
- [ ] Spam phrase list
- [ ] **Owner: unassigned — this needs a named person, not a rota**

### Evaluation (continuous from M1)
- [ ] Golden query set → 200 judged queries, 4 languages
- [ ] Language detection labelled set → 1 000
- [ ] Sentiment labelled set → 1 000
- [ ] Summary faithfulness set → 100
- [ ] Every reported quality complaint becomes a new golden row

### Security (continuous from M0)
- [ ] `SafeUrl` before any fetching code exists (M0, not later)
- [ ] Telemetry lint in CI from the first `tracing` call — query **and** credential fields
- [ ] Egress test in CI from the first container
- [ ] Dependency audit (`cargo-deny`, `cargo-audit`) in CI

### Collection maintenance (continuous from M3, **permanent**)
- [ ] Signer re-extraction when platforms rotate ([[Signature Service]] §4.5)
- [ ] Fingerprint profile version ageing and successor migration
- [ ] Access-path repair as platform DOM/endpoints change
- [ ] Identity pool refresh — replace burned accounts, keep pool ≥ `min_pool_size`
- [ ] Canary fixture upkeep so cloaking detection stays trustworthy
- [ ] **Owner: unassigned ← B9. Without one, collection decays to zero within ~2 months.**

### Documentation
- [ ] Keep component notes' `status` frontmatter current: `specified` → `implemented` → `verified`
- [ ] Every resolved open question becomes an ADR or a note edit
- [ ] Runbook per alert before that alert goes live

---

## 5. Blockers and Decisions Needed

| # | Item | Blocks | Owner | Needed by |
|:--|:---|:---|:---|:---|
| B1 | **Account acquisition + pool sizing** for FB/IG; warm-up is 10+ days wall-clock | M3-T08/09 | — | **start in M1** |
| B2 | **Residential/mobile proxy provider** — DZ coverage across ≥ 4 ASNs, exit-node consent | M3-T07, all platform collection | — | before M3-T07 |
| B3 | Monthly bandwidth budget for residential pools | M3-T07 cost gate | — | M3 planning |
| ~~B4~~ | Is a 3B model good enough for Arabic summaries? | — | **Yes on quality, no on speed.** Resolved during M1; became a latency question, see [[Summarizer]] §8 | closed |
| B5 | Who owns lexicon and registry curation? | quality of everything | — | M1 |
| B6 | Hardware: is a GPU in the budget? | **Now blocking summary latency**, not just M2 | Code is ready: build with `--features cuda` and the device layer does the rest. Needs the CUDA toolkit installed on the host | urgent |
| B7 | Native Darija speakers for UI strings and evaluation | M1-T14, M5-T06 | — | M1 |
| B8 | Legal entity (for takedowns/submissions) + Law 18-07 position | M5-T01 | — | before M5 |
| B9 | Who owns the collection maintenance tail? Signer re-extraction, path repair, pool refresh | M3 onward, **permanently** | — | M3 |

**B1, B2, B5, B7, and B9 are people/procurement problems, not engineering problems**, and they are
the ones most likely to slip. B1 and B9 are new consequences of
[[ADR-0009 - Direct Collection for Social Platforms]]: an unwarmed pool cannot be rushed, and a
collection stack without a named maintainer degrades to zero within a couple of months of platform
changes.

---

## 6. Definition of Done (per task)

A task is done when **all** of these are true:

- [ ] Code merged with tests at the levels [[Testing Strategy]] specifies for it
- [ ] Its component note's §11 test plan is actually implemented
- [ ] Metrics and log events from the note's §9 are emitted
- [ ] It meets its [[Performance Budgets]] entry, measured
- [ ] Failure modes from the note's §7 are handled and at least the top three are tested
- [ ] The note's `status` frontmatter is updated
- [ ] Any decision made along the way is recorded in [[Decision Log]] or the note's §12

## Related

[[Home]] · [[Component Map]] · [[Testing Strategy]] · [[Performance Budgets]] · [[Decision Log]] ·
[[Legal and Compliance]]
