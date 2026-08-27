---
tags:
  - component
  - ingestion
  - serving
component-id: C34
binary: xustive-cli
status: built
updated: 2026-08-27
---

# Media Extraction

> **ID** C34 · **Runs in** `xustive-cli` (`crawld` / `worker` parse path, `media-repass`) and
> `xustive-api` (verticals) · **Upstream** [[Content Parser]] · **Downstream** [[Search Index]],
> [[Thumbnail Proxy]], the Images and Videos tabs ([[UI - Search Verticals]])

## 1. Purpose

Find the images and videos a crawled page carries, store **references** to them on the document,
and let a reader browse the corpus by them. This is the ingestion half of
[[Milestone 9 - Images and Videos]]. It does not fetch image bytes for its own sake (OCR and
similarity embedding do, under their own switches — [[Image Pipeline]]) and it never fetches video
bytes at all.

The design rule that shapes everything else: a tile is **a page that has this image**. The title
and host are the page's; clicking goes to the page, because the page is what we indexed and what
a reader can judge. There is no separate image corpus.

## 2. Responsibilities

**In scope**: extracting `Media` entries at parse time; recognising video providers and turning
embeds into watch pages; the `media.type` / `media.provider` facets; the `images` and `videos`
verticals; the grid components; re-extracting media from stored raw bodies.

**Out of scope**: OCR, perceptual hashing, CLIP embeddings ([[Image Pipeline]], `xustive-media`
crate); proxying and signing thumbnails ([[Thumbnail Proxy]]); OCR-side image fetch limits
(`[media]` config belongs to [[Image Pipeline]]).

## 3. Where it lives today

| Piece | Path |
|:---|:---|
| `Media` / `MediaKind` on the document | `crates/xustive-core/src/model.rs` |
| `extract_media` (images, then videos) | `crates/xustive-ingest/src/parse.rs` |
| Video providers, watch-page rule, posters | `crates/xustive-ingest/src/video.rs` |
| Facets `media.type`, `media.provider` | `crates/xustive-search/src/settings.rs` |
| `Filters.media_kind` → `media.type = …` | `crates/xustive-search/src/filter.rs` |
| `?v=images` / `?v=videos`, `media[]` on the wire | `crates/xustive-api/src/search.rs` |
| Tabs and grid | `web/components/search/Verticals.tsx`, `MediaGrid.tsx` |
| Re-extraction from raw bodies | `crates/xustive-cli/src/main.rs` `media-repass` |
| Operator view of the image stack | `web/app/(operator)/admin/media/page.tsx`, `GET /api/v1/admin/media` |

## 4. Interface

On the wire, every result card carries `media[]` — `{ type, url, thumb_url?, provider? }` —
capped at `MEDIA_PER_CARD = 3` images per page (a photo gallery must not be the whole first
screen) and all of its videos (a page rarely has more than one).

```
GET /api/v1/search?q=…&v=images    → Filters.media_kind = "image"  → media.type = "image"
GET /api/v1/search?q=…&v=videos    → Filters.media_kind = "video"  → media.type = "video"
```

A vertical is a saved filter over the one index, not a separate corpus ([[Search Index]]). An
unknown `v` falls back to All. Federation, when on, maps the vertical to a SearXNG category
([[Federation Gateway]]).

## 5. Internal Design

### 5.1 Images

`extract_media` takes `og:image` first, then `article img, main img, .content img` up to
`ParseConfig.max_media` (4), skipping anything whose `width` attribute is under 200 (tracking
pixels and spacers) and de-duplicating by absolute URL. `width`/`height` are stored as 0 — the
parser does not fetch the image to learn them.

### 5.2 Videos — metadata only, ever

`<iframe>` sits on the parser's invisible-element list for *text* and stays there; `video.rs`
reads its `src`, which is a different thing. Sources, in order: `og:video`, `og:video:url`,
`og:video:secure_url`; `iframe[src]` and `embed[src]`; one `<video>` element per page (the page
*is* the watch page, so `self_hosted(page_url, poster)` — its `src` is a stream and is dropped).

Two rules, both pinned by tests:

- The stored `url` is the **watch page**, never a stream. A stream URL would invite something
  downstream to fetch it, and [[Milestone 2 - Ingestion at Scale|M2-T10.8]] requires that no code
  path downloads video bytes. The fetcher's `INDEXABLE` MIME list contains no video type.
- A poster is stored only when it is **derivable without a fetch**: YouTube
  (`i.ytimg.com/vi/{id}/hqdefault.jpg`) and Dailymotion publish thumbnails at URLs computable
  from the id; a self-hosted `<video poster>` is an image URL and kept. Everything else has no
  poster rather than a poster we paid a request for.

Providers: `youtube` (`youtube.com`, `m.`, `youtube-nocookie.com`; `/watch?v=`, `/embed/`,
`/shorts/`, `/v/`), `dailymotion` (`dailymotion.com`, `geo.`, `dai.ly`), `vimeo`, `self`. The
provider is named on the tile because leaving our site is the reader's choice.

### 5.3 Index and verticals

Meilisearch flattens arrays of objects, so `media.type` and `media.provider` are filterable and
select any document carrying at least one entry of that type. `media.ocr_text` is searchable
(that field is written by [[Image Pipeline]]).

### 5.4 Grid

`MediaGrid` is a pure server component: a CSS grid of `<img loading="lazy">` through the signed
proxy, and video tiles that link out with their provider named. No JavaScript ships for either —
a gallery script would fail the no-JS path and the bundle budget for nothing the browser cannot
already do. One tile per image in result order, so relevance still reads left-to-right.

### 5.5 `media-repass`

Documents indexed before M9 have no videos. `xustive-cli media-repass [--limit N] [--dry-run]`
re-runs extraction over bodies still held in the raw store (`crawl.raw_ttl_days`; 0 means nothing
is kept and the command says so). The report states how many documents still had a body and how
many did not, because "the Videos tab is sparse" has two very different causes and an operator
needs to know which one they are looking at.

## 6. Configuration

| Key | Default | Meaning |
|:---|:---|:---|
| `ParseConfig.max_media` (code, not TOML) | 4 | images per page, and separately videos per page |
| `crawl.raw_ttl_days` | see `config/*.toml` | how long raw bodies exist for a repass |

The `[media]` table in `config/*.toml` (`image_ocr_enabled`, `max_images_per_doc`,
`max_image_bytes`, `ocr_backend`) governs OCR and embedding of the extracted images, not
extraction itself — see [[Image Pipeline]].

## 7. Failure Modes

| Failure | Response |
|:---|:---|
| Page has no recognisable media | empty `media[]`; the document indexes normally |
| Embed URL from an unknown host | ignored; nothing is stored for it |
| Upstream image gone at render time | the proxy answers a transparent pixel ([[Thumbnail Proxy]]) |
| Raw body expired before repass | counted and reported as "no body" |

## 8. Security

Media URLs are untrusted strings from crawled HTML. Nothing on the ingestion side fetches them
except the opt-in OCR/embedding passes, which go through the crawler's `SafeUrl` guard. On the
serving side the only fetch is the signed proxy, which re-validates the host on every hop. Video
bytes are never fetched anywhere — enforced by a test that greps the fetcher for video MIME
acceptance (M9-T01.4).

## 9. Observability

Extraction has no metrics of its own; coverage is visible as the `images` / `videos` vertical hit
counts and in the `media-repass` report. The admin **Media** page shows the OCR/embedding stack
health, not extraction.

## 10. Open Questions

- [ ] Image dimensions are always 0; reading `width`/`height` attributes when present would let
      the grid reserve space and avoid layout shift.
- [ ] Should `max_media` be a TOML key rather than a `ParseConfig` default?

## Related

[[Milestone 9 - Images and Videos]] · [[Content Parser]] · [[Image Pipeline]] · [[Thumbnail Proxy]] ·
[[Search Index]] · [[UI - Search Verticals]] · [[UI - Image Search]]

## Since M10 (2026-08-27)

`Media` has `ext` and `style` (`crates/xustive-core/src/model.rs`). The parser sets `ext` from
the URL at extraction (`xustive_media::ext::from_url`); the embed pass corrects it from the bytes
(`from_bytes`, magic numbers — a `.jpg` that is a PNG is a PNG) and sets `style`. Both are
filterable on `documents` (`media.ext`, `media.style`) and the Images tab accepts `ext=` and
`style=`; `MediaOut` carries them on the wire.
