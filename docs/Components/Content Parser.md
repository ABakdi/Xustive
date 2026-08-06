---
tags:
  - component
  - ingestion
component-id: C16
binary: xustive-worker
status: specified
updated: 2026-08-06
---

# Content Parser

> **ID** C16 · **Binary** `xustive-worker` · **Upstream** `q:parse` · **Downstream** [[Deduplication Service]] → `q:enrich`

## 1. Purpose

Convert heterogeneous raw input — HTML, JSON API payloads, XML feeds — into the single canonical
`Document` shape ([[Data Model]]). Everything downstream depends on this normalisation; a parser bug
is indistinguishable from missing content.

## 2. Responsibilities

**In scope**: content extraction from HTML; metadata extraction (title, dates, author, canonical
URL); social payload mapping; text normalisation; outlink extraction; excerpt generation; language
detection; media URL extraction; boilerplate removal.

**Out of scope**: fetching (→ [[Web Fetcher]]); enrichment (→ [[Enrichment Pipeline]]); deduplication
decisions (→ [[Deduplication Service]], though the parser computes the hashes).

## 3. Interface

Consumes `q:parse`. Produces a `Document` (pre-enrichment) onto `q:enrich`, plus `Comment[]`.
Also returns `outlinks: Vec<Url>` to [[Crawler Orchestrator]].

```rust
pub trait Parser: Send + Sync {
    fn parse(&self, raw: &RawFetch) -> Result<ParseOutput, ParseError>;
}
pub struct ParseOutput { pub document: Document, pub comments: Vec<Comment>,
                         pub outlinks: Vec<Url>, pub flags: ParseFlags }
```

`ParseFlags` carries `needs_render`, `boilerplate_ratio`, `charset_guessed`, `extraction_method`.

## 4. Internal Design

### 4.1 HTML extraction cascade

Tried in order; first success wins, and the method is recorded in `extraction_method` for debugging.

| # | Method | When |
|:---|:---|:---|
| 1 | **JSON-LD** (`schema.org/NewsArticle`, `Article`, `BlogPosting`) | present and parseable — best dates and authors by far |
| 2 | **OpenGraph / Twitter cards** | `og:title`, `og:description`, `og:image`, `article:published_time` |
| 3 | **Readability-style extraction** | density-based main-content detection over the DOM |
| 4 | **Per-domain selector rules** | `data/parsers/{domain}.toml` — hand-written CSS selectors for high-value Algerian sites |
| 5 | **Fallback** | `<title>` + all `<p>` text |

Per-domain rules matter more here than on the general web: many Algerian news sites use custom
templates that Readability handles poorly. The rules file is data, not code, so adding a site is a
PR anyone can review:

```toml
# data/parsers/elkhabar.com.toml
title    = "h1.article-title"
body     = "div.article-content"
date     = { selector = "time.published", attr = "datetime", format = "iso8601" }
author   = ".author-name"
exclude  = [".related-posts", ".ads", ".comments-widget"]
```

### 4.2 Boilerplate removal

Strip `nav`, `header`, `footer`, `aside`, `script`, `style`, `noscript`, `form`, elements matching
`exclude` rules, and elements whose link-density > 0.5. Compute `boilerplate_ratio =
1 − (kept_text / total_text)`; if `> 0.9`, flag the extraction as suspect and set `quality_score`
low rather than dropping — a low-quality document is still better than a hole in the index.

### 4.3 Date extraction (the hard part)

Ordered attempts:

1. JSON-LD `datePublished`
2. `<meta property="article:published_time">`
3. `<time datetime=…>`
4. Per-domain rule
5. URL pattern (`/2026/08/04/`)
6. Text patterns, **including Arabic and French formats**: `4 أوت 2026`, `04/08/2026`,
   `4 août 2026`, relative forms (`قبل ساعتين`, `il y a 2 heures`)

Rules:
- Ambiguous `DD/MM` vs `MM/DD` resolves to **DD/MM** (Algerian convention) unless the day > 12 proves
  otherwise.
- Timezone assumed `Africa/Algiers` (UTC+1, no DST) when absent.
- Future dates > 24 h ahead are rejected as parse errors.
- Dates before 1995 are rejected.
- On total failure: `published_at = crawled_at`, `published_at_precision = "unknown"` — and ranking
  discounts it ([[Ranking and Relevance]] §3). **Never silently pretend a crawl date is a publish
  date.**

### 4.4 Text normalisation

The shared `xustive-text` function, identical to the one [[Query Pipeline]] applies to queries:
NFKC → strip tatweel → strip harakat → fold Arabic-Indic digits → collapse whitespace → normalise
alef/ya/ta-marbuta variants into the secondary form. **Query-time and index-time must call the same
function**; a test asserts byte-identical output for a shared fixture set.

### 4.5 Derived fields

| Field | Method |
|:---|:---|
| `excerpt` | first 320 chars of `body` at a sentence boundary; from `og:description` if it is better |
| `content_hash` | BLAKE3 of normalised `body` |
| `simhash` | 64-bit SimHash over 3-token shingles ([[Deduplication Service]]) |
| `language` | [[Language Detector]] over `title + first 2 KB of body` |
| `entities` | capitalised-sequence + gazetteer match (wilayas, institutions) — cheap, no model |
| `canonical_url` | `<link rel="canonical">` if same-registrable-domain, else the fetched URL |
| `media[]` | `og:image`, `<img>` with `width ≥ 200`, JSON-LD `image` — capped at 4 |
| `body_len`, `robots_indexable` (`<meta name="robots">`) | direct |

### 4.6 Social payload mapping

Dispatch on `kind` to the mapping tables in [[Social Connector - Facebook]] §4.3,
[[Social Connector - Instagram]] §4.2, [[Social Connector - TikTok]] §4.2. The social path skips
HTML extraction entirely but shares §4.4 and §4.5.

## 5. Configuration

| Key | Default |
|:---|:---|
| `excerpt_chars` | 320 |
| `max_body_bytes` | 200 KiB |
| `max_media_per_doc` | 4 |
| `max_outlinks` | 200 |
| `min_body_chars` | 120 (below → drop as content-less) |
| `link_density_threshold` | 0.5 |
| `boilerplate_suspect_ratio` | 0.9 |
| `default_timezone` | `Africa/Algiers` |
| `rules_dir` | `data/parsers/` (hot-reload) |

## 6. Data

Reads `raw:{trace_id}` blobs and rules files. Writes `Document` + `Comment[]` to `q:enrich`. Pure
apart from the raw read — the same input always produces the same output, which is what makes replay
safe ([[Error Handling and Resilience]] §7).

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Malformed HTML | parser tolerance | `html5ever` recovers; never fatal |
| Empty extraction | `body_len < min_body_chars` | drop with `Permanent` class; count metric |
| SPA shell only | text < 512 B + framework markers | set `needs_render`, requeue for headless once |
| Date unparseable | all methods fail | `precision = "unknown"` |
| Charset mojibake | replacement-char ratio > 5 % | re-decode with the alternative charset; else drop |
| Rules file selector no longer matches | extraction falls to method 5 | metric `parser_rule_miss_total{domain}` → alert |
| Panic on adversarial DOM | `catch_unwind` at the task boundary | DLQ + fixture ([[Error Handling and Resilience]] §5) |
| Document exceeds size cap | length check | truncate `body`, flag `truncated` |

`parser_rule_miss_total` is the important one: sites redesign, selectors silently stop matching, and
extraction quality degrades without any error. A rising miss rate for a domain is a maintenance
ticket.

## 8. Performance

| Metric | Budget |
|:---|:---|
| HTML parse + extract (100 KB) | ≤ 25 ms p95 |
| Social JSON map | ≤ 3 ms p95 |
| SimHash + BLAKE3 | ≤ 5 ms |
| Throughput | ≥ 200 docs/s/worker |
| Memory | ≤ 300 MB |

## 9. Observability

`xustive_parse_duration_seconds{kind}`, `xustive_parse_method_total{method}`,
`xustive_parse_dropped_total{reason}`, `xustive_parse_rule_miss_total{domain}`,
`xustive_date_precision_total{precision}`, `xustive_boilerplate_ratio` (histogram),
`xustive_needs_render_total`.

## 10. Security

Input is hostile HTML. `html5ever` is memory-safe and does not execute scripts. Guards: bounded DOM
depth (200), bounded node count (200k), bounded output size, no XXE (external entities disabled in
the XML/feed path), no network fetches during parse (no resolving `<img>` or `<link>`). Extracted
strings are stored raw and escaped at render time ([[Security and Privacy]] T8).

## 11. Testing

- Corpus: 200 saved pages from real Algerian sites (news, forums, gov, blogs) with hand-labelled
  expected title/date/body. Target ≥ 90 % exact title, ≥ 85 % correct date, ≥ 0.9 body F1.
- Date suite: ~80 date strings across Arabic, French, English, relative, and ambiguous formats.
- Normalisation symmetry: `parse_normalize(x) == query_normalize(x)` for a shared fixture set — this
  test failing means search silently breaks.
- Adversarial: 10 MB of nested divs, billion-laughs XML, unclosed tags, mixed encodings, `<img>`
  bombs — all bounded, none panic.
- Per-domain rules: each rules file ships with a fixture page and an assertion, so a site redesign
  fails CI rather than silently degrading production.

## 12. Open Questions

- [ ] Who maintains per-domain rules as sites change? This is ongoing work, not a one-off.
- [ ] Should we run a real NER model for `entities` instead of the gazetteer? Better recall, costs
      ~40 ms/doc.
- [ ] Extract structured data (prices, job titles, phone numbers) for future vertical search?

## Related

[[Data Model]] · [[Web Fetcher]] · [[Deduplication Service]] · [[Enrichment Pipeline]] ·
[[Language Detector]] · [[Query Pipeline]] · [[Ranking and Relevance]]
