---
tags:
  - component
  - ingestion
component-id: C17
binary: xustive-worker
status: specified
updated: 2026-08-06
---

# Enrichment Pipeline

> **ID** C17 · **Binary** `xustive-worker` · **Upstream** `q:enrich` · **Downstream** `q:index` → [[Indexer Worker]]

## 1. Purpose

Add everything the raw document lacks but search needs: sentiment, image understanding, quality and
spam scoring, geo hints, and topic labels. It is the last stage where a document can be improved, and
the one with the widest cost variance — a text-only post costs 5 ms, a post with four images costs
two seconds.

## 2. Responsibilities

**In scope**: orchestrating enrichment steps; sentiment via [[Sentiment Engine]]; media fetch and
analysis via [[Image Pipeline]]; quality and spam scoring; geo/wilaya hinting; topic classification;
comment enrichment; deciding what is optional under load.

**Out of scope**: the models themselves; indexing (→ [[Indexer Worker]]); dedup (→ [[Deduplication Service]],
which runs before this).

## 3. Interface

Consumes `q:enrich` → `{ document, comments }`. Produces `q:index` → `{ document, comments }` with
enrichment fields populated.

```rust
#[async_trait]
pub trait EnrichmentStep: Send + Sync {
    fn name(&self) -> &'static str;
    fn required(&self) -> bool;          // false ⇒ may be skipped under load
    async fn apply(&self, doc: &mut Document, ctx: &Ctx) -> Result<(), StepError>;
}
```

Steps are a `Vec<Box<dyn EnrichmentStep>>` executed in order. Adding an enrichment means adding one
implementation and one config line — nothing else changes.

## 4. Internal Design

### 4.1 Step order and cost

| # | Step | Required | Typical cost | Notes |
|:--|:---|:---|:---|:---|
| 1 | Language confirm | yes | 10 ms | re-run [[Language Detector]] on the full body (the parser only saw 2 KB) |
| 2 | Sentiment (document) | yes | 5 ms / 40 ms | [[Sentiment Engine]], lexicon or transformer |
| 3 | Sentiment (comments) | no | 5 ms × N | capped at `max_comments_scored` |
| 4 | Quality score | yes | 2 ms | §4.2 |
| 5 | Spam score | yes | 3 ms | §4.3 |
| 6 | Geo hint | no | 3 ms | gazetteer of 58 wilayas + communes |
| 7 | Topic labels | no | 4 ms | keyword rules → `topics[]` |
| 8 | Media fetch | no | 300 ms × M | size-capped, via [[Proxy Manager]] |
| 9 | OCR + CLIP + pHash | no | 400 ms × M | [[Image Pipeline]] |
| 10 | Body backfill from OCR | no | 1 ms | when `body` is empty and OCR is usable |

Under queue pressure (`yellow` or worse, [[Error Handling and Resilience]] §4) the optional steps are
skipped and the document is indexed without them, with `enrichment_level = "partial"` recorded. A
partially enriched document is re-queued at low priority for a later full pass.

**Rule: a document is never blocked from the index by an optional enrichment.** Search availability
of content beats completeness of its metadata.

### 4.2 Quality score (0…1)

A weighted blend, all cheap signals:

| Signal | Weight | Direction |
|:---|:---|:---|
| `body_len` (log-scaled, saturating at 3 000 chars) | 0.25 | longer is better, up to a point |
| Boilerplate ratio from [[Content Parser]] | 0.15 | lower is better |
| Has a real title (not a truncated body) | 0.10 | |
| Date precision (`second` > `day` > `unknown`) | 0.15 | |
| Source `trust_tier` | 0.20 | |
| Punctuation/structure sanity (not ALL CAPS, not emoji-only) | 0.10 | |
| Has media | 0.05 | |

### 4.3 Spam score (0…1)

| Signal | Notes |
|:---|:---|
| Phone-number density | classifieds spam pattern |
| Repeated-token ratio | keyword stuffing |
| Link density in body | link farms |
| Excessive emoji/hashtag ratio | `#dz #algerie #follow #like #f4f` |
| Known spam phrase list (`data/spam/phrases.tsv`) | Arabic, French, English |
| Author posting frequency | > 50 posts/day from one handle |
| Duplicate cluster size | the same text posted to 40 groups |

`spam_score > 0.8` → the document is indexed but hard-filtered from default results; it remains
findable by exact-phrase search. We prefer suppression over deletion because the classifier is
imperfect and false positives are invisible when content is deleted.

### 4.4 Media handling

- Fetch at most `max_media_per_doc` (4) images, each ≤ 5 MB, 10 s timeout, through
  [[Proxy Manager]], with `SafeUrl` validation.
- Skip fetching entirely when the `phash` is already known ([[Deduplication Service]] §4.4) — reuse
  the existing `embedding_id`.
- Instagram media is prioritised because its CDN URLs expire
  ([[Social Connector - Instagram]] §7).
- A failed image never fails the document.

### 4.5 Concurrency

Per-worker: a bounded `JoinSet` with `media_concurrency` (4). Documents are processed concurrently up
to `doc_concurrency` (8). Media work is the only part that blocks on network, so it is the only part
that needs its own limit.

## 5. Configuration

| Key | Default |
|:---|:---|
| `doc_concurrency` | 8 |
| `media_concurrency` | 4 |
| `max_media_per_doc` | 4 |
| `max_media_bytes` | 5 MiB |
| `media_timeout_ms` | 10000 |
| `max_comments_scored` | 100 |
| `spam_suppress_threshold` | 0.8 |
| `skip_optional_on_pressure` | `yellow` |
| `repass_partial_after_h` | 6 |
| `sentiment_mode` | `lexicon` \| `transformer` \| `hybrid` |

## 6. Data

Reads and mutates the `Document` in flight; reads gazetteer/spam/topic data files. Writes the
enriched document to `q:index` and embeddings to [[Vector Index]] (via [[Indexer Worker]]).

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Required step fails | `StepError` | retry once; then DLQ |
| Optional step fails | `StepError` | log, skip, mark `enrichment_level = "partial"`, continue |
| Media fetch timeout | 10 s cap | skip that image |
| Media URL expired (403) | status | one re-fetch of the parent post (Instagram), else skip |
| [[Image Pipeline]] unavailable | connection error | skip steps 8–10 entirely; re-pass later |
| Sentiment model OOM | allocator | fall back to lexicon mode |
| Step wedges (no timeout) | per-step 30 s watchdog | abort step, mark partial |
| Queue pressure | depth gauge | skip optional steps |

## 8. Performance

| Case | Budget |
|:---|:---|
| Text-only document | ≤ 30 ms p95 |
| Document with 1 image | ≤ 700 ms p95 |
| Document with 4 images | ≤ 2 s p95 |
| Throughput | ≥ 100 docs/s/worker (text), ≥ 10 docs/s (media-heavy) |
| Memory | ≤ 2 GB/worker |

## 9. Observability

`xustive_stage_duration_seconds{stage="enrich"}`, `xustive_enrich_step_duration_seconds{step}`,
`xustive_enrich_step_skipped_total{step,reason}`, `xustive_enrich_partial_total`,
`xustive_media_fetched_total{outcome}`, `xustive_quality_score` (histogram),
`xustive_spam_score` (histogram), `xustive_spam_suppressed_total`.

Watch the quality/spam histograms for drift: a sudden shift usually means a parser regression
upstream, not a change in the web.

## 10. Security

Media URLs are attacker-controlled (they come from crawled content), so every fetch passes `SafeUrl`
and the size/pixel caps in [[Image Pipeline]] §4.1 ([[Security and Privacy]] T3, T7). Enrichment never
executes content. Spam and topic data files are git-reviewed.

## 11. Testing

- Unit: each step in isolation with fixture documents; quality/spam scoring tables.
- Step-skipping: simulate queue pressure; assert optional steps are skipped and `partial` is marked.
- Failure isolation: make each optional step fail; assert the document still reaches `q:index`.
- Media: a document with 4 images including one 404, one oversized, one expired URL → the document
  indexes with the successful images only.
- Repass: a partial document is re-queued and completes on the second pass without duplicating.
- Spam: a labelled set of 300 spam/ham Algerian posts; target precision ≥ 0.9 at the 0.8 threshold
  (false positives suppress legitimate content, so precision matters more than recall).

## 12. Open Questions

- [ ] Should quality/spam be a learned model rather than hand-weighted rules? Rules are explainable
      and tunable without labelled data; a model needs a labelling effort we have not scoped.
- [ ] Is `topics[]` worth having before we have a UI that uses it?
- [ ] Should comment sentiment aggregate into a document-level "discussion sentiment" distinct from
      the post's own sentiment? (Probably yes — a positive post with 200 angry comments is a
      meaningfully different result.)

## Related

[[Sentiment Engine]] · [[Image Pipeline]] · [[Deduplication Service]] · [[Indexer Worker]] ·
[[Data Model]] · [[Ranking and Relevance]] · [[Error Handling and Resilience]]
