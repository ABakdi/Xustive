---
tags:
  - planning
  - milestone
milestone: 9
status: planned
updated: 2026-08-26
---
# Milestone 9 - Images and Videos

> **Goal:** the same query, seen. A reader types text and gets pages; this milestone adds an
> **Images** tab and a **Videos** tab over the same corpus, so the same query returns the pictures
> and the videos the Algerian web attached to it.
> **Exit gate:** `?v=images` renders a server-side grid with **no JavaScript** and no reader
> address reaching any crawled host; `?v=videos` lists videos without ever embedding a player or
> downloading a byte of video; both tabs name themselves when empty; `make egress-test`, the
> no-JS check, bidi lint and bundle budget stay green.
> Parent: [[TODO]] · Previous: [[Milestone 8 - The Answer Layer]] · Governed by
> [[ADR-0021 - Proxied Thumbnails with Signed URLs]] · Components: [[Image Pipeline]],
> [[Content Parser]], [[UI - Search Verticals]]

## Why This Milestone Exists

[[UI - Search Verticals]] laid the rule down in M2: *tabs appear as their content does*, because an
empty tab is indistinguishable from a broken one. Images and Videos were parked there "for M3",
and M3 built the reverse-image search instead — a photo in, similar pages out. Nobody built the
ordinary direction: words in, pictures out.

The content has been there the whole time. A sample of the live index shows **176 of 200 pages
carry `media[]` image entries** — the `og:image` and the article figures the parser has extracted
since M2 — and not one of them is ever displayed. Width and height are zero because nothing
measures them; `thumbnail_url` is on the wire and unrendered; `media.type` is not filterable, so
even a hand-written query could not select them.

Video is the honest gap: the parser extracts **nothing**. `<iframe>` is on its invisible-element
list, so every embedded YouTube or Dailymotion player is discarded at parse time, and `og:video`
is never read. The Videos tab therefore needs extraction first and a tab second.

## The shape of the thing

Three decisions, each cheap because the architecture already made the expensive part.

**One corpus, filtered — not a new index.** Images and Videos are verticals in the exact sense
[[UI - Search Verticals]] §2 defines: a saved filter over `documents`, in the URL as `?v=`, so a
tab is a link and the back button works. The filter is `media.type = image`; Meilisearch flattens
arrays of objects, so this is a settings change, not a reindex. A tile is *a page that has this
image*, ranked by the page's relevance and the image's OCR text, which is what an Algerian query
about a place or a person actually wants.

**Thumbnails are proxied and signed** ([[ADR-0021 - Proxied Thumbnails with Signed URLs]]). The
grid must not hotlink: forty images from forty crawled hosts is forty disclosures of the reader's
address, to sites we chose to crawl and they did not choose to visit. ADR-0014 already proxies
Wikimedia through the web tier with a fixed host allowlist; an allowlist cannot cover the open
web, so instead the server that renders the page **signs** each thumbnail URL and the proxy serves
only what carries a valid signature. It cannot be turned into an open proxy, and the reader's
browser only ever talks to us.

**Video is linked, never played.** A `<video>` tile that embeds a YouTube player hands the reader
to Google on page load. The tile shows the poster (proxied), the title, the host, and links out on
a click — the reader chooses the disclosure. And no code path downloads video bytes, which
[[Milestone 2 - Ingestion at Scale|M2-T10.8]] already required and this milestone pins with a test.

---

## M9-T01 — Video extraction and a filterable media type

- [ ] M9-T01.1 The parser reads `og:video` / `og:video:url`, `<video src>` and `<video><source>`,
      and the embed iframes of YouTube, Dailymotion and Vimeo — `<iframe>` comes **off** the
      invisible list for this one purpose and stays invisible for text. Each becomes a
      `Media { kind: Video }` whose `url` is the **watch page**, never a stream
- [ ] M9-T01.2 A poster for every video that has one derivable without a fetch: YouTube and
      Dailymotion publish thumbnails at URLs computable from the id. Stored in `thumb_url`, so the
      tab can render on day one with zero extra crawling
- [ ] M9-T01.3 `provider` on video media (`youtube` / `dailymotion` / `vimeo` / `self`), because
      the tile names where a click will take the reader
- [ ] M9-T01.4 **No video bytes, ever.** A test greps the fetcher for video MIME acceptance and
      the parser for stream URLs, and fails on the commit that adds either
- [ ] M9-T01.5 `media.type` and `media.provider` become filterable; `Filters` gains `media_kind`;
      `?v=images` and `?v=videos` map to it beside `news` and `files`. Unknown verticals still fall
      back to All
- [ ] M9-T01.6 The result card carries `media[]` on the wire — url, thumb, kind, provider,
      dimensions — capped per page so one gallery article cannot fill a whole grid

## M9-T02 — The signed thumbnail proxy

- [ ] M9-T02.1 `/api/thumb?u=…&s=…`: HMAC-SHA256 over the URL with a server secret
      (`XUSTIVE_THUMB_SECRET`; a per-process random one when unset, so a missing secret degrades to
      "thumbnails break on restart", never to an open proxy)
- [ ] M9-T02.2 The search page — a server component, so it holds the secret — signs every
      thumbnail it renders. The browser never sees the secret and never sees a crawled host
- [ ] M9-T02.3 Same guards as `wiki-image`, and two more: `https` only, no IP-literal or private
      hostnames, redirects followed by hand and re-validated per hop, `image/*` only, 5 MB cap
      checked on header and body, 4 s timeout, `Referrer-Policy: no-referrer`
- [ ] M9-T02.4 Cached a day, keyed by the URL: the same thumbnail for every reader is one fetch
      from the crawled host, which is also the polite thing
- [ ] M9-T02.5 Tests: a forged signature is a 403; a private host is a 400 even when signed; a
      redirect to a disallowed host is refused mid-chain

## M9-T03 — The tabs

- [ ] M9-T03.1 Images and Videos join the tab row, in all four languages. Shown always: the
      operator asked for them, and the empty state below is what makes an empty tab honest
- [ ] M9-T03.2 The image grid — a pure server component, CSS grid, `<img loading="lazy">`
      through the proxy, no JavaScript. Each tile: the image, the page title, the host, a link to
      the page. `alt` from the page title, never empty
- [ ] M9-T03.3 The video list — poster, play glyph, title, provider, host; the whole tile links
      to the watch page with `rel="noopener noreferrer nofollow"`. Nothing embedded
- [ ] M9-T03.4 The empty state names the vertical — "no images for *سونلغاز*" — and links back
      to All, per [[UI - Search Verticals]] §3. A generic "no results" would leave the reader
      unsure whether the engine has nothing or the tab is broken
- [ ] M9-T03.5 A tile whose thumbnail fails to load must not leave a hole: the proxy answers a
      transparent 1×1 on upstream failure so the grid stays a grid, and the title still links
- [ ] M9-T03.6 RTL: the grid is direction-agnostic by construction; titles are `<bdi>`-isolated;
      bidi lint green

## M9-T04 — Repass, so the tabs are not empty on day one

- [ ] M9-T04.1 `xustive media-repass`: walk the index, re-extract media from the **raw store**
      body where one is still held (`crawl.raw_ttl_days`), and patch only `media`. No refetch —
      the whole point is not to pay a crawl for a parser change
- [ ] M9-T04.2 Honest about coverage: reports how many documents had a stored body and how many
      did not. The ones that did not get their video entries on the next revisit, which the
      adaptive recrawl already schedules

## M9-T05 — Gates

- [ ] M9-T05.1 `scripts/no-js-check.sh` extended: `?v=images` renders tiles with script off
- [ ] M9-T05.2 `make egress-test` green — the web tier's fetch is the same class as ADR-0014's;
      the serving plane's no-egress is untouched
- [ ] M9-T05.3 Bundle budget holds — the grid ships no JavaScript at all
- [ ] M9-T05.4 Telemetry lint: no image URL in a log line (a URL is a page someone read)

## Deliberately not in this milestone

- **CLIP text-to-image ranking.** The reverse-image search embeds *images*; ranking image tiles
  by a text query needs the CLIP *text* encoder, which the sidecar does not currently expose.
  Ordinary page relevance plus OCR text is the honest ranking until it does. Tracked as the
  natural next step, opt-in behind `[vector]` like everything else there
- **Short videos and Social.** Blocked on the social connectors exactly as [[UI - Search Verticals]]
  says
- **Measuring image dimensions at crawl.** Would need the crawler to fetch every image; the grid
  does without by letting the browser size the tile
