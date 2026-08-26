---
tags: [adr]
adr-id: "0021"
status: accepted
date: 2026-08-26
---
# ADR-0021 - Proxied Thumbnails with Signed URLs

## Status

Accepted. Constrains [[UI - Search Verticals]], [[UI - Results Page]] and
[[Security and Privacy]]. Generalises the image proxy of
[[ADR-0014 - Knowledge Panel from Wikipedia via the Web Tier]] from a fixed host allowlist to the
open web, without becoming an open proxy. Does not amend [[ADR-0001 - Two-Plane Architecture]]:
the serving plane's no-egress property is untouched, because the fetch happens in the web tier
exactly as ADR-0014's does.

## Context

An Images tab is a grid of forty pictures from forty sites we crawled. The obvious way to render
it — `<img src="https://crawled-host/…">` — sends the reader's address, and a referrer, to every
one of those hosts on page load. The reader chose to search; they did not choose to visit forty
sites, and [[Security and Privacy]] P2 exists so that a search is not a disclosure.

ADR-0014 solved this for one source by proxying through the web tier with a **fixed host
allowlist** (`upload.wikimedia.org`, later `commons.wikimedia.org`). That is the right shape and
the wrong scope: an allowlist cannot enumerate the open web, and a proxy that accepts any URL is
an open proxy — anyone can make our server fetch anything, which is both an SSRF vector and a
free bandwidth relay.

The observation that resolves it: **the server already decides which thumbnails to render.** The
results page is a server component. It knows every URL it puts on the page, and it can vouch for
each one in a way the browser cannot forge.

## Decision

**Thumbnails are served through a same-origin proxy that accepts only URLs the rendering server
signed. The signature is an HMAC over the URL with a secret the browser never sees. Everything
else about the proxy is the ADR-0014 discipline, plus the guards an open-web source needs.**

1. **Signed, not allowlisted.** `/api/thumb?u=<url>&s=<hmac>`. The page signs; the route
   verifies; an unsigned or mis-signed request is refused before any fetch. The proxy therefore
   fetches only what our own render chose to show — which is what "not an open proxy" means.
2. **The secret is server-side and optional.** `XUSTIVE_THUMB_SECRET` when set; a per-process
   random value when not. Missing configuration degrades to *thumbnails break across restarts*,
   never to *anyone can use the proxy*.
3. **The guards ADR-0014 has, and two it did not need.** `https` only; `image/*` only; 5 MB cap
   checked on the header and again on the body; a short timeout; `Referrer-Policy: no-referrer`
   on the response. New for the open web: **no IP-literal and no private hostnames**, and
   **redirects followed by hand with every hop re-validated** — a crawled page can carry an
   `<img>` pointing anywhere, including inside our own network.
4. **Cached, by URL.** The same thumbnail for every reader is one fetch from the crawled host —
   which is also the polite thing to do to a site that did not ask to be a CDN.
5. **A failed upstream is a placeholder, not a hole.** The route answers a transparent pixel when
   the host is down or the file is not an image, so the grid stays a grid and the title still
   links.
6. **Video is never fetched and never embedded.** A video tile shows a poster (proxied like any
   image) and links out. No player is embedded — an embedded player is a third-party page load —
   and no code path downloads video bytes, which [[Milestone 2 - Ingestion at Scale|M2-T10.8]]
   already required.

## Consequences

**Good**

- The reader's browser talks only to us. Forty images from forty hosts is zero disclosures.
- No open proxy: the signature makes the route useless to anyone but our own renderer.
- No amendment to ADR-0001. The web tier's fetch is the same class as ADR-0014's, already
  accounted for in the egress test.
- Politeness falls out of caching: one fetch per thumbnail per day, however many readers.

**Bad**

- The web tier does the fetching, and forty thumbnails is forty upstream requests on a cold
  cache. Mitigated by the cache and by lazy loading; a very popular query warms quickly.
- A crawled host can be slow or hostile. The timeout and size cap bound the cost per tile, and
  the placeholder bounds the visible damage, but a page of slow hosts is a slow grid.
- DNS rebinding is not fully closed by a hostname check. The hop-by-hop re-validation and the
  short timeout narrow it; a resolver-level check would close it and is the revisit trigger.
- A per-process fallback secret means a multi-instance web tier **must** set the shared secret,
  or thumbnails signed by one instance fail on another. Documented in the deployment notes.

## Alternatives

| Option | Why not |
|:---|:---|
| Hotlink the images | Forty disclosures per page. The exact thing P2 forbids. |
| Extend ADR-0014's host allowlist | Cannot enumerate the open web; an allowlist that grows per host is a proxy that is open in practice and closed on paper. |
| Store thumbnails at crawl time on the ingestion plane | The right long-term answer for size and speed, and a large storage feature — image fetching, resizing, a blob store, retention. Not needed to ship a grid, and the signed proxy does not preclude it. Revisit trigger below. |
| Verify the URL exists in the index instead of signing | A Meilisearch lookup per thumbnail — forty per page — to answer a question the renderer already knew. Signing is the same guarantee for a hash. |
| Embed video players | Hands the reader to a third party on page load, before they chose anything. |

## Revisit when

- Thumbnail traffic on the web tier becomes a measurable share of its load, at which point
  crawl-time thumbnailing on the ingestion plane earns its storage.
- A multi-instance web tier is deployed — the shared secret becomes mandatory, not advisory.
- A resolver-level private-range check becomes cheap to add in the web tier, closing the
  rebinding gap the hostname check leaves.
