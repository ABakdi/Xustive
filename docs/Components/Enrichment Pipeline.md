---
tags:
  - component
  - ingestion
component-id: C17
binary: xustive-cli (crawld / crawl)
status: built
updated: 2026-08-27
---

# Enrichment Pipeline

> **ID** C17 · **Runs in** the parse path of `crawld` (`crates/xustive-ingest/src/enrichment.rs`,
> then the orchestrator's media hooks) · **Upstream** [[Content Parser]] · **Downstream**
> [[Deduplication Service]] → `q:index` → [[Indexer Worker]]

## 1. Purpose

Add everything the raw document lacks but search needs: a wilaya, topic labels, a spam score, a
quality score, and — when switched on — the text inside its images and their embeddings. It is
the last stage where a document can be improved, and the one with the widest cost variance: the
four text steps cost microseconds, a page with three images costs seconds of fetch and OCR.

## 2. What exists today

There is no `q:enrich` stream and no enrichment worker. Enrichment runs **inline in the parser**
(`Parser::parse` calls `Pipeline::standard().run(doc, Full)`), and the media passes run right after
parse in `Orchestrator::step`. A separate stage bought nothing while the crawl is one process; the
queue exists only between crawl and index ([[Task Queue]]).

| Step | Where | Required | Built? |
|:---|:---|:---|:---|
| Gazetteer (wilaya hint) | `gazetteer.rs` | no | yes |
| Topic labels | `topics.rs` | no | yes |
| Spam score | `spam.rs` | yes | yes |
| Quality score | `parse.rs::quality_score` | yes | yes |
| Image OCR + body backfill | `media_ocr.rs` | opt-in `[media] image_ocr_enabled` | yes |
| Image CLIP embed + pHash | `media_embed.rs` | opt-in `[vector] enabled` | yes |
| Text embedding | `text_embed.rs` | opt-in `[vector] text_enabled` | yes |
| Sentiment | `xustive-lang::Scorer` | — | only in the one-shot `xustive-cli crawl`; **not in `crawld`** (2026-08-27) |
| Language re-confirm on the full body | — | — | not built; the parser's detection is what is stored |
| Comment sentiment, author frequency | — | — | not built (no comments are ingested) |
| Partial-under-load + repass | `Pipeline::run(_, Partial)` | — | **executor built, never invoked** (2026-08-27) |

## 3. Interface

```rust
// crates/xustive-ingest/src/enrichment.rs
pub trait EnrichmentStep: Send + Sync {
    fn name(&self) -> &'static str;
    fn required(&self) -> bool;     // false ⇒ skipped at EnrichmentLevel::Partial
    fn apply(&self, doc: &mut Document);
}
pub struct Pipeline { steps: Vec<Box<dyn EnrichmentStep>> }
impl Pipeline {
    pub fn standard() -> Self;      // gazetteer, topics, spam, quality — in that order
    pub fn run(&self, doc: &mut Document, level: EnrichmentLevel) -> Ran; // stamps doc.enrichment_level
}
```

Every step has the same shape even though they touch different fields; the one step that needed
extra context (quality reads the extraction method) gets it back off `doc.access_path`, which the
parser set first. That uniformity is what makes "run only the required ones" a one-line decision.
`enrichment_level` is a filterable attribute in the index, so a future repass can find `partial`
documents with a filter.

## 4. Internal Design

### 4.1 Text steps

Required steps run last so an optional step's output could feed a required one without reordering.

- **Gazetteer.** Fold the text, count whole-token mentions of the 58 wilaya names (mirrored from
  `xustive-tools`' table, so the two cannot drift), hint the most-mentioned one into
  `geo.wilaya` / `geo.wilaya_name`. A lookup, not a model: it never invents a place.
- **Topics.** Keyword classifier over a small label set (politics, economy, sport, …). A document
  can carry several labels or none — a wrong label is worse than no label.
- **Spam** (`spam_score`, 0–1). The stronger of two signals: distinct spam phrases from
  `data/spam/phrases.txt` (Arabic/French/English, betting, loans, pharma, crypto, adult, SEO
  filler — counted once each, so one phrase repeated is one signal) and keyword stuffing (share of
  the body taken by its most common content word, saturating at 18 %). Search suppresses at
  **≥ 0.8**; the document stays in the index. Phone density, link density, emoji ratio and author
  frequency from the original design are not implemented.
- **Quality** (`quality_score`, 0–1), additive and clamped: body length up to 3 000 chars (0.25);
  date precision second/day/month (0.20/0.18/0.10); extraction method JSON-LD/OpenGraph/density/
  fallback (0.20/0.12/0.10/0.02); a title over 15 chars (0.10); an author (0.08); any media (0.05);
  a detected language (0.12). Feeds the ranker's quality weight ([[Ranking and Relevance]]).

### 4.2 Media passes (opt-in)

Both use one `ImageFetcher` (own client, `XustiveBot/1.0 (… media OCR)` UA, 15 s timeout,
`SafeUrl` — private addresses refused, `image/*` only, capped by declared and actual bytes) and both
obey `[media] max_images_per_doc` and `max_image_bytes`. Details in [[Image Pipeline]].

1. **OCR** (`media_ocr::enrich`): images without `ocr_text`, up to the cap; tesseract on the
   blocking pool; usable text lands in `media[].ocr_text` / `ocr_lang`. When the page's own body
   is under 20 words the OCR text **backfills** it (`body_source = Ocr` if it was empty, appended
   otherwise) — the image *was* the content, which is the Facebook-screenshot case this exists for.
2. **Embed** (`media_embed::embed_and_store`): fetch, dHash into `media[].phash` if not already
   stamped, reuse a cached vector for a known hash or call the CLIP sidecar, upsert to Qdrant.
3. **Text embed** (`text_embed::embed_and_store`): `title\nbody` truncated to 4 000 chars, one
   vector per document keyed on the document id, after the id is final (a federated URL takes the
   id of its eager placeholder so it overwrites it).

**Rule: a document is never blocked from the index by an optional enrichment.** Every media
failure is swallowed — fetch, decode, OCR, sidecar, Qdrant — and the document is queued for text
regardless.

### 4.3 Under load

The executor can run required-only and stamp `Partial`, and `Full` explicitly clears the marker so
a repass finishes the job. Nothing calls it: the parser has already paid for the DOM, and the four
text steps are too cheap to be worth skipping. Load is handled upstream instead — the crawler
pauses when the indexer backlog passes 5 000 or Redis passes 85 % of `maxmemory` ([[Task Queue]]).
The repass job for partial documents was never needed and is **not built**. The only repass that
exists is `xustive-cli media-repass`, which re-parses stored raw bodies for media references
([[Media Extraction]] §5.5) — it does not re-run OCR or embeds.

### 4.4 Concurrency

None of its own. Each `crawld` worker runs its parse and its media passes sequentially inside its
own step; parallelism is the worker count ([[Crawler Orchestrator]]). OCR goes to
`spawn_blocking` so it never holds an async worker.

## 5. Configuration

| Key (`config/*.toml`) | Dev default | Meaning |
|:---|:---|:---|
| `[media] image_ocr_enabled` | `false` | index-side OCR pass |
| `[media] tessdata_dir`, `ocr_langs` | `data/tessdata`, `ara+fra+eng` | tesseract data |
| `[media] max_images_per_doc` | 3 | cap for OCR and embed passes |
| `[media] max_image_bytes` | 5 MiB | per image, declared and read |
| `[vector] enabled` | `false` | image embed pass |
| `[vector] text_enabled` | `false` | text embed pass |
| `[vector] embed_cache_ttl_days` | 30 | pHash → vector reuse; 0 disables |

Spam threshold (0.8), stuffing saturation (0.18), the thin-body word count (20) and the text-embed
character cap (4 000) are constants in code.

## 6. Data

Mutates the `Document` in flight: `geo`, `topics`, `spam_score`, `quality_score`,
`enrichment_level`, `media[].ocr_text/ocr_lang/phash`, `body`/`body_source`. Writes points to
[[Vector Index]] directly from the crawler — the indexer never sees vectors. Reads
`data/spam/phrases.txt` and the wilaya/topic tables compiled in.

## 7. Failure Modes

| Failure | Response |
|:---|:---|
| Image fetch fails / not an image / too large / private host | that image skipped; document intact |
| Tesseract init or read error | image skipped; the error is not logged with image content |
| CLIP or text-embed sidecar down, Qdrant down | vector skipped with a `debug`/`warn`; document indexed for text |
| Embed cache Redis down | cache absent; every image embedded |
| `[media]` on but no traineddata | every OCR fails quietly — check the sidecar/tessdata before blaming the corpus |

## 8. Observability

Per-step `Ran { applied, skipped }` exists for logging; there are no `xustive_enrich_*` Prometheus
metrics yet. `[[Crawler Console]]` counts `images` and `videos` seen per page, which is the
cheapest signal that media extraction upstream is alive.

## 9. Security

Media URLs are attacker-controlled (they come from crawled content), so every fetch goes through
`SafeUrl` and the pixel budget in [[Image Pipeline]] ([[Security and Privacy]]). Enrichment never
executes content. The spam phrase list is a git-reviewed data file.

## 10. Testing

`enrichment.rs` unit-tests full vs partial runs and that a `Full` repass clears the marker;
`spam.rs`, `gazetteer.rs`, `topics.rs` and `quality_score` have their own tables;
`tests/ssrf.rs` covers the fetcher; `tests/embed_cache_redis.rs` the cache. No labelled
spam/ham evaluation set exists yet.

## 11. Open Questions

- [ ] Sentiment in `crawld`: `Scorer` is wired only into the one-shot `crawl` command. Add it as a
      step, or drop the field until [[Sentiment Engine]] has a use in the UI?
- [ ] Should quality/spam be learned rather than hand-weighted? Rules are explainable and tunable
      without labelled data.
- [ ] Is `Partial` worth keeping if nothing triggers it? It is cheap insurance for a future
      expensive step (a transformer sentiment pass would be one).

## Related

[[Content Parser]] · [[Image Pipeline]] · [[Deduplication Service]] · [[Indexer Worker]] ·
[[Vector Index]] · [[Media Extraction]] · [[Data Model]] · [[Ranking and Relevance]] ·
[[Error Handling and Resilience]]
