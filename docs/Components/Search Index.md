---
tags:
  - component
  - serving
  - storage
component-id: C06
binary: meilisearch (settings in xustive-search::settings; migration via xustive migrate)
status: built
updated: 2026-08-27
---

# Search Index

> **ID** C06 · **Service** `meilisearch` v1.13 · **Our side** `crates/xustive-search`
> (`settings.rs`, `client.rs`), `xustive migrate` / `xustive reindex` ·
> **Upstream** [[Indexer Worker]] · **Downstream** [[Query Pipeline]], [[Autocomplete Service]],
> [[Knowledge Store]], the admin console

## 1. Purpose

The retrieval engine and, per [[ADR-0002 - Meilisearch as System of Record]], the system of
record. Sub-50 ms full-text search with typo tolerance, native Arabic tokenisation, faceting and
custom ranking rules — without us writing an inverted index.

## 2. Responsibilities

**In scope**: storing `Document`, `Comment`, `Source` and knowledge-entity records; tokenising
Arabic/Latin text; matching, typo tolerance, phrase search; facet counts; ranking stage 1;
highlighting; applying the curated synonym classes.

**Out of scope**: re-ranking with business signals (→ [[Query Pipeline]] §4.6); vector search
(→ [[Vector Index]]); durability beyond snapshots (→ [[Deployment Topology]]).

## 3. Interface

All access goes through `MeiliClient` (`client.rs`), a thin `reqwest` wrapper with the timeout
and error-classification policy baked in (`SearchError::Timeout` is what the pipeline narrows on).

| Op | Endpoint | Caller |
|:---|:---|:---|
| Search | `POST /indexes/{ix}/search` (one leg per call; no `/multi-search`) | [[Query Pipeline]], [[Autocomplete Service]], [[Knowledge Store]] |
| Upsert / update | `POST` / `PUT /indexes/{ix}/documents` (batch) | [[Indexer Worker]], eager federation index, media repass |
| Delete | `DELETE /indexes/{ix}/documents/{id}` | takedowns, admin |
| Page documents | `GET /indexes/{ix}/documents` (optionally by fields) | admin console, repass jobs |
| Settings | `PATCH /indexes/{ix}/settings` + `GET` for drift | `xustive migrate` |
| Keys | `POST /keys`, `GET /keys` | `xustive migrate` (`SEARCH_KEY`, `INDEX_KEY`) |
| Swap | `POST /swap-indexes` | `xustive reindex` |
| Task poll | `GET /tasks/{uid}`, `wait_task` | everything that writes |

Scoped keys: `SEARCH_KEY` carries `search` alone — not `indexes.get`, not `stats.get`; the API
never needs them. `INDEX_KEY` has no `indexes.delete`: a worker never drops an index. Dev runs
with an empty master key.

## 4. Internal Design (our configuration of it)

### 4.1 Indexes

| Index | Primary key | Notes |
|:---|:---|:---|
| `documents` | `id` | the corpus |
| `comments` | `id` | [[ADR-0003 - Comments in a Separate Index]]; declared and migrated, **empty** — nothing produces comments (2026-08-27) |
| `sources` | `id` | the registry, searchable by `display_name`/`id`/`notes`, filterable by kind, tier, approval, legal basis |
| knowledge (`xustive_knowledge::index::INDEX`) | `F_ID` | entities by name ([[Knowledge Store]]) |

The alias/versioned-index indirection exists in the client (`versions_of`, `resolve`,
`swap_indexes`): `xustive reindex` builds `documents_vN`, swaps, and `reindex --rollback` swaps
back. For now the alias and the concrete index usually share a name.

### 4.2 `documents` settings (`documents_settings()`)

```jsonc
{
  "searchableAttributes": ["title","excerpt","entities","body","media.ocr_text","translit_body",
                           "author.name"],
  "filterableAttributes": ["source_type","source_id","domain","language","script",
      "sentiment.label","published_at","crawled_at","is_nsfw","published_at_precision",
      "quality_score","spam_score","geo.wilaya","topics","robots_indexable","discovery",
      "enrichment_level","content_type","id","media.type","media.provider"],
  "sortableAttributes": ["published_at","crawled_at","quality_score","engagement.likes",
                         "hits.opens","hits.reports","endorsement"],
  "displayedAttributes": ["id","title","url","canonical_url","excerpt","source_type","source_id",
      "domain","author","published_at","published_at_precision","sentiment","engagement",
      "language","media","simhash","quality_score","comments_count","discovery","entities",
      "topics","body_len"],
  "proximityPrecision": "byAttribute",   // PROB-004: byWord's word-pair database dominated indexing
  "rankingRules": ["words","typo","proximity","attribute","sort","exactness",
                   "endorsement:desc","quality_score:desc","published_at:desc"],
  "typoTolerance": { "enabled": true,
    "minWordSizeForTypos": { "oneTypo": 4, "twoTypos": 9 },
    "disableOnAttributes": ["entities"],
    "disableOnWords": ["وهران","قسنطينة","عنابة","سطيف","تلمسان","بجاية","سونلغاز","سيال",
      "موبيليس","جيزي","أوريدو","Oran","Setif","Annaba","Bejaia","Tlemcen","Sonelgaz","Seaal",
      "CNAS","ANEM","Mobilis","Djezzy","Ooredoo"] },
  "faceting": { "maxValuesPerFacet": 100, "sortFacetValuesBy": { "*": "count" } },
  "pagination": { "maxTotalHits": 2000 },
  "separatorTokens": ["|","·","—","–"],  "nonSeparatorTokens": ["@","#","_"],
  "dictionary": ["سونلغاز","الجزائر","Sonelgaz","CNAS","ANEM","Seaal","Ooredoo","Djezzy",
                 "Mobilis","Naftal","Sonatrach"],
  "stopWords": STOP_WORDS,            // ar + fr + en function words, one shared list
  "synonyms": Expander::meili_synonyms()
}
```

Why each of the non-obvious ones:

- The **order** of `searchableAttributes` is load-bearing: the `attribute` rule reads it, so a
  title match outranks a body match. `media.ocr_text` (text OCR'd from a page's images) sits
  below `body` — real content, noisier than prose the page wrote itself. `translit_body` is
  declared but never populated ([[Query Expander]] §4.2).
- `oneTypo: 4`: Arabic roots are short; the default of 5 let `وهران` match `إيران`.
- `published_at_precision` is filterable because the News vertical excludes *guessed* dates.
- `id` is filterable so image-similarity and dense-fusion results resolve with one `id IN […]`.
- `media.type`/`media.provider`: Meilisearch flattens arrays of objects, so a filter on them
  selects any document carrying at least one such entry — the Images and Videos tabs cost a
  settings change, not a reindex ([[Media Extraction]]).
- `discovery` and `enrichment_level` let the admin console facet by provenance and the repass
  job find documents enriched under load.
- `STOP_WORDS` and `MAX_TOTAL_HITS` are `pub` constants shared with the [[Query Pipeline]]: the
  stop-word rescue can only recognise a query the engine will strip if it sees the *same* list,
  and the pages the API advertises must never exceed the pages the engine can serve (BUG-002).
- The two custom ranking rules are tie-breakers *after* textual relevance, never before it.
- `synonyms` come from the same lexicon as the query-time expander, emitted both directions
  because Meilisearch synonyms are directional ([[Query Expander]] §4.1).

### 4.3 `comments` settings

Searchable `body`, `author.name`; filterable `document_id`, `source_type`, `sentiment.label`,
`published_at`, `language`; sortable `published_at`, `likes`; `maxTotalHits` 1000. No custom
freshness rule: comment recency was to be inherited from the parent at re-rank time, and ranking
comments by their own date here would double-count it.

### 4.4 Knowledge settings

Only names and descriptions searchable; `exactness` ahead of `typo` (for a name an exact match
is almost always right — typo tolerance that outranks it turns `Oran` into `Orano`); `oneTypo` 5;
no stop words; `prominence` sortable as the tie-breaker between entities sharing a name.

### 4.5 Arabic tokenisation

Meilisearch's `charabia` handles Arabic segmentation and normalisation on both the index and
query side. Our own `xustive-text::normalize` is applied to the *query* before it is sent, and
inside the detector, expander and scorer; the parser stores the body as extracted. The symmetry
guarantee we test is that `normalize` is idempotent and its fast and slow paths agree
(`xustive-text/tests/symmetry.rs`), not that we pre-normalise the index.

## 5. Configuration (service level)

`deploy/docker-compose.yml`, dev values:

| Env | Value | Notes |
|:---|:---|:---|
| image | `getmeili/meilisearch:v1.13` | |
| `MEILI_ENV` | `development` | |
| `MEILI_MASTER_KEY` | `${MEILI_MASTER_KEY:-}` | empty by default so dev needs no setup |
| `MEILI_NO_ANALYTICS` | `true` | |
| `MEILI_MAX_INDEXING_MEMORY` | `3Gb` | sized for the dev box, not the 16 Gb of the original plan |
| `MEILI_MAX_INDEXING_THREADS` | 6 | so indexing cannot starve search |
| `MEILI_EXPERIMENTAL_ENABLE_METRICS` | `true` | Prometheus scrape |

Our side: `search.meili_url`, `search.meili_key`, `search.documents_index`,
`search.comments_index`, `search.timeout_ms` (1200 dev; connect timeout 3 s) in `config/*.toml`.
No `ports:` mapping on meilisearch in production.

## 6. Data

Shapes are defined in [[Data Model]]. The on-disk size estimates of the original design
(10 M documents, 50 M comments) have not been measured against a real corpus.

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Service down | `health()`, `/readyz` | search errors surface as 503-class; ingestion buffers in [[Task Queue]] |
| Slow engine (indexing backlog) | `SearchError::Timeout` | the pipeline narrows to a page's worth (BUG-041) |
| Task failure on a batch | `wait_task` status | [[Indexer Worker]] retries / DLQ |
| Settings drift | `xustive migrate` diffs live vs code per index | prints the differing keys and re-applies |
| Bad reindex | `xustive reindex --rollback` | swap back to the previous contents |
| Index corruption / disk full | not automated | restore snapshot; [[Deployment Topology]] |

## 8. Performance

Not asserted by a test. Under a crawl backlog the 200-candidate query with facets and
highlighting was measured at several hundred milliseconds while a bare page answered in ~30 ms —
the reason narrowing exists.

## 9. Observability

Meilisearch's own `/metrics` (enabled above) plus `xustive stats` (document counts per alias,
resolving the alias first). Dashboards in `deploy/grafana`.

## 10. Security

Never exposed outside the Docker network in production; the API holds a search-only key, the
worker an index-only key, and only `migrate` holds the master key. Filter expressions are built
from typed values ([[Query Pipeline]] §10).

## 11. Testing

- `settings.rs` unit: stop-word recognition, required filterable attributes present on every
  index.
- `xustive migrate` is idempotent and reports drift; `all()` lists every index in creation order.
- Integration and upgrade drills from the original plan are manual.

## 12. Open Questions

- [ ] Populate `translit_body` or remove it from `searchableAttributes`.
- [ ] `comments` is migrated but never written; keep the index for the connectors, or drop it?
- [ ] One node at 10 M docs, or a read replica before beta?

## Related

[[Data Model]] · [[Ranking and Relevance]] · [[Indexer Worker]] · [[Query Pipeline]] ·
[[Query Expander]] · [[Knowledge Store]] · [[Vector Index]] · [[Deployment Topology]] ·
[[ADR-0002 - Meilisearch as System of Record]] · [[ADR-0003 - Comments in a Separate Index]]
