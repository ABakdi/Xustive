---
tags:
  - component
  - serving
  - storage
component-id: C06
binary: meilisearch
status: specified
updated: 2026-08-06
---

# Search Index

> **ID** C06 · **Service** `meilisearch` · **Upstream** [[Indexer Worker]] · **Downstream** [[Query Pipeline]], [[Autocomplete Service]]

## 1. Purpose

The retrieval engine and, per [[ADR-0002 - Meilisearch as System of Record]], the system of record.
It provides sub-50 ms full-text search with typo tolerance, native Arabic tokenisation, faceting, and
custom ranking rules — without us writing an inverted index.

## 2. Responsibilities

**In scope**: storing `Document`, `Comment`, and `Source` records; tokenising Arabic/Latin text;
matching, typo tolerance, phrase search; facet counts; ranking stage 1; highlighting.

**Out of scope**: re-ranking with business signals (→ [[Query Pipeline]]); vector search
(→ [[Vector Index]]); durability guarantees beyond snapshots (→ [[Deployment Topology]] §7).

## 3. Interface

| Op | Endpoint | Caller |
|:---|:---|:---|
| Search | `POST /multi-search` | [[Query Pipeline]] |
| Suggest | `POST /indexes/documents/search` (limited attrs) | [[Autocomplete Service]] |
| Upsert | `POST /indexes/{ix}/documents` (batch) | [[Indexer Worker]] |
| Delete | `POST /indexes/{ix}/documents/delete-batch` | [[Admin and Source Submission]] |
| Settings | `PATCH /indexes/{ix}/settings` | migration job |
| Task poll | `GET /tasks/{uid}` | [[Indexer Worker]] |

Access is via scoped API keys: search-only for `xustive-api`, index-only for `xustive-worker`
([[Security and Privacy]] §7).

## 4. Internal Design (our configuration of it)

### 4.1 Indexes

| Index | Primary key | Approx docs | Notes |
|:---|:---|:---|:---|
| `documents_v1` (alias `documents`) | `id` | 10M | main corpus |
| `comments_v1` (alias `comments`) | `id` | 50M | see [[ADR-0003 - Comments in a Separate Index]] |
| `sources_v1` (alias `sources`) | `id` | 10k | registry, also served to admins |

Aliases enable zero-downtime reindexing ([[Data Model]] §7).

### 4.2 `documents` settings

```jsonc
{
  "searchableAttributes": ["title","excerpt","entities","body","translit_body","author.name"],
  "filterableAttributes": ["source_type","source_id","domain","language","script",
                           "sentiment.label","published_at","crawled_at","is_nsfw",
                           "quality_score","geo.wilaya","topics","robots_indexable"],
  "sortableAttributes": ["published_at","crawled_at","quality_score","engagement.likes"],
  "displayedAttributes": ["id","title","url","canonical_url","excerpt","source_type","source_id",
                          "domain","author","published_at","published_at_precision","sentiment",
                          "engagement","language","media","simhash","quality_score"],
  "rankingRules": ["words","typo","proximity","attribute","sort","exactness",
                   "published_at:desc","quality_score:desc"],
  "typoTolerance": {
    "enabled": true,
    "minWordSizeForTypos": { "oneTypo": 4, "twoTypos": 9 },
    "disableOnAttributes": ["entities"],
    "disableOnWords": ["وهران","قسنطينة","سونلغاز","Oran","Setif","CNAS"]
  },
  "faceting": { "maxValuesPerFacet": 100, "sortFacetValuesBy": { "*": "count" } },
  "pagination": { "maxTotalHits": 2000 },
  "separatorTokens": ["|","·","—"],
  "nonSeparatorTokens": ["@","#","_"],
  "dictionary": ["سونلغاز","Sonelgaz","CNAS","ANEM","Seaal","Ooredoo","Djezzy","Mobilis"],
  "stopWords": ["من","في","على","الى","عن","le","la","les","de","des","the","and"],
  "synonyms": { /* generated from data/expansion/*.tsv at deploy time */ }
}
```

Notes:
- `body` is searchable but ranked *below* `title`/`excerpt`/`entities` via the `attribute` rule.
- `dictionary` prevents the tokeniser from splitting known multi-part entity names.
- `synonyms` are generated from the same lexicon that feeds [[Query Expander]] — one source of truth.
  Meilisearch synonyms handle the common cases cheaply; the expander handles the generative ones.
- `maxTotalHits: 2000` bounds deep pagination cost; the UI caps at page 100 anyway.

### 4.3 `comments` settings

Searchable: `["body","author.name"]`. Filterable: `["document_id","source_type","sentiment.label",
"published_at","language"]`. Same typo settings. Ranking rules without the custom freshness rule —
comment recency is inherited from the parent at re-rank time.

### 4.4 Arabic tokenisation

Meilisearch uses `charabia`, which handles Arabic segmentation and normalisation. We additionally
normalise at write time in [[Content Parser]] (tatweel, diacritics, alef variants, digit folding)
because relying on any engine's internal normalisation for a *query-time/index-time symmetric*
guarantee is fragile. **The same `xustive-text` function must run on both sides.**

## 5. Configuration (service level)

| Env | Value | Notes |
|:---|:---|:---|
| `MEILI_ENV` | `production` | enables the master-key requirement |
| `MEILI_MASTER_KEY` | secret | migration job only |
| `MEILI_MAX_INDEXING_MEMORY` | `16Gb` | leave headroom for search |
| `MEILI_MAX_INDEXING_THREADS` | half of cores | so indexing cannot starve search |
| `MEILI_SNAPSHOT_DIR` / interval | 6 h | [[Deployment Topology]] §7 |
| `MEILI_HTTP_PAYLOAD_SIZE_LIMIT` | `100Mb` | matches indexer batch size |

## 6. Data

Shapes are defined in [[Data Model]]. Estimated on-disk size at 10M documents + 50M comments:
**~180–260 GB** depending on whether `translit_body` is stored (open question in [[Data Model]] §9).
RAM benefits from the OS page cache — provision RAM ≥ 25 % of index size.

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Service down | `/health`, `readyz` | 503 `search_unavailable`; ingestion buffers in [[Task Queue]] |
| Indexing task queue backed up | `GET /tasks` pending count | [[Indexer Worker]] slows batch submission |
| Disk full | disk metric, task failures | `DiskPressure` page; prune raw blobs; expand volume |
| Settings drift (manual edit) | settings hash check at boot | `WARN` + re-apply from git |
| Task failure on one batch | task status `failed` | split batch, isolate the bad doc → DLQ |
| Index corruption | startup failure | restore latest snapshot; re-run ingestion since snapshot |
| Version upgrade needs migration | version check | snapshot → dumpless upgrade → verify counts |

## 8. Performance

| Operation | Budget |
|:---|:---|
| Simple search, 10M docs | ≤ 30 ms p95 |
| Multi-search (3 legs) | ≤ 50 ms p95 |
| Facet computation | ≤ 20 ms p95 |
| Indexing throughput | ≥ 2 000 docs/s in batches of 5 000 |
| Cold start after restart | ≤ 60 s to `/health` green |

Indexing and search compete for CPU. `MEILI_MAX_INDEXING_THREADS` is deliberately capped so a large
crawl burst cannot degrade the [[Performance Budgets]] search SLO.

## 9. Observability

Scrape Meilisearch's own `/metrics` (enable `MEILI_EXPERIMENTAL_ENABLE_METRICS`). Track: index doc
count by index, pending tasks, task failure rate, DB size on disk, search request duration.
Dashboard: **Index Health** ([[Observability]] §5).

## 10. Security

Never exposed outside the `core` Docker network ([[Deployment Topology]] §3 / [[Security and Privacy]] T5).
No public port mapping — a CI check greps `docker-compose.yml` for a `ports:` entry on this service
and fails the build. Encryption at rest is provided by full-disk encryption on the host, not by
Meilisearch.

## 11. Testing

- Settings are applied by an idempotent migration job; a test asserts live settings == git settings.
- Integration: seed 10k fixture documents, run the golden query set, assert expected top hits.
- Tokenisation: an Arabic/Darija/French token table asserting index-time == query-time forms.
- Upgrade drill: snapshot → restore into a clean container → assert doc count and a sample query.
- Load: 500 rps search while indexing 2 000 docs/s; assert p95 stays within §8.

## 12. Open Questions

- [ ] Store `translit_body` (index +35 %) or transliterate at query time only?
- [ ] Is one Meilisearch node enough at 10M docs, or do we need a read replica before beta?
- [ ] Should `sources` live in Meilisearch at all, or just in Redis + git?

## Related

[[Data Model]] · [[Ranking and Relevance]] · [[Indexer Worker]] · [[Query Pipeline]] ·
[[Vector Index]] · [[Deployment Topology]] · [[ADR-0002 - Meilisearch as System of Record]]
