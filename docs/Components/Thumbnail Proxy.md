---
tags:
  - component
  - serving
  - frontend
component-id: C35
binary: web (Next.js route handler)
status: built
updated: 2026-08-27
---

# Thumbnail Proxy

> **ID** C35 · **Runs in** the Next.js server (`web/app/api/thumb`) · **Upstream** crawled hosts,
> Wikimedia, Open Library · **Downstream** `MediaGrid`, the relation row ([[Knowledge Store]])

## 1. Purpose

Show a reader an image from a crawled page without the reader's browser ever contacting that
page's host. Direct `<img src="https://crawled.example/…">` tags would leak the reader's address
and referrer to every site whose picture appears in a grid — the single largest privacy leak an
image tab could introduce. So the browser asks us, we ask the host, and the answer is served from
our origin. The argument in full is [[ADR-0021 - Proxied Thumbnails with Signed URLs]].

The second half of the design is what stops this from being an **open proxy**: only URLs our own
renderer chose will fetch, because the renderer signs each one with a secret the browser never
sees.

## 2. Where it lives today

| Piece | Path |
|:---|:---|
| Route handler | `web/app/api/thumb/route.ts` |
| Signing, verification, host rules (`server-only`) | `web/lib/thumb.ts` |
| Callers | `web/components/search/MediaGrid.tsx` (`signThumb`) |
| Sibling proxy for the knowledge panel | `web/app/api/wiki-image/route.ts` ([[Knowledge Store]]) |

## 3. Interface

```
GET /api/thumb?u=<upstream URL>&s=<HMAC-SHA256(url), base64url>
  403  missing or wrong signature (checked before the URL is even parsed)
  400  URL fails the host rules, on the first request or on any redirect hop
  200  image bytes, Content-Type as upstream, Cache-Control: public, max-age=86400
  200  1×1 transparent GIF, max-age=60, when upstream fails
```

`signThumb(upstream)` returns the same-origin URL for an `<img src>`, or `null` for a URL that
must not be proxied. `verifyThumb` compares in constant time.

A 200 pixel rather than an error on upstream failure is deliberate: a grid with holes reads as
broken, and the tile's title still links to the page.

## 4. Internal Design

### 4.1 The secret

`XUSTIVE_THUMB_SECRET` if set, otherwise 32 random bytes per process. Random is the safe default,
not a weakening: the failure without configuration is "thumbnails signed by one process are
refused by another" — visible, harmless, fixed by setting the variable. A multi-instance
deployment must set it.

The value is held on **`globalThis`**, not at module level. Next compiles server components and
route handlers as separate bundles, each with its own instance of the module, so a module-level
random value is *two* random values and every signature the page produces is one the route
refuses. The process is one process; `globalThis` is what the bundles actually share.

### 4.2 Host rules (`isProxyableUrl`)

`https:` only; no credentials in the URL; no IPv4 literal, no IPv6, no `localhost`, `.local`,
`.internal`, `.lan`, no bare hostnames. A crawled page can carry an `<img>` pointing anywhere,
including inside our own network, and a signature only proves *we* rendered it — it does not make
the destination safe. DNS rebinding is not fully closed by this; ADR-0021 says so.

### 4.3 Unsigned exception for public image hosts

`upload.wikimedia.org`, `commons.wikimedia.org` and `covers.openlibrary.org` are accepted
**without** a signature. They serve only public images, so there is no relay to protect against,
and the relation row's cards are cached by the browser for five minutes: after every deploy those
cached URLs carried the previous process's signatures, and every deploy was five minutes of
broken photos. For those hosts the host itself is the gate; everything else still needs `s`.

### 4.4 The fetch

`redirect: 'manual'` and at most `MAX_REDIRECTS = 4` hops, each re-validated against the host
rules — more necessary here than in `wiki-image` because the origin is the open web. 4 s total
timeout, `MAX_BYTES = 5 MiB` (checked on `Content-Length` and again on the body), `Content-Type`
must start with `image/`. Fixed `User-Agent: XustiveThumb/0.1`. `next: { revalidate: 86400 }`
lets the Next data cache serve one fetch to every reader for a day, keyed by the URL.
Responses carry `Referrer-Policy: no-referrer` and `X-Content-Type-Options: nosniff`.

## 5. Configuration

| Env | Default | Meaning |
|:---|:---|:---|
| `XUSTIVE_THUMB_SECRET` | random per process | HMAC key; required for more than one web instance |

Nothing in `config/*.toml`; this component lives entirely in the web tier.

## 6. Failure Modes

| Failure | Response |
|:---|:---|
| No / bad signature | 403, no fetch |
| Private or non-https destination (any hop) | 400, no fetch |
| Upstream 4xx/5xx, timeout, non-image, too large | transparent pixel, cached 60 s |
| Two web processes with different random secrets | 403s until `XUSTIVE_THUMB_SECRET` is set |

## 7. Security

The proxy is reachable by anyone but useful only to our renderer. The route's own network
position matters: the web tier is the one serving-side process with egress
([[ADR-0014 - Knowledge Panel from Wikipedia via the Web Tier]]), and this handler is one of the
reasons. It never forwards the reader's headers, cookies or referrer upstream.

## 8. Testing

Host rules and signature round-trip are unit-tested in `web/lib`; ADR-0021 lists the manual
checks (unsigned 403, private-host 400, redirect to a private host 400).

## 9. Open Questions

- [ ] Resolve-then-connect pinning to close DNS rebinding properly (ADR-0021).
- [ ] The wiki-image and thumb handlers share most of their logic; fold `wiki-image` into this
      route with its allow-list?

## Related

[[ADR-0021 - Proxied Thumbnails with Signed URLs]] · [[Media Extraction]] · [[Knowledge Store]] ·
[[Web Upstream Client]] · [[Security and Privacy]] · [[Milestone 9 - Images and Videos]]
