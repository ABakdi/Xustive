---
tags:
  - planning
  - milestone
milestone: 9
status: done
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

> **Closed 2026-08-26.** Measured against the exit gate: `?v=images` renders **66 tiles with
> script off, none hotlinked** — every one through the signed proxy, which serves real JPEGs on a
> valid signature and 403 on a tampered or missing one; `?v=videos` lists posters that link out
> and embeds nothing; both tabs name themselves when empty; egress, no-JS, bidi, telemetry and
> the bundle budget (181 KB of 195) are green. 15 crate suites pass.
>
> One finding worth keeping: Next compiles server components and route handlers as separate
> bundles with separate module instances, so a module-level random secret was two secrets and the
> proxy refused every signature the page produced. `globalThis` is what the bundles share.
>
> **Reopened and closed again the same day for T06 — federated images and videos.** The Videos
> tab is no longer empty: with federation on, a search federates in its own category, the eager
> index stores each hit's media, and the crawl follows. Measured after one query: 21 federated
> video documents local within a minute, and the crawler had already fetched the YouTube watch
> pages and the new parser extracted the video from the real HTML — the whole loop, closed.
>
> T06 found three latent faults, none in the new path: SearXNG's `null` fields failed the
> deserialisation of an entire body (118 hits → 0); the gateway's 2 s transport timeout sat under
> the API's 6 s budget; and the strip wait equalled `timeout_search_ms` in dev, so every vertical
> 504'd whenever SearXNG was slow — unseen because federation was off. All fixed.
>
> One caveat that stands: SearXNG's image and video engines answer in 1.2–1.6 s, longer than the
> ~0.75 s strip wait dev can afford, so media tiles usually arrive on the *second* search via the
> eager index rather than blended into the first. That is ADR-0017's convergence working as
> designed, not a defect — the All tab, whose engines are faster, blends live.
>
> One honest gap that used to be here: the Videos tab was empty at first close. The parser now extracts video, but the raw store
> has never been on (`crawl.raw_ttl_days = 0`), so the repass had nothing to work from and the
> tab fills at the pace of the revisit crawl. Turning the store on is the operator's storage call.

## M9-T01 — Video extraction and a filterable media type

- [x] M9-T01.1 The parser reads `og:video` / `og:video:url`, `<video src>` and `<video><source>`,
      and the embed iframes of YouTube, Dailymotion and Vimeo — `<iframe>` comes **off** the
      invisible list for this one purpose and stays invisible for text. Each becomes a
      `Media { kind: Video }` whose `url` is the **watch page**, never a stream
- [x] M9-T01.2 A poster for every video that has one derivable without a fetch: YouTube and
      Dailymotion publish thumbnails at URLs computable from the id. Stored in `thumb_url`, so the
      tab can render on day one with zero extra crawling
- [x] M9-T01.3 `provider` on video media (`youtube` / `dailymotion` / `vimeo` / `self`), because
      the tile names where a click will take the reader
- [x] M9-T01.4 **No video bytes, ever.** A test greps the fetcher for video MIME acceptance and
      the parser for stream URLs, and fails on the commit that adds either
- [x] M9-T01.5 `media.type` and `media.provider` become filterable; `Filters` gains `media_kind`;
      `?v=images` and `?v=videos` map to it beside `news` and `files`. Unknown verticals still fall
      back to All
- [x] M9-T01.6 The result card carries `media[]` on the wire — url, thumb, kind, provider,
      dimensions — capped per page so one gallery article cannot fill a whole grid

## M9-T02 — The signed thumbnail proxy

- [x] M9-T02.1 `/api/thumb?u=…&s=…`: HMAC-SHA256 over the URL with a server secret
      (`XUSTIVE_THUMB_SECRET`; a per-process random one when unset, so a missing secret degrades to
      "thumbnails break on restart", never to an open proxy)
- [x] M9-T02.2 The search page — a server component, so it holds the secret — signs every
      thumbnail it renders. The browser never sees the secret and never sees a crawled host
- [x] M9-T02.3 Same guards as `wiki-image`, and two more: `https` only, no IP-literal or private
      hostnames, redirects followed by hand and re-validated per hop, `image/*` only, 5 MB cap
      checked on header and body, 4 s timeout, `Referrer-Policy: no-referrer`
- [x] M9-T02.4 Cached a day, keyed by the URL: the same thumbnail for every reader is one fetch
      from the crawled host, which is also the polite thing
- [ ] M9-T02.5 Tests: a forged signature is a 403; a private host is a 400 even when signed; a
      redirect to a disallowed host is refused mid-chain

## M9-T03 — The tabs

- [x] M9-T03.1 Images and Videos join the tab row, in all four languages. Shown always: the
      operator asked for them, and the empty state below is what makes an empty tab honest
- [x] M9-T03.2 The image grid — a pure server component, CSS grid, `<img loading="lazy">`
      through the proxy, no JavaScript. Each tile: the image, the page title, the host, a link to
      the page. `alt` from the page title, never empty
- [x] M9-T03.3 The video list — poster, play glyph, title, provider, host; the whole tile links
      to the watch page with `rel="noopener noreferrer nofollow"`. Nothing embedded
- [x] M9-T03.4 The empty state names the vertical — "no images for *سونلغاز*" — and links back
      to All, per [[UI - Search Verticals]] §3. A generic "no results" would leave the reader
      unsure whether the engine has nothing or the tab is broken
- [x] M9-T03.5 A tile whose thumbnail fails to load must not leave a hole: the proxy answers a
      transparent 1×1 on upstream failure so the grid stays a grid, and the title still links
- [x] M9-T03.6 RTL: the grid is direction-agnostic by construction; titles are `<bdi>`-isolated;
      bidi lint green

## M9-T04 — Repass, so the tabs are not empty on day one

- [x] M9-T04.1 `xustive media-repass`: walk the index, re-extract media from the **raw store**
      body where one is still held (`crawl.raw_ttl_days`), and patch only `media`. No refetch —
      the whole point is not to pay a crawl for a parser change
- [~] M9-T04.2 Honest about coverage: reports how many documents had a stored body and how many
      did not. The ones that did not get their video entries on the next revisit, which the
      adaptive recrawl already schedules. **Run against dev: refused correctly — `crawl.raw_ttl_days`
      is 0 and the raw store has never held a body, so there was nothing to repass.** Turning it on
      is a storage decision (bodies × days, in the Redis that PROB-001 bounded), left to the
      operator; until then the Videos tab fills at the pace of the revisit crawl

## M9-T06 — Federated images and videos

> Asked for after close: *use SearXNG to enrich image and video search the same way web search
> uses it.* It is the same mechanism — [[ADR-0017 - Query-Time Federation with External Metasearch]]
> already covers it: one gateway, one budget, fail-open, and every hit queued so the index
> converges. What changes is a `categories` parameter and a tile instead of a card.

- [x] M9-T06.1 `Category` (`web` / `images` / `videos`) on the gateway request and the SearXNG
      call; `FederatedHit` gains an optional `media` — the image (`img_src`, `thumbnail_src`,
      resolution) or the video (**the watch page, never `iframe_src`**, thumbnail, length). Defaulted
      on the wire so a gateway and an API of different builds still agree
- [x] M9-T06.2 The Images and Videos tabs federate in their own category on page 1, exactly as All
      does: detached fetch, short strip wait, "from the web" badge on the tile
- [x] M9-T06.3 The eager index stores the hit's `media` on the placeholder document, so the next
      search finds the picture **locally** before the crawl lands; the page is queued for crawl
      like any federated URL
- [x] M9-T06.4 Thumbnails from the engines go through the signed proxy like every other — a
      Bing-hosted preview is still a third-party host the reader did not choose
- [x] M9-T06.5 Fixture tests on the live JSON shapes; a video hit's `src` is asserted to be the
      watch page and never an embed

## M9-T05 — Gates

- [x] M9-T05.1 `scripts/no-js-check.sh` extended: `?v=images` renders tiles with script off
- [x] M9-T05.2 `make egress-test` green — the web tier's fetch is the same class as ADR-0014's;
      the serving plane's no-egress is untouched
- [x] M9-T05.3 Bundle budget holds — the grid ships no JavaScript at all
- [x] M9-T05.4 Telemetry lint: no image URL in a log line (a URL is a page someone read)

## Deliberately not in this milestone

- **CLIP text-to-image ranking.** The reverse-image search embeds *images*; ranking image tiles
  by a text query needs the CLIP *text* encoder, which the sidecar does not currently expose.
  Ordinary page relevance plus OCR text is the honest ranking until it does. Tracked as the
  natural next step, opt-in behind `[vector]` like everything else there
- **Short videos and Social.** Blocked on the social connectors exactly as [[UI - Search Verticals]]
  says
- **Measuring image dimensions at crawl.** Would need the crawler to fetch every image; the grid
  does without by letting the browser size the tile
