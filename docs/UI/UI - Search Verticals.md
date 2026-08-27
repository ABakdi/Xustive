---
tags:
  - ui
type: ui
status: built
updated: 2026-08-27
---

# UI — Search Verticals

> The tab row above results. Today: **All, News, Files, Images, Videos**
> (`web/components/search/Verticals.tsx`). Short videos and Social are still to come.
>
> Interface here; the content each depends on is in [[Image Pipeline]], [[Web Fetcher]] and the
> social connectors. Tiles for Images/Videos: `web/components/search/MediaGrid.tsx`; the signed
> thumbnail proxy: `web/lib/thumb.ts` + `web/app/api/thumb/route.ts`
> ([[ADR-0021 - Proxied Thumbnails with Signed URLs]]).

---

## 1. The problem with building all of them now

A tab is a promise that there is something behind it. When this note was written (2026-08) most
of these had nothing:

| Tab | Content exists? (2026-08-27) | Notes |
|:---|:---|:---|
| **All** | yes | — |
| **News** | yes | a filter over what is already indexed |
| **Files** | **yes** | the fetcher now accepts `application/pdf` ([[Web Fetcher]] `INDEXABLE`), the orchestrator extracts text, and `?v=files` filters `content_type = application/pdf`. Earlier text here said PDFs were refused outright — superseded. |
| **Images** | **yes** (M9) | pages carry `media[]` images; [[Milestone 9 - Images and Videos]] filters and renders them |
| **Videos** | **yes** (M9) | the parser extracts `og:video`, `<video>` and YouTube/Dailymotion/Vimeo embeds; fills as pages are (re)parsed |
| **Short videos** | **no** | TikTok / Reels — the social connectors, M2-T08–T10 |
| **Social** | **no** | same |

Shipping seven tabs where five return "no results" is worse than shipping two. An empty tab is
indistinguishable from a broken one, and the reader's conclusion is that the engine does not work
rather than that the feature is unfinished.

**So tabs appear as their content does.** The row renders the verticals that can return something,
and each new one lights up when its pipeline lands. Images and Videos were shown before the corpus
had filled — the operator asked for them — with the empty state naming the vertical so an empty
tab is honest rather than indistinguishable from a broken one.

## 2. What each tab actually is

Not separate indexes. One corpus, filtered (`crates/xustive-api/src/search.rs`, the `?v=` match) —
which is why News and Files are cheap and Images is not.

| Tab | Definition |
|:---|:---|
| All | no filter |
| News | `source_type = web` and `exclude_unknown_dates` — a guessed date is not news |
| Files | `content_type = application/pdf` |
| Images | `media_kind = image`; a tile is a page that has the image, ranked by page relevance and OCR text; thumbnails proxied per [[ADR-0021 - Proxied Thumbnails with Signed URLs]] |
| Videos | `media_kind = video`; poster + link out to the **watch page**, never an embedded player |
| Short videos | (planned) video documents under a duration threshold, plus platform origin |
| Social | (planned) `source_type != web` |

An unknown `?v=` falls back to All rather than erroring, so a stale link still returns results.

The News definition is worth arguing about: "has a date" is doing most of the work. A section index
and an article are both HTML on a news site, and the date is the cheapest signal that separates
them. It will be wrong sometimes — an undated article is excluded — which is the right direction
to be wrong in for a tab whose whole promise is recency.

### Federation per category (M9-T06)

When the local index is thin, the federation gateway asks SearXNG for the matching **category**:
`general` for All/News/Files, `images` for Images, `videos` for Videos
(`Category::from_vertical` in `crates/xustive-federation`). Federated hits carry `from_web` and the
tiles badge them `t.fromTheWeb` in accent, for the same reason the list badges them: provenance is
the reader's to judge.

## 3. Behaviour

- **The tab is in the URL** (`?v=news`, `?v=files`, `?v=images`, `?v=videos`; All has no `v`), so a
  vertical is shareable and the back button works. A tab that only exists in memory is a tab that
  vanishes when someone sends a link.
- **The query survives switching.** Each tab is a `<Link>` to `/[lang]/search?q=<same q>&v=<id>`;
  other filters (`lang`, `source`, `sentiment`, `page`) are **dropped** on switch — a fresh start in
  the new vertical.
- **Counts are not shown per tab.** Getting them means running every vertical on every search —
  seven queries to render one row. Google shows no counts either, and for the same reason.
- **An empty vertical says which vertical is empty** — `t.noNews` / `t.noFiles` / `t.noImages` /
  `t.noVideos` — with a link back to All (`t.noNewsHint`, "Show all results"). A generic "no
  results" leaves the reader unsure whether the engine has nothing or the tab is broken.
- **Pure server component, no JavaScript.** The row is a `<nav aria-label={t.verticalAll}>` of
  links with `aria-current="page"` on the active one; the accent underline is `border-b-2`. It is
  **not** a `role="tablist"` and arrow keys do not move between tabs — the original wording said
  they would; superseded 2026-08-27. Tab order through the links is enough for a navigation
  control that is a set of URLs.

## 4. Images and Videos tiles (M9-T03)

Both are pure server components in `MediaGrid.tsx`, rendered in place of the result list when
`?v=images` / `?v=videos`; the `InteractionBeacon` wrapper is not applied to them.

**`ImageGrid`** — `<ul aria-label={t.verticalImages}>`, CSS grid `repeat(auto-fill, minmax(160px, 1fr))`.
One tile per image in result order (a page with three images is three tiles, adjacent), so
relevance still reads start-to-end, top-to-bottom. Each tile links to **the page**
(`rel="noopener noreferrer nofollow"`, `dir="auto"`): the page is what we indexed and what a reader
can judge. The `<img>` is `loading="lazy" decoding="async" referrerPolicy="no-referrer"`,
`aspect-[4/3] object-cover` on a `--bg-sunk` box, `alt` = the page title; below it the title
(`line-clamp-2`) and the host in `<bdi>`.

**`VideoList`** — `<ul aria-label={t.verticalVideos}>`, `minmax(240px, 1fr)`. The tile links to the
**watch page** (`m.url`) with `target="_blank"`; the poster (if any) is proxied like any image under
a ▶ glyph (`aria-hidden`); the caption reads `t.watchOn` + provider name (YouTube / Dailymotion /
Vimeo, else the watch host) `·` page host. Nothing is embedded — an embedded player is a
third-party page load the reader did not choose (ADR-0021) — and the provider is named because
leaving our site is the reader's decision. Videos are **metadata only**: the index holds the URL,
poster and provider, never the media.

**Signed thumbnails.** `signThumb(url)` (server-only) returns `/api/thumb?u=<url>&s=<HMAC>` or
`null` for a URL that must not be proxied (non-https, credentials, IP literals, local/private
names). The route refuses anything unsigned with 403 before any fetch — except
`upload.wikimedia.org`, `commons.wikimedia.org` and `covers.openlibrary.org`, which are public
by construction and skip the signature so a browser-cached relation row survives a restart —
re-validates every redirect hop, caps at 5 MB / 4 s, and answers a transparent GIF on upstream
failure so a grid never shows broken-image icons. The secret is `XUSTIVE_THUMB_SECRET` or a random
per-process value held on `globalThis` (two Next bundles share one process, not one module).

## 5. Files

Files shipped as a filter over PDF documents the fetcher now indexes. The concerns that were
listed here before it landed still apply to what the tab *covers*: a large share of `.gov.dz`
PDFs are scans of printed documents with no text layer, so text extraction returns nothing for
them and the tab covers the born-digital minority until OCR ([[Image Pipeline]]) is applied to
PDFs. Size and page caps live in the fetcher ([[Web Fetcher]]).

## 6. Order of work

1. ~~**News**~~ — done.
2. ~~**Files**~~ — done (PDF text extraction; scanned PDFs still empty).
3. ~~**Images / Videos**~~ — done, [[Milestone 9 - Images and Videos]].
4. **Social** and **Short videos** — arrive with the connectors, which are already tracked.

## 7. Related

[[UI - Results Page]] · [[UI - Filters and Facets]] · [[UI - Image Search]] (search *by* an
image, a different feature) · [[Image Pipeline]] · [[Web Fetcher]] ·
[[Milestone 3 - Multimodal Input]] · [[Milestone 9 - Images and Videos]] ·
[[ADR-0021 - Proxied Thumbnails with Signed URLs]]
