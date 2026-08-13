---
tags:
  - component
  - ingestion
component-id: C23
binary: xustive-worker
status: specified
updated: 2026-08-06
---

# Deduplication Service

> **ID** C23 · **Binary** `xustive-worker` · **Upstream** [[Content Parser]] · **Downstream** [[Enrichment Pipeline]], [[Query Pipeline]]

## 1. Purpose

Stop the same content from appearing many times. In this corpus that is not a rare edge case: the
same press release is republished by six sites, the same job posting is cross-posted to twenty
Facebook groups, and the same image circulates for weeks. Without dedup, a results page is one story
repeated ten times.

## 2. Responsibilities

**In scope**: exact-content dedup; near-duplicate detection (SimHash); image dedup (pHash);
URL canonicalisation dedup; cross-platform "same story" clustering; deciding **keep vs merge vs drop**.

**Out of scope**: computing the hashes (→ [[Content Parser]], [[Image Pipeline]]); result-time
collapsing (→ [[Query Pipeline]], which uses the `simhash` this component validates).

## 3. Interface

```rust
pub trait Deduplicator: Send + Sync {
    async fn check(&self, doc: &Document) -> DedupVerdict;
}
pub enum DedupVerdict {
    New,                                  // index it
    ExactDuplicate { of: DocumentId },    // drop
    NearDuplicate  { of: DocumentId, distance: u32 },  // drop or merge, see §4.3
    Revision       { of: DocumentId },    // same URL, changed content → update in place
}
```

## 4. Internal Design

Four checks, cheapest first, short-circuiting.

### 4.1 URL identity

Canonicalise: lowercase host, strip `www.`, strip fragment, strip tracking params
(`utm_*`, `fbclid`, `gclid`, `igshid`, `ref`, `si`), sort remaining params, strip a trailing slash,
resolve to `canonical_url` when present.

Look up `url:{canonical_url_hash}` in Redis → if present with a *different* `content_hash`, the
verdict is `Revision`: same address, new content, so update the existing document rather than
creating a second one.

### 4.2 Exact content

`content_hash` (BLAKE3 of the normalised body) checked against a Redis set. Hit → `ExactDuplicate`.

Memory: a plain set of 10M 16-byte prefixes ≈ 400 MB. A **Bloom filter** (`RedisBloom`, fp 0.001)
front-runs it at ~18 MB and eliminates ~99.9 % of the set lookups.

### 4.3 Near duplicate (SimHash)

64-bit SimHash over 3-token shingles, weighted by token IDF. Candidate lookup by **banding**: split
the 64 bits into 4 bands of 16; index each band in a Redis hash `simhash:b{i}:{value} → [doc_ids]`.
Two documents within Hamming distance ≤ 3 must share at least one band — so we compare against a
handful of candidates, not 10M.

| Distance | Interpretation | Action |
|:---|:---|:---|
| 0–3 | same content, trivially different | drop the newcomer; record `duplicate_of` |
| 4–8 | same story, different wording | keep both, link them as a cluster |
| > 8 | different | `New` |

**Which copy wins** when dropping: prefer (1) the earlier `published_at`, (2) the higher source
`trust_tier`, (3) the longer `body`. The engagement counts of the dropped copy are **added** to the
survivor's `engagement.aggregated` so a widely cross-posted item still ranks as popular.

### 4.4 Image dedup

Per media item, 64-bit dHash `phash`. Hamming ≤ 5 → the same image. Two uses:
- Skip re-embedding an image already in [[Vector Index]]; reuse the existing `embedding_id`. This is
  a large cost saving — the same meme appears thousands of times.
- Boost near-duplicate confidence: two documents with short bodies and identical images are the same
  post even if the SimHash distance is large.

### 4.5 Cross-platform clustering

A `cluster_id` groups documents at SimHash distance 4–8 or sharing an image. The cluster's canonical
member is the highest-trust, earliest one. [[Query Pipeline]] shows the canonical member with a
`"+N similar"` affordance ([[UI - Results Page]]).

## 5. Configuration

| Key | Default |
|:---|:---|
| `simhash_bands` | 4 |
| `drop_distance` | 3 |
| `cluster_distance` | 8 |
| `phash_distance` | 5 |
| `bloom_fp_rate` | 0.001 |
| `dedup_key_ttl_days` | 180 |
| `min_body_tokens_for_simhash` | 20 (shorter → exact-hash only; SimHash is meaningless on 5 words) |
| `aggregate_engagement` | `true` |

## 6. Data

Redis: `dedup:bloom`, `dedup:hash → doc_id`, `simhash:b{0..3}:*`, `phash:* → embedding_id`,
`url:* → doc_id`. Sliding 180-day TTL bounds memory; an evicted key means at worst one re-indexed
duplicate, which the result-time collapse still catches.

Estimated Redis footprint at 10M documents: ~1.2 GB.

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Redis unavailable | command error | **fail open** — index the document. A duplicate is far better than a lost document |
| Bloom false positive | set lookup confirms | proceed to the exact check; no impact |
| SimHash band explosion (common boilerplate) | candidate list > 1 000 | truncate; compare the 100 most recent |
| Over-aggressive dedup drops distinct content | offline eval + user report | raise `drop_distance` threshold; add a fixture |
| Short documents all collide | `min_body_tokens_for_simhash` guard | exact-hash only |
| Revision loop (page changes every fetch, e.g. a timestamp in the body) | revision count per URL | after 5 revisions/day, mark the document `volatile` and stop re-indexing |

The fail-open stance is deliberate and is the opposite of the usual default: dedup is a *quality*
optimisation, never a correctness gate.

## 8. Performance

| Operation | Budget |
|:---|:---|
| Exact check (Bloom hit path) | ≤ 1 ms |
| SimHash band lookup + compare | ≤ 8 ms p95 |
| pHash check per image | ≤ 2 ms |
| Full verdict | ≤ 15 ms p95 |
| Throughput | ≥ 300 docs/s/worker |

## 9. Observability

`xustive_dedup_verdict_total{verdict}`, `xustive_dedup_distance` (histogram),
`xustive_dedup_cluster_size` (histogram), `xustive_dedup_duration_seconds`,
`xustive_dedup_fail_open_total`, `xustive_phash_reuse_total` (embedding cost avoided),
`xustive_volatile_docs_total`.

**Duplicate rate is a product health metric.** If < 5 %, dedup is probably not working; if > 60 %,
the crawl frontier is wasting budget on redundant sources.

## 10. Security

Hashes are of public content and carry no user data. Redis is internal-only. A hostile site could
attempt to poison dedup by publishing content matching a competitor's SimHash to suppress it — the
"which copy wins" rule in §4.3 prefers *earlier* publication and *higher trust*, so a later low-trust
copy can never displace an earlier high-trust one.

## 11. Testing

- Unit: URL canonicalisation table (tracking params, case, trailing slash, fragments); band indexing;
  winner selection rules.
- SimHash quality: 500 known duplicate pairs (republished articles) and 500 known distinct pairs;
  target precision ≥ 0.95, recall ≥ 0.85 at distance ≤ 3.
- Image: same photo re-encoded, resized, cropped 5 %, watermarked → all within `phash_distance`.
- Cross-post: the same job posting in 10 Facebook groups collapses to one document with aggregated
  engagement.
- Fail-open: kill Redis → documents still index.
- Volatile page: a fixture whose body includes `now()`; assert it stops re-indexing after 5 revisions.

## 12. Open Questions

- [ ] Is 180 days the right dedup key TTL? Longer means more memory; shorter means old content
      re-enters as "new".
- [ ] Should clusters be exposed in the API (`cluster_id`, `similar_count`) or stay internal?
- [ ] Cross-language duplicates (the same story in Arabic and French) — genuinely useful to cluster,
      but needs translation or multilingual embeddings. v2.

## Related

[[ADR-0011 - Adaptive Recrawl over Static Crawling]] ·
[[Content Parser]] · [[Enrichment Pipeline]] · [[Query Pipeline]] · [[Image Pipeline]] ·
[[Ranking and Relevance]] · [[Task Queue]]
