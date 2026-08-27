---
tags:
  - component
  - serving
  - ingestion
component-id: C33
binary: xustive-api
status: built
updated: 2026-08-27
---

# Knowledge Store

> **ID** C33 · **Crate** `xustive-knowledge` (library) · **Runs in** `xustive-api` (resolver and
> panel), `xustive-toold` (harvester), the Next.js web tier (live fallback) · **Upstream** Wikidata,
> Wikipedia, Wikimedia Commons · **Downstream** the results page rail
> ([[UI - Results Page]])

## 1. Purpose

Search returns documents. This returns *the thing itself* — who a person is, what a film scored,
where a place is — so the rail beside the results answers the question rather than pointing at
somewhere it might be answered. It is the entity panel of [[Milestone 8 - The Answer Layer]] and
the store that [[ADR-0019 - The Knowledge Layer]] argued for.

Why a **store** rather than a fetch: the serving plane has no route to the internet
([[ADR-0001 - Two-Plane Architecture]]), and a cache keyed by a query is a query log with extra
steps ([[ADR-0008 - No Query Logging]]). Both constraints dissolve when the unit of caching is the
**entity**: `Q42` is enumerable, identical for everyone who asks, and says nothing about who asked.
So the ingestion plane harvests entities on a schedule and the serving plane reads what is there.
The consequence, stated as plainly as [[Tool Data Plane]] states its own: an entity nobody has
harvested has no panel from the store — and, since M8-T03, gets one from the live fallback (§4.4)
while the harvester catches up.

## 2. Responsibilities

**In scope**: the entity model; parsing Wikidata documents into it; the `knowledge` Meilisearch
index; resolving a query to one entity or declining; per-kind fact templates; the harvester; the
live fallback through the web tier; the relation row ("cast of X", "books by Y").

**Out of scope**: rendering labels (translations live with the other translations in
`web/lib/i18n`); instant answers ([[Instant Answers]]); the summariser ([[Summarizer]]).

## 3. Where it lives today

| Piece | Path |
|:---|:---|
| Entity model, facts with per-field provenance | `crates/xustive-knowledge/src/entity.rs` |
| Kind (closed enum) and the `P31` lookup table | `crates/xustive-knowledge/src/kind.rs` |
| Wikidata document → `Entity` (pure, tested on saved docs) | `crates/xustive-knowledge/src/wikidata.rs` |
| Index document shape, `INDEX = "knowledge"` | `crates/xustive-knowledge/src/index.rs` |
| Resolver (score tiers, kind preference, tie-breaks) | `crates/xustive-knowledge/src/resolve.rs` |
| Per-kind fact templates, exhaustive match | `crates/xustive-knowledge/src/template.rs` |
| Index settings | `crates/xustive-search/src/settings.rs` `knowledge_settings()` |
| Harvester (fetch, pace, demand seeds) | `crates/xustive-toold/src/knowledge.rs`, `main.rs` |
| Seed list | `data/knowledge/seeds.tsv` |
| Panel, render and resolve-live endpoints | `crates/xustive-api/src/knowledge.rs` |
| Model-assisted blurb and disambiguation | `crates/xustive-api/src/knowledge_model.rs` |
| Live fallback (Wikidata from the web tier) | `web/app/api/knowledge-live/route.ts` |
| Relation lists (SPARQL, claims fallback) | `web/app/api/knowledge-list/route.ts`, `web/lib/relations.ts` |
| Older Wikipedia-only panel route (kept, no longer mounted) | `web/app/api/knowledge/route.ts`, `KnowledgePanel.tsx` |
| Image proxy for Wikimedia hosts | `web/app/api/wiki-image/route.ts`, `web/lib/commons.ts` |
| Components | `web/components/search/EntityPanel.tsx`, `ListPanel.tsx` |

## 4. Interface

### 4.1 Serving plane (`xustive-api`)

```
GET  /api/v1/knowledge?q=&lang=&kind=film,series   → 200 panel | 204 no panel
POST /api/v1/knowledge/render                       → panel from a raw Wikidata document
POST /api/v1/knowledge/resolve-live                 → { id } chosen among web-tier candidates
```

All three sit behind the `KNOWLEDGE` rate limit (`ratelimit.rs`: 90 requests per 60 s per
client). `panel` answers **204** rather than an empty 200 because "there is no panel" is a
different thing from "here is an empty panel", for the client and every cache between.

`kind=` is a hint from the relation row: "the matrix" beside its cast must be the film, not the
album. It is a **filter** in the resolver (§5.3), not just a lift.

The panel body is `render()`'s JSON: names, description, the template-selected `facts[]` each
with `provenance { source, licence }`, `images[]`, an optional `extract` (Wikipedia paragraph with
its article URL), authorities (links by identifier — IMDb, Goodreads, Open Library and so on, never
fetched), an optional `also` (the runner-up, offered as "did you mean"), and — when
`ml.knowledge_assist` is on — a `blurb { text, generated: true }`. A machine-written sentence is
always labelled as one.

### 4.2 Web tier (Next.js route handlers, the one place with egress — ADR-0014)

```
GET /api/knowledge-live?q=&lang=&kind=     resolve on Wikidata, render via the API
GET /api/knowledge-live?id=Q…&lang=        a known entity by id (the relation row's picks)
GET /api/knowledge-list?q=&lang=           cards for a relation query (≤ 16 members)
GET /api/wiki-image?u=                     Wikimedia-only image proxy (5 MiB, 6 s, 5 redirects)
```

`EntityPanel` asks the store first (a millisecond, and the answer for anything harvested), then
`/api/knowledge-live` for what the store does not hold. Both are fetched after paint; the panel
shows a skeleton, then collapses to nothing or fills — the three states `Summary` established.

## 5. Internal Design

### 5.1 The entity

Facts carry **source and licence individually**, because an entity's parts genuinely come from
different places under different terms — claims are CC0, an encyclopedic extract is CC BY-SA, and
every image has its own author. Values stay **typed** (`Value::Date { at, precision }`, quantities,
entity references) rather than pre-formatted, because a date rendered at harvest time is a date
rendered in one language and the product answers in four. Labels are kept per language with
Wikidata's `mul` (multilingual) label in the fallback chain right after the reader's own, so a
proper name reads in its own script rather than in whichever of our four languages happened to
exist.

### 5.2 Kind and template

`Kind` is a closed enum — Person, Film, Series, Place, Organisation, Product, Book, Music, Event,
Species, Concept — decided by a lookup on Wikidata's `P31`, which is a fact rather than a
judgement. `template::template(kind)` is an exhaustive match with no wildcard arm, so a kind
without a template is a compile error, not a blank card. Templates carry machine keys
(`birth_date`, not "Born"); labels are translations.

### 5.3 Resolution

Pure and index-free: the API fetches ten candidates from Meilisearch (`CANDIDATES = 10`) plus,
per candidate, a count of corpus documents mentioning the name (`CORPUS_PROBE = 50`); the
resolver decides what they mean. Precision governs — a confident panel about the wrong thing is
worse than no panel.

1. **Gate.** `is_panel_shaped`: 2–60 chars, ≤ 8 words, no question marker in any of the four
   languages. A question belongs to the summariser.
2. **Filter.** Non-renderable (bare-label) entities are excluded before scoring, not penalised —
   a maximally prominent bare label once won on points. With a `kind` hint, candidates of another
   kind are removed outright: "cast of the matrix" once resolved to a boxer nicknamed The Matrix.
3. **Score tiers.** Exact normalised name 0.70 · every query word a whole word of a name 0.60
   (surname search: "mahrez") · prefix 0.35 · matched on description only 0.10. Then two
   tie-breakers capped low on purpose — prominence (sitelinks / 200, ≤ 0.15) and corpus mentions
   (/ 50, ≤ 0.15) — which decide *which* Oran, never *whether* this is Oran. A `kind` hint adds
   `KIND_PREFERENCE = 0.20`; a release inside the last two years adds `RECENCY = 0.05` ("dune"
   means the new film). Normalisation lower-cases, strips the Arabic definite article and
   punctuation.
4. **Decide.** Below `MIN_CONFIDENCE = 0.55`, nothing. A runner-up within `AMBIGUOUS_WITHIN =
   0.15` is surfaced as `also` rather than swallowed. Ordering uses the unclamped score: clamping
   first flattened every strong candidate to 1.0 and let the id string decide (The Matrix
   Reloaded, Q189600, sorts before Q83495).

When the resolver leaves a near-tie and `ml.knowledge_assist` is on, the model may break it —
live, uncached, bounded at 8 s; if it declines, the deterministic leader ships. Disambiguation
**cannot** be cached against the entity: which entity a query means is a property of the query.
The blurb is, with a 30-day TTL in the tool cache.

### 5.4 Index

One Meilisearch index, `knowledge`, primary key `id`. Searchable: `names`, `descriptions` only —
an entity is never found by the contents of its own image credit. Filterable `kind`, sortable
`prominence` and `updated_at`; the whole entity rides along in the nested `entity` field so one
round trip returns a complete panel. Ranking rules put `exactness` before `typo` ("Oran" must not
become "Orano"), and a name needs 5 / 9 characters before one / two typos are tolerated.

### 5.5 Harvester (in `xustive-toold`)

Lives in `toold` because `toold` already *is* the sanctioned bridge — on `ingest` for egress and
on `core` for storage, taking no user input. Every pass (`--tick`, default 300 s): read
`data/knowledge/seeds.tsv`; add **demand seeds** from the ephemeral signals store when
`XUSTIVE_SIGNALS_URL` is set (the API records a panel miss under the `entity` namespace through
the same k-anonymous mechanism weak coverage uses — `k_anonymity` default 20, window 30 days — so
an entity fewer than k people asked for is never written down); skip anything harvested within
`--knowledge-max-age` (7 days); fetch `wbgetentities` in batches of 50 with a 300 ms pace; resolve
referenced labels (directors, units); attach the `ar`/`fr`/`en` Wikipedia extracts; write
documents. Seeds may carry an expectation (`matches_expectation`) so a QID that drifted to the
wrong thing is dropped rather than indexed. A failed batch costs its own entities, not the pass;
a failed pass is a warning, never fatal. `MEILI_URL` empty leaves the harvest off and the weather
pass untouched.

### 5.6 Live fallback and lists (web tier)

`knowledge-live` fetches **names only** for up to 12 Wikidata search hits (a full document is
megabytes and a dozen timed out), drops disambiguation / given-name / family-name pages, looks up
each candidate's `P31` with one small `wbgetclaims` call (three in flight — Wikimedia resets
more; SPARQL took 6.8 s for a six-id `VALUES`), then hands the candidates to the API's
`resolve-live` so the **same resolver** decides. The first version ranked by sitelinks in
TypeScript and resolved "messi" on the French page to Jesus Christ (*Messie*). The winner's full
document goes to `/knowledge/render` — two rounds: the API returns `unresolved` label ids, the web
tier fetches those, renders again. No second, weaker parser in TypeScript.

`knowledge-list` resolves the subject the same way, then lists members with one SPARQL query
(6 s leash) and falls back to the subject's own claims — `P527` parts, `P179` series — when SPARQL
stalls. Cards link to authorities by identifier; there are no ratings, because Goodreads has had
no public API since 2020 and a single source's number next to another source's link invites the
wrong reading. Thumbnails are built with `commonsThumbUrl` (the MD5 layout Commons has always
used, widths 120/250/330/500) so the proxy makes one request instead of following three
redirects. All upstream calls share the [[Web Upstream Client]] pool.

`relations.ts` decides server-side, with regular expressions in four scripts, whether a query is
relation-shaped (`cast`, `books`, `films`, `albums`) and which kinds the subject may be; the
rail then shows the subject itself beside the row.

## 6. Configuration

| Where | Key | Default | Meaning |
|:---|:---|:---|:---|
| `config/*.toml` | `ml.knowledge_assist` | `false` | model blurb + disambiguation |
| `config/*.toml` | `discovery.weak_coverage_enabled` | `false` | also gates entity-demand recording |
| `xustive-toold` | `--meili` / `MEILI_URL`, `--meili-key` / `MEILI_KEY` | empty (off) | |
| `xustive-toold` | `--seeds` | `data/knowledge/seeds.tsv` | |
| `xustive-toold` | `--signals` / `XUSTIVE_SIGNALS_URL` | empty (off) | demand queue |
| `xustive-toold` | `--k-anonymity`, `--demand-window-days`, `--knowledge-max-age` | 20, 30, 7 d | |
| web tier | `XUSTIVE_API_URL` | `http://127.0.0.1:8080` | render / resolve-live self-call |

`deploy/docker-compose.yml` does **not** pass `MEILI_URL` to the `toold` service today, so in the
stock compose stack the harvest is off until the operator adds it.

## 7. Failure Modes

| Failure | Response |
|:---|:---|
| Knowledge index missing or unreachable | no candidates → 204; never a failed search |
| Nothing above the confidence floor | 204, miss recorded (if enabled), live fallback tries |
| Wikidata slow or refusing (live path) | 12 s route timeout, panel collapses to nothing |
| Model absent / slow (assist on) | fails open: no blurb, deterministic leader |
| Harvest batch fails | that batch's entities skipped; pass continues; warning |

The rail is additive. Nothing on the results page depends on it.

## 8. Security and privacy

The serving plane fetches nothing. The web tier's upstreams are fixed hosts with a fixed
User-Agent; the reader's IP never reaches Wikimedia because images go through `wiki-image`, whose
allow-list is `upload.wikimedia.org` and `commons.wikimedia.org` only, grown one named host at a
time. Nothing keyed by a query is ever stored; demand is recorded only above the k-anonymity
floor, in the ephemeral signals instance, under the same switch as weak coverage.

## 9. Testing

60 unit tests across the crate: parsing against saved Wikidata documents, resolver judgements
against fixed candidate sets (surname, kind filter, recency, tie surfacing), template
exhaustiveness, index round-trip. `knowledge_model` asserts the cache key is the entity, never
the query.

## 10. Open Questions

- [ ] Wire `MEILI_URL` / `XUSTIVE_SIGNALS_URL` into the compose `toold` service so the harvest and
      demand loop run out of the box.
- [ ] The retired Wikipedia-only route (`/api/knowledge`, `KnowledgePanel.tsx`) is dead code since
      the rail became one panel; remove or keep as a documented fallback.
- [ ] Live-fallback results are not written back to the store (by design — the serving plane does
      not write); the demand loop is what closes the gap, and only where signals are enabled.

## Related

[[ADR-0019 - The Knowledge Layer]] · [[ADR-0014 - Knowledge Panel from Wikipedia via the Web Tier]] ·
[[Milestone 8 - The Answer Layer]] · [[Tool Data Plane]] · [[Search Index]] · [[Web Upstream Client]] ·
[[Instant Answers]] · [[Security and Privacy]]
