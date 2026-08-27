---
tags:
  - component
  - ingestion
component-id: C16
binary: xustive crawld (in-process), xustive crawl, xustive parse-check
status: built
updated: 2026-08-27
---

# Content Parser

> **ID** C16 · **Crate** `xustive-ingest` (`parse.rs`, `date.rs`, `rules.rs`) · **Runs in** the
> crawl daemon's orchestrator, in-process · **Upstream** [[Web Fetcher]] · **Downstream**
> [[Enrichment Pipeline]] (called from inside `parse`) → [[Deduplication Service]] →
> [[Indexer Worker]]

## 1. Purpose

Turn a fetched HTML page into the single canonical `Document` shape ([[Data Model]]). Everything
downstream depends on this normalisation; a parser bug is indistinguishable from missing content.

The original design had the parser as its own queue stage (`q:parse` → `q:enrich`). It never
became one: the parser is a struct the [[Crawler Orchestrator]] calls directly after a fetch, and
the only queue in the path is the index stream that feeds the [[Indexer Worker]]. One process
already holds the bytes; a hop through Redis would buy replayability at the price of a second copy
of every page.

## 2. Responsibilities

**In scope**: content extraction from HTML; metadata (title, date, author, canonical URL);
outlink extraction; excerpt; language detection; image and video pointers; cheap entity list;
content and SimHash hashes; running the enrichment pipeline; refusing pages that are `noindex`,
content-less, or hostile.

**Out of scope**: fetching (→ [[Web Fetcher]]); enrichment rules themselves
(→ [[Enrichment Pipeline]]); deduplication decisions (→ [[Deduplication Service]], though the
parser computes the hashes); social payloads (→ the connector notes; **not built** as of
2026-08-27, see §4.7).

## 3. Interface

```rust
// crates/xustive-ingest/src/parse.rs
pub struct Parser { /* Detector, ParseConfig, rules::Rules */ }
impl Parser {
    pub fn new(config: ParseConfig) -> Self;
    pub fn with_rules(self, rules: rules::Rules) -> Self;
    pub fn parse(&self, html: &str, url: &str, source_id: &str, source_type: SourceType)
        -> Result<Parsed, ParseError>;
}
pub struct Parsed { pub document: Document, pub outlinks: Vec<String>, pub method: Method }
pub enum Method { JsonLd, OpenGraph, Density, Fallback }      // as_str(): "json-ld" …
pub enum ParseError {
    TooLittleContent { chars, min, outlinks: Vec<String> },  // links survive the refusal
    NoIndex,
    TooComplex { what, found, limit },
}
```

Two things about that shape are deliberate. `TooLittleContent` **carries the outlinks**: a
category page or paginator is mostly links and little prose, so it fails the content check almost
by definition — and dropping its links means the crawler refuses to follow exactly the pages that
exist to be followed. And the winning `Method` is written to `document.access_path`, so a site
redesign shows up as a shift in the method distribution rather than as silently worse results.

There is no `ParseFlags`, no `needs_render`, no `Comment[]` output.

## 4. Internal Design

### 4.1 Complexity guard, before the DOM

`check_complexity` is one pass over the bytes, run before `scraper` builds a tree: `MAX_HTML_BYTES`
8 MiB, `MAX_TAGS` 100 000, `MAX_DEPTH` 512. Measured: 50 000 nested `<div>`s took 47 s to parse
and 20 000 unclosed tags 18 s. A page that shape is broken or hostile; either way one page must not
stall the crawler. Checked *before* the parse because the parse is the cost being guarded.

### 4.2 Extraction cascade (`extract_body`)

| # | Method | Wins when |
|:---|:---|:---|
| 1 | **JSON-LD** `articleBody` | present and ≥ `min_body_chars` — best dates and authors by far |
| 2 | **Density** (`densest_block`) | the block with the most text and the fewest links; link density > 0.5 rejects a block; hidden elements skipped |
| 3 | **OpenGraph** `og:description` | long enough to stand as a body |
| 4 | **Fallback** | every `<p>` |

Density comes *before* Open Graph because it beats per-site selectors on the long tail nobody has
written rules for, and an `og:description` is a summary, not the article. Title has its own
cascade: JSON-LD `headline` → `og:title` → `twitter:title` → `<h1>` → `<title>`, else the first 80
chars of the body.

Text is taken through `visible_text`, which walks the tree and skips `script`/`style`/hidden
nodes — `scraper`'s `.text()` returns *every* descendant text node, and a page whose body is 80 %
inline JavaScript is not about anything. Meta values go through `strip_tags`: publishers paste
article HTML into `og:description`, and a literal `<p>` in an excerpt reaches the results page.

### 4.3 Per-domain rules (`rules.rs`, `data/parsers/domains.toml`)

```toml
[[domain]]
host = "aps.dz"
date = "span.text-xs"
note = "date is a bare span with a Tailwind utility class; no JSON-LD, no <time>"
```

Rules are tried **before** generic extraction, never after: a publisher shipping correct metadata
does not need one, and one that is not is telling us its markup is unreliable. Twelve hosts today.
Subdomains match their parent (`www.aps.dz` → `aps.dz`); a duplicate host refuses the whole file
rather than letting file order decide; a missing file is not an error, a malformed one is logged
loudly and ignored entirely.

Only the `date` selector is consulted by `parse` today; `title` and `body` are parsed from the
file but not yet applied. Rules are loaded by `xustive crawl` and `xustive parse-check`; the
daemon's orchestrator builds `Parser::new(ParseConfig::default())` **without** rules as of
2026-08-27 — see §12.

`parse-check <url> [--date SEL]` fetches a real page and shows what each field came from, so a
rule is verified against the page before it is written, never after.

### 4.4 Date extraction (`date.rs`) — the hard part

Sources, in order: rule selector → JSON-LD `datePublished` → `article:published_time` →
`publish-date` → `itemprop=datePublished` → `<time datetime>` → `<time>` text → common containers
(`.date, .post-date, .article-date, .published, .entry-date, [class*=date]`) → a **prose scan**
of the first 400 chars of the body (4-token windows; a bare number is not a date).

The scan exists because 362 of 502 crawled documents had no date while the page plainly showed
`05 أوت 2026` — the extractor was only looking at markup those publishers do not emit.

`date::parse(text, now)` tries ISO 8601 → numeric → month names → relative, and knows:

- **Maghrebi month names** (`أوت`, `جويلية`, `جانفي`, `فيفري`, `ماي`, `جوان`) alongside the
  Levantine forms, plus French and English.
- **DD/MM**, never MM/DD; `04/08/2026` is 4 August.
- Relative forms: `قبل ساعتين`, `منذ يومين`, `il y a 3 jours`, `2 hours ago`.
- `Africa/Algiers` is UTC+1 all year when no offset is given.
- Rejects years outside 1995–2100 and anything more than a day in the future.
- Precision is recorded: `Second`, `Day`, `Month`, or `Unknown`.

On total failure `published_at = crawled_at` with `Unknown` precision, and ranking discounts it
([[Ranking and Relevance]]). **Never silently pretend a crawl date is a publish date.**

### 4.5 Derived fields

| Field | Method |
|:---|:---|
| `excerpt` | `og:description` or `description` if > 40 chars, else first `excerpt_chars` of body at a sentence boundary |
| `content_hash` | BLAKE3 of the body, `b3:` prefix (`xustive_core::hash`) |
| `simhash` | 64-bit SimHash, hex ([[Deduplication Service]]) |
| `language`, `language_confidence`, `script` | [[Language Detector]] over `title + first 2 000 chars` |
| `author.name` | JSON-LD author → `meta author` → `article:author` → `.author, .byline, [rel=author]` |
| `canonical_url` | `<link rel=canonical>` if present, else the fetched URL (no same-domain check) |
| `media[]` | `og:image` + `<img>` (image); `og:video`, `<video>`, embedded players as watch-page pointers (video, M9) — each capped at `max_media`, never bytes |
| `entities` | capitalised tokens from title + first 4 000 chars, 3–40 chars, up to 20. A gazetteer, not a model: cheap, explainable, enough for the `entities` field typo tolerance is disabled on |
| `body_len`, `access_path` (= method), `fetch_method = "static"`, `body_source = Native` | direct |

The body is stored **as extracted**, not normalised: `xustive-text::normalize` runs on queries
and inside the detector, and Meilisearch's tokeniser handles the index side ([[Search Index]] §4.4).

### 4.6 Enrichment, inside `parse`

The last thing `parse` does is `enrichment::Pipeline::standard().run(&mut doc, Full)`: wilaya
gazetteer, topics, spam, quality — see [[Enrichment Pipeline]]. The parser always runs the full
pipeline because it has already paid for the DOM; the *pressure* decision (`Partial`) belongs to
the daemon. `quality_score` reads the method: JSON-LD 0.20, OpenGraph 0.12, Density 0.10,
Fallback 0.02, plus length, known date, title, author, media, detected language.

### 4.7 Not built (2026-08-27)

Social payload mapping (JSON from the connectors), XML/feed input, a `needs_render` SPA path,
charset re-decoding, and a `boilerplate_ratio` measure. PDFs *are* handled, by the orchestrator
wrapping the extracted text in a minimal escaped HTML article so this one parser serves both.

## 5. Configuration

`ParseConfig` in code; nothing in `config/*.toml`.

| Field | Default |
|:---|:---|
| `excerpt_chars` | 320 |
| `max_body_bytes` | 200 KiB (truncated on a char boundary) |
| `max_media` | 4 (images and videos each) |
| `min_body_chars` | 120 (below → `TooLittleContent`) |
| `max_outlinks` | 200 (the frontier then keeps the best 64, `crawl.max_outlinks_per_page`) |

## 6. Data

Pure: the same input always produces the same output, except `crawled_at`/`indexed_at` (now).
Reads the rules file at startup. Writes nothing itself; the orchestrator publishes the `Document`.

## 7. Failure Modes

| Failure | Response |
|:---|:---|
| Malformed HTML | `scraper`/`html5ever` recover; never fatal (tests: unclosed tags, broken encodings) |
| Too little content | `Err(TooLittleContent { outlinks })` — not indexed, still crawled through |
| `<meta name=robots>` noindex | `Err(NoIndex)`; the orchestrator still follows its links |
| Hostile markup (size, tags, depth) | `Err(TooComplex)` before the DOM is built |
| Date unparseable | `Unknown` precision, never a guess |
| Empty title | first 80 chars of the body |
| Rules file missing / malformed | generic extraction only, logged (`info` / `error`) |

## 8. Performance

Adversarial fixtures (`tests/adversarial.rs`) assert a parse budget on nested divs, node bombs,
unclosed tags, entity bombs, huge attributes and text nodes, pathological whitespace. No p95
budget for ordinary pages is enforced by a test.

## 9. Observability

No parser-level metrics. `access_path` on every document is the extraction-method signal; the
admin document list shows `body_len` so an article and a navigation page can be told apart.
The `parser_rule_miss_total{domain}` alert from the original design is **not built**.

## 10. Security

Input is hostile HTML. `html5ever` is memory-safe and executes nothing. The complexity guard
bounds size, tag count and depth; there are no network fetches during parse; extracted strings
are stored raw and escaped at render time ([[Security and Privacy]]). PDF text is escaped before
it is wrapped as HTML.

## 11. Testing

- `tests/extraction_accuracy.rs`: gates of ≥ 90 % exact title, ≥ 85 % correct date, body F1.
- `tests/domain_rules.rs` with saved fixture pages: a rule without a fixture is a guess that rots.
- `date.rs` unit suite: Maghrebi months, DD/MM, relative forms, future rejection, tolerance.
- `tests/adversarial.rs`: bounded and panic-free on hostile input.
- Query/index normalisation symmetry lives in `xustive-text/tests/symmetry.rs`.

## 12. Open Questions

- [ ] Wire `Rules::load` into the daemon's orchestrator; today only `crawl` and `parse-check`
      see `domains.toml`, so the aps.dz date fix does not apply in production crawling.
- [ ] Apply the rules' `title`/`body` selectors, or drop the fields.
- [ ] A real NER model for `entities` instead of capitalised tokens? Better recall, ~40 ms/doc.
- [ ] Structured data (prices, phone numbers) for future vertical search?

## Related

[[Data Model]] · [[Web Fetcher]] · [[Crawler Orchestrator]] · [[Enrichment Pipeline]] ·
[[Deduplication Service]] · [[Language Detector]] · [[Query Pipeline]] · [[Ranking and Relevance]]
