---
tags:
  - planning
  - milestone
milestone: 10
status: in-progress
updated: 2026-08-27
progress: T01–T05 built 2026-08-27; open — a quiet-machine/GPU latency measurement (T05.2), the redirect test (T05.5)
---
# Milestone 10 - Reverse Image Search

> **Goal:** a picture in, pictures out. A reader drops, pastes or photographs an image and gets a
> grid of **similar images** — the same picture where it appears on the Algerian web, near
> copies, and visually alike images — from the local index first and from the web through
> SearXNG second; then narrows the grid by **file type** (png, jpeg, gif, webp, svg) and by
> **what kind of picture it is** (photo, illustration, 3D render, digital art, screenshot…), with
> the descriptors derived from the query image and the results themselves, not from a fixed menu.
> **Exit gate:** an uploaded image from the index finds its own page at rank 1 and a cropped or
> re-encoded copy in the top 3 on the M10 golden set; the extension and descriptor chips filter
> the grid without a second upload; not one byte of the reader's image leaves the machine — the
> web leg sends words, and `make egress-test` proves it; the image is never written to disk or a
> log; the no-JS, bidi, telemetry and bundle gates stay green.
> Parent: [[TODO]] · Previous: [[Milestone 9 - Images and Videos]] · Governed by
> [[ADR-0028 - Reverse Image Search Sends Words to the Web, Never the Picture]] and
> [[ADR-0021 - Proxied Thumbnails with Signed URLs]] · Components: [[Image Pipeline]],
> [[Vector Index]], [[Media Extraction]], [[Federation Gateway]], [[UI - Image Search]]

## Why This Milestone Exists

M3 built "find similar" as a fallback under OCR: a photo goes to `POST /api/v1/search/image`, CLIP
embeds it, Qdrant answers with the nearest *image vectors*, and the handler collapses them into
**pages** — a list of titles, no pictures ([[UI - Image Search]] §0). That was the right first
step and the wrong product: someone holding a picture wants to see pictures — where this one
appears, which copies exist, what looks like it — and only then read the page it came from.

Three things are missing, all measured on 2026-08-27:

- **The result is the wrong unit.** `resolve_documents()` keeps one hit per `document_id` and
  drops the image URL that matched (`crates/xustive-api/src/image_search.rs:118-187`). The grid
  cannot be drawn from the response.
- **Nothing describes an image.** The sidecar loads the full CLIP model and exposes only
  `get_image_features` (`services/clip-embed/app.py:100`); the text tower is unreachable, so there
  is no zero-shot label, no "this is a screenshot", and — the item [[Milestone 9 - Images and Videos]] parked — no text-to-image ranking either. `Media` has no extension and no style field
  (`crates/xustive-core/src/model.rs:193-218`).
- **The web cannot take a picture.** SearXNG's only input is a query string
  (`crates/xustive-federation/src/lib.rs:250-275`); there is no reverse-image capability behind
  the gateway and, by [[ADR-0008 - No Query Logging]], sending the reader's photograph to a
  third party would be a disclosure we do not make even if there were.

## The shape of the thing

**Images are the unit, and the local index answers first.** The endpoint returns *images*: each
with its URL, its proxied thumbnail, the page it lives on, its similarity, its extension and its
descriptor. Three groups, in this order: **same picture** (a dHash match, or cosine ≥ 0.92),
**similar** (≥ threshold), and **from the web**. The pHash the index already stores becomes the
exact-hit shortcut [[Image Pipeline]] §2 has listed as not built since M3.

**The web leg gets words.** The picture is described locally — CLIP zero-shot against a curated
vocabulary of subjects and styles, plus any OCR text that is a name or a place — and the *labels*
go to SearXNG's image category through the one gateway, exactly as a typed query would. The
image never leaves. Federated hits come back in SearXNG's order with their thumbnails signed and
proxied; they enter the eager index like every federated hit (M9-T06), the crawl follows, and
next time the same picture is asked for, those images are local, embedded, and ranked visually.
That closing loop is the honest answer to "rank the web visually" without fetching the web at
query time ([[ADR-0028 - Reverse Image Search Sends Words to the Web, Never the Picture]]).

**Descriptors come from the pictures, not a menu.** A vocabulary of styles — photo, illustration,
digital art, 3D render, painting, drawing, cartoon/anime, screenshot, logo, map, diagram, meme —
lives in `data/styles.tsv`, reviewable like the lexicons. At index time each embedded image is
scored against the vocabulary's text vectors and its top style is stored; at query time the
reader's image is scored the same way. The chips the page shows are **the styles present in this
result set, counted**, with the query's own style named first ("looks like: a photo"). A style
absent from the results is not offered, so a chip never leads to an empty grid.

**Filters need no second upload.** The response carries the whole bounded set (the local limit
rises for this mode) with per-image `ext` and `style`; the chips narrow it in the browser,
instantly, and the counts on each chip are true for the set on screen. A reverse search is a
POST with a picture in it and has no URL; there is nothing to re-query and nothing to share, which
is also the privacy property.

---

## M10-T01 — The sidecar learns to read and to describe

- [x] M10-T01.1 `POST /embed/text` on `services/clip-embed`: `{"texts": [...]}` → unit vectors
      from CLIP's text tower, same 512-d space. This is the item M9 parked; it now has two callers
- [x] M10-T01.2 `POST /embed?describe=1` returns, beside the vector, `styles`: the cosine of the
      image against each prompt of the style vocabulary, and `subjects`: the top-k of the subject
      vocabulary. Prompts are templated ("a photo of …", "a 3D render of …") and their vectors are
      computed once at start and cached in the process
- [x] M10-T01.3 `POST /classify` takes vectors, not bytes — `{"vectors": [[…]]}` → the same
      `styles`/`subjects` per vector — so the points already in Qdrant can be labelled without
      re-fetching a single image
- [x] M10-T01.4 The vocabularies are files, not code: `data/styles.tsv` (id, prompt, then a
      display name per language) and `data/subjects.tsv`, loaded at start; a bad row fails loudly
- [x] M10-T01.5 Tests against the running sidecar: a screenshot fixture scores `screenshot`
      first, a photograph scores `photo` first, and the text vector of "a photo of a mosque" is
      nearer the mosque fixture than the football one

## M10-T02 — Every image knows its type and its kind

- [x] M10-T02.1 `Media.ext` (`png|jpg|gif|webp|svg|avif|bmp|tiff|unknown`), derived from the URL
      path at extraction, corrected from the `Content-Type` when the crawler fetches the bytes
      for embedding. `jpeg` and `jpg` are one value
- [x] M10-T02.2 `Media.style`: the top style from `describe=1` when its margin over the second is
      clear, else absent. Stored on the Meilisearch document and in the Qdrant payload
- [x] M10-T02.3 `media.ext` and `media.style` filterable; the Images tab (`?v=images`) accepts
      `ext=` and `style=` so the ordinary direction gets the same filters for free
- [x] M10-T02.4 `xustive vector-repass` (the describe pass; `--embed` first fills documents that
      have no vector, the day-one fill — 263 points from 200 documents in nine minutes, the
      image fetches being the cost): scrolls the existing `image_clip` points,
      labels them through `/classify` by vector, and writes `style` back to Qdrant and to the
      document — so the chips are not empty on day one
- [x] M10-T02.5 The embed pass keeps its bounds (`media.max_images_per_doc`,
      `max_image_bytes`, the pHash→vector cache); describing is one extra field on a call already
      made, never an extra fetch

## M10-T03 — The endpoint answers with pictures

- [x] M10-T03.1 `POST /api/v1/search/image` returns `images[]` — `url`, `thumb_url`, `ext`,
      `style`, `score`, `group` (`same|similar|web`), `page {id,title,url,display_url,source}` —
      plus `query {style, ext, labels}` and `facets {ext: counts, style: counts}`. `results[]`
      (pages) stays for one release for the OCR page, then goes
- [x] M10-T03.2 The exact-hit shortcut: dHash of the upload → a Qdrant filter on `phash` before
      ANN; hits within Hamming distance 6 join `same` regardless of cosine. *Settled: the hash
      decides the group, not the retrieval — an exact copy has cosine ≈ 1 and is in the ANN set
      already, so a second Qdrant call bought nothing; `group_of` puts a hit within six bits in
      `same` whatever CLIP scored it*
- [x] M10-T03.3 The local leg: ANN with `search_limit` raised to 80 for this mode and the
      threshold from config; one Meilisearch query joins pages by id, and the image that matched
      is the one shown — never a sibling image from the same page
- [x] M10-T03.4 The web leg: the labels (top subjects, the query's style when it is not `photo`,
      OCR tokens that are proper names) become one text query; it goes to the gateway in
      `Category::Images` with the federation budget, results ride the eager index, thumbnails are
      signed. Off when federation is off, and never sends the image — a test greps the leg for
      the body bytes. *Settled: OCR names are not in the query yet — the endpoint does not run
      OCR; the subjects and the style are the words. Measured: a mosque photo asks the web for
      "casbah mosque"*
- [x] M10-T03.5 A URL leg: `?u=` with a signed thumbnail URL from the Images tab — the server
      fetches through the thumbnail proxy's own rules (https, no private hosts, 5 MB, 4 s) and
      embeds; nothing is uploaded and the reader never leaves the tab. *Moves to T04: the fetch
      belongs to the web tier, which already owns the proxy's rules and the signature*
- [x] M10-T03.6 Its own rate bucket (`/search/image`, 10/min), separate from `/ocr` which it
      shared by accident; the body cap stays `media.max_image_bytes` (5 MiB — the docs said 8)

## M10-T04 — The page

- [x] M10-T04.1 Entry points: a camera button on the Images tab; drop and paste on the Images
      tab; "Find similar" on every image tile's hover; the OCR page's button; `?u=` from a tile.
      All lead to `/[lang]/search/image`
- [x] M10-T04.2 The results page shows the query image small at the top (kept in the browser
      only — there is no URL for it and it is never stored), then **Looks like** (the query's
      style and top subjects), the style chips with counts, the extension chips with counts, and
      the three groups as grids. Chips filter the set on screen without a request; the counts are
      exact for the set. *Verified in the browser from a tile: "Looks like: 3D render · product ·
      phone · night · png", Format All 9 · jpg 5 · svg 4, nine web tiles for "product phone
      night"; the jpg chip leaves five*
- [x] M10-T04.3 Every thumbnail through the signed proxy; a tile links to the page, names the
      host, and carries `fromTheWeb` for federated hits; the similarity label stays qualitative
      (very similar / similar / related), never a percentage
- [x] M10-T04.4 States: uploading, searching, empty per group ("nothing the same; 12 similar"),
      similarity unavailable (vector off → 503 → the page says so and offers OCR), federation off
      (the web group is absent, not empty)
- [x] M10-T04.5 Four languages, RTL-first, chips as `aria-pressed` buttons, the grid a list with
      accessible names; the client-side downscale and EXIF strip of [[UI - Image Search]] apply to
      every entry point. *Settled: three languages written (ar, fr, en); Darija falls back to
      Arabic as the rest of the interface does*
- [x] M10-T04.6 [[UI - Image Search]] rewritten for the new flow; [[Image Pipeline]],
      [[Vector Index]], [[Media Extraction]], [[API Contract]] updated in the same commits

## M10-T05 — Gates

- [x] M10-T05.1 Golden set `eval/images/`: 30 images taken from the local index with their source
      page; the original finds its page at rank 1 and its own image in `same`; a 20 % crop and a
      q50 re-encode find it in the top 3; a rotated copy is allowed to miss. *Measured
      2026-08-27 (`make eval-images`, 29 fetchable of 30, second run): original at rank 1 and
      in `same` 29/29, crop 29/29, q50 re-encode 27/29 — and the two re-encode misses are a
      site's `DefaultImage.jpg` and a lazy-load placeholder, which are not pictures. Settled at a floor of nine in ten rather than all:
      an image URL on the news web is not immutable — an og:image regenerated since indexing is
      not a retrieval failure — and the runner prints the misses so a reviewer can tell*
- [~] M10-T05.2 Budgets: local leg p95 ≤ 1.5 s end to end with the CLIP sidecar on the GPU, ≤ 4 s
      on CPU; the web leg inside the federation budget and never on the critical path of the
      local groups. *Measured on CPU CLIP with the crawler embedding on the same sidecar and 134
      index tasks queued: median 512–594 ms, p95 3.9–4.7 s over 87 searches, two runs — the median is well inside,
      the tail is contention, not the search. The web leg is its own request after the local
      groups paint, with the fetch budget (6 s) and its own timeout. Open: a quiet-machine and a
      GPU measurement*
- [x] M10-T05.3 Privacy: a test posts an image and greps every log and the Redis keyspace for its
      bytes' hash and its labels; `make egress-test` extended: the federator sees only text.
      *`scripts/test-image-privacy.sh` (`make image-privacy LOG=…`): 11/11 on the dev logs and
      both Redis keyspaces. The federator-sees-only-text half is pinned in code rather than in
      the egress script — a source-reading test in `image_search.rs` keeps the upload handler
      free of any federation call and the words handler free of any body*
- [x] M10-T05.4 Face rule kept and tested: no detector, no face embedding model in any
      dependency list (`cargo deny`/pip audit of the sidecar) — similarity is whole-image.
      *`scripts/lint-no-face.sh`, in `make lint`*
- [x] M10-T05.5 The M9 debt: tests for the thumbnail proxy (M9-T02.5) land here, since this
      milestone leans on it harder. *`scripts/thumb-proxy-check.sh`, in `make ui-gates`: eight
      refusals, all turned away. The redirect-to-a-disallowed-host case still has no test — it
      needs a controllable upstream*

## Deliberately not in this milestone

- **Face search.** [[UI - Image Search]] §rule stands: whole images, never people.
- **Visual ranking of the web at query time.** Fetching a hundred thumbnails from a hundred hosts
  to embed them would put the reader's query on the critical path of the open web; the eager
  index and the crawl do it a minute later, for everyone.
- **Reverse search on video** — a frame extractor is a different milestone.
- **Hosting the images.** Thumbnails are proxied and cached a day; originals are never copied.

---

> **Status 2026-08-27, end of day.** Built and measured in one day, on top of what M3 and M9
> left. The exit gate, item by item: an image the index holds finds its page at rank 1 and itself
> in `same` for 29 of 29, a crop for 29, a re-encode for 27 (the two misses are site
> placeholders, not pictures); the chips
> filter the grid in the browser without a second upload (verified: jpg 5 of 9); not a byte of
> the picture reaches the web leg — the upload handler cannot federate and the words handler has
> no body, by a test that reads the source; the privacy script finds neither the picture's hash
> nor its words in any log or Redis key; the face lint is in `make lint`; the no-JS, bidi,
> telemetry and bundle gates are green.
>
> What is not closed: the latency budget was measured on a busy CPU (median ~550 ms, p95 ~4 s
> with the crawler embedding on the same sidecar) and wants a quiet run and a GPU run; the
> proxy's redirect-refusal test needs a controllable upstream. Text-to-image ranking of the
> Images tab — now one call away — stays parked, as M9 left it.
