---
tags:
  - component
  - ingestion
component-id: C23
binary: xustive-cli (crawld) · xustive-api (result-time collapse)
status: built
updated: 2026-08-27
---

# Deduplication Service

> **ID** C23 · **Runs in** the crawl daemon (`crates/xustive-ingest`: `dedup.rs`,
> `simhash_index.rs`) and the ranker (`crates/xustive-search/src/rank.rs`) · **Upstream**
> [[Content Parser]] · **Downstream** [[Task Queue]] (`q:index`), [[Query Pipeline]]

## 1. Purpose

Stop the same content from appearing many times. In this corpus that is not a rare edge case: the
same press release is republished by six sites, the same job posting is cross-posted to twenty
Facebook groups, and the same image circulates for weeks. Without dedup, a results page is one story
repeated ten times.

## 2. What exists today

The original design was one `Deduplicator` trait returning a four-way verdict. What was built is
smaller and split across two planes, and the split is the interesting part:

| Check | Where | Built? |
|:---|:---|:---|
| Same URL within a run (`seen:` generations) | `frontier.rs` ([[Crawler Orchestrator]]) | yes |
| Exact body, across runs (`content_hash`) | `dedup.rs`, wired in `crawld.rs` | yes |
| Near-duplicate at index time (SimHash banding) | `simhash_index.rs` | **built, not wired** (2026-08-27) |
| Near-duplicate at result time (SimHash collapse) | `rank.rs` `collapse_near_duplicates` | yes |
| Winner selection with aggregated engagement | `dedup::select_winner` | **built, not wired** (2026-08-27) |
| Image reuse by pHash | `embed_cache.rs` ([[Vector Index]] §5) | yes |
| Revision of a page (same URL, new body) | `revisit.rs` ([[ADR-0011 - Adaptive Recrawl over Static Crawling]]) | yes, as recrawl policy |
| Cross-platform `cluster_id` | — | **not built** (2026-08-27) |

## 3. Interface

```rust
// crates/xustive-ingest/src/dedup.rs
impl Dedup {
    pub fn connect_in(url: &str, namespace: &str) -> Option<Self>;   // lazy; cannot fail on a slow Redis
    pub async fn is_new(&self, content_hash: &str) -> bool;          // SADD → 1 means new; errors → true
    pub async fn forget(&self, content_hash: &str);                  // takedown / forced reindex
}
pub fn select_winner(candidates: &[Candidate]) -> Option<Winner>;  // §4.3

// crates/xustive-ingest/src/simhash_index.rs
pub const BANDS: usize = 4;  pub const NEAR_DISTANCE: u32 = 3;
impl SimHashIndex { pub async fn find_near(&self, simhash: u64) -> Option<String>; pub async fn index(..) }
```

`crawld` calls `is_new` on every parsed document before it is queued; a hit is counted as the
`duplicate` skip reason per source and per discovery channel ([[Crawler Console]]).

## 4. Internal Design

### 4.1 URL identity

The parser prefers `<link rel="canonical">` for `canonical_url` (`parse.rs`), and the frontier's
`seen:` set keeps one URL from being fetched twice within a generation. There is **no** tracking-
parameter stripping (`utm_*`, `fbclid`, …) and no `url → doc_id` map: two spellings of one URL that
fetch the same body are caught by the content hash instead. Good enough so far, because the body
check is cheap and exact; the canonicalisation table stays an open item.

### 4.2 Exact content

`content_hash` is BLAKE3 over the extracted, normalised body — the same hash `revisit.rs` compares
across fetches — so a syndicated wire story at two URLs hashes the same. The store is one Redis set
`frontier:seen_hashes`, checked and recorded in a single `SADD` so two workers racing on the same
hash cannot both see it as new. An empty hash is always "new": it means the body was never hashed,
and must not collapse every unhashed document into one.

No Bloom filter, no TTL. At the current corpus (hundreds of thousands, not tens of millions) the
plain set is well inside the 1 GB queue Redis, and the deployment's memory backstop
([[Task Queue]] §4.5) would show it long before it mattered.

**Fail-open, deliberately.** Any Redis error returns `true` — index the document. Treating an
unknown as a duplicate would make a Redis outage silently drop real documents, and a dropped
document is gone: the site was crawled, politely, and the result thrown away. An accidental
duplicate costs nothing, because the indexer writes keyed by id and a repeated write is a no-op.

### 4.3 Near duplicate (SimHash)

The parser stamps `simhash` (64-bit, hex) on every article-length body ([[Content Parser]]). Two
consumers exist:

- **Result time (wired).** `rank.rs` folds hits within Hamming distance
  `simhash_collapse_distance = 3` into the best-scoring copy, which given the trust weight is
  usually the most accountable publisher. The folded hits ride along as `collapsed` and the
  `explain` block counts them. This is what actually keeps six copies of a press release to one slot.
- **Index time (built, not wired).** `SimHashIndex` splits the hash into four 16-bit bands and
  indexes each under `frontier:sim:{band}:{value}`. Four bands are not arbitrary: by pigeonhole,
  two hashes differing in ≤ 3 bits must share a band, so the lookup has **no false negatives** at
  `NEAR_DISTANCE = 3`; candidates are then confirmed by full Hamming distance. It fails open like
  the exact check. It has Redis tests (`tests/simhash_redis.rs`) but `crawld` does not call it yet —
  result-time collapse was enough, and index-time dropping needs the winner rule below wired first.

The 4–8 "same story, different wording" band is deliberately **not** collapsed anywhere: that is a
cluster, not a duplicate, and clusters are not built.

**Which copy wins** (`select_winner`, unit-tested, not yet called): a *trusted* date beats an
untrusted one first — a guessed date is usually the crawl time, which would crown a copy as the
original — then earliest `published_at`, then higher source trust, then longer body, then id for
determinism. Engagement is summed across the set onto the winner.

### 4.4 Image dedup

Per image, a 64-bit dHash `phash` (`xustive-media::phash`), stamped by whichever of the OCR or
embed passes fetched the bytes first ([[Image Pipeline]]). Its one live use is the
`phash → CLIP vector` cache: the same picture reposted at another URL costs a Redis read, not a
model call. The reused vector is still upserted as a **new point** for this image and document —
reuse saves the model call, it does not merge documents. There is no Hamming-distance image
matching and no "short body + same image ⇒ same post" boost.

### 4.5 Revisions and volatile pages

Handled by the recrawl policy, not here: `revisit.rs` compares `content_hash` across fetches and
halves/grows the interval; a page that keeps changing at the floor is marked volatile and the
orchestrator skips it (`skip("volatile")`). See [[ADR-0011 - Adaptive Recrawl over Static Crawling]].

## 5. Configuration

None in `config/*.toml`. Constants live in code: `BANDS = 4`, `NEAR_DISTANCE = 3`,
`simhash_collapse_distance = 3` (`rank.rs` weights), `[vector] embed_cache_ttl_days = 30`. The
Redis namespace is `frontier` (shared with the frontier and raw store).

## 6. Data

`frontier:seen_hashes` (set), `frontier:sim:{0..3}:{u16}` (sets of `hash\tid`, unused),
`frontier:vecphash:{phash}` (little-endian f32 bytes, TTL). No dedup key has a TTL except the
vector cache.

## 7. Failure Modes

| Failure | Response |
|:---|:---|
| Redis unavailable | **fail open** — the document is queued; counted nowhere as a duplicate |
| Same hash from two workers at once | atomic `SADD`; exactly one sees it as new |
| Unhashed body (empty `content_hash`) | always new |
| Result page with many near-copies | collapsed at rank time; a copy scored below the survivor is never shown twice |
| Takedown | `forget(hash)` so a re-crawl can index the page again if it is ever allowed back |

## 8. Testing

`tests/dedup_redis.rs` (new/seen/forget, two URLs one body), `tests/dedup_quality.rs`,
`tests/simhash_redis.rs`, `tests/embed_cache_redis.rs`; `select_winner` and the band arithmetic are
unit-tested; `rank.rs` tests the collapse at distance 1 vs 64 and that the higher-scoring copy
survives. All Redis tests skip when no Redis is reachable.

## 9. Open Questions

- [ ] Wire `SimHashIndex` + `select_winner` into `crawld` so near-duplicates stop entering the
      index at all, or keep result-time collapse as the whole answer? Result-time is simpler and
      never loses a document; index-time saves index space.
- [ ] Tracking-parameter canonicalisation before the frontier `seen:` check.
- [ ] Should the exact-hash set get a TTL or a Bloom front, or is the memory backstop enough?
- [ ] Cross-language duplicates (the same story in Arabic and French) — needs the text embeddings
      now in [[Vector Index]]; nothing uses them for this yet.

## Related

[[ADR-0011 - Adaptive Recrawl over Static Crawling]] · [[Content Parser]] ·
[[Enrichment Pipeline]] · [[Query Pipeline]] · [[Image Pipeline]] · [[Vector Index]] ·
[[Ranking and Relevance]] · [[Task Queue]]
