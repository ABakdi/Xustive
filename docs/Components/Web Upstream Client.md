---
tags:
  - component
  - frontend
component-id: C37
binary: web (Next.js server)
status: built
updated: 2026-08-27
---

# Web Upstream Client

> **ID** C37 · **Runs in** the Next.js server · **Upstream** `xustive-api`, Wikidata, Wikipedia,
> Open Library · **Downstream** every server component and route handler in `web/`

## 1. Purpose

How the web tier talks to anything that is not the browser. Two different things share this
note because they are the two halves of one question — *where does a request from the Next.js
server go?* — and because both hold state on `globalThis` for the same reason.

1. The typed client for the Rust API (`web/lib/api.ts`, `web/lib/admin.ts`).
2. The shared keep-alive pool for the external hosts the knowledge routes reach
   (`web/lib/upstream.ts`).

## 2. Where it lives today

| Piece | Path |
|:---|:---|
| Typed client for `/api/v1/*` (search, suggest, summary, knowledge, tools, OCR, image search, transcribe, translate) | `web/lib/api.ts` |
| Admin client for `/api/v1/admin/*` (browser-side, `cache: 'no-store'`) | `web/lib/admin.ts` |
| Shared undici agent | `web/lib/upstream.ts` |
| Browser → API rewrite | `web/next.config.*` `rewrites()` |

## 3. Reaching `xustive-api`

`BASE` is chosen by where the code runs: on the server, `XUSTIVE_API_URL` (default
`http://127.0.0.1:8080`); in the browser, the empty string, so a request to `/api/v1/…` hits the
web origin and Next's rewrite forwards it to `${XUSTIVE_API_URL}/api/v1/:path*`. The browser
therefore never learns the API's address, and the API sees one client — the web tier — for
browser traffic. Every call is `cache: 'no-store'` except two lists that change only on deploy
(`tools()`, `translateLanguages()`), revalidated every 60 s; an hour-long cache once hid a newly
registered translator from the settings page.

The client is hand-written; M1B-T01.3 planned to generate it from an OpenAPI description so a
contract change becomes a compile error. Until then these interfaces *are* the contract
([[API Contract]]) and must be kept honest — two renderers drifting apart was the reason for the
frontend rewrite.

The route handlers under `web/app/api/knowledge-*` also self-call the API (`/knowledge/render`,
`/knowledge/resolve-live`) using the same `XUSTIVE_API_URL`. That self-call deliberately does
**not** go through the shared pool below, because it was seen queueing behind Wikimedia calls and
timing out against its own host.

## 4. The shared upstream agent

One undici `Agent` for every external host the knowledge routes talk to, held on
`globalThis.__xustiveUpstreamAgent`:

| Setting | Value | Why |
|:---|---:|:---|
| `connections` | 4 | Wikimedia asks for a handful per client and enforces it |
| `pipelining` | 1 | |
| `keepAliveTimeout` | 30 s | reuse instead of a fresh TLS handshake per call |
| `connect.timeout` | 5 s | a refused connection becomes a fast failure, not a 10 s stall |
| `connect.family` | 4 | the host's resolver returns an IPv6 address for Wikimedia it cannot route |

Without it every call opened a fresh TLS connection and Wikimedia began refusing the surplus —
`UND_ERR_CONNECT_TIMEOUT` in the server log while `curl` on the same host connected in under a
second. `viaUpstream()` returns `{ dispatcher }` to spread into any `fetch` init;
`knowledge-list` applies it only to `wikidata.org`, `wikipedia.org` and `openlibrary.org`.

**Why `globalThis`.** Next compiles each route into its own bundle with its own module instances.
A module-level pool is one pool per bundle, which defeats the point; the process is one process
and `globalThis` is what the bundles share. `thumb.ts` holds its secret the same way and for the
same reason ([[Thumbnail Proxy]]).

## 5. Configuration

| Env | Default | Used by |
|:---|:---|:---|
| `XUSTIVE_API_URL` | `http://127.0.0.1:8080` | `api.ts`, the rewrite, knowledge route handlers |
| `XUSTIVE_THUMB_SECRET` | random | [[Thumbnail Proxy]] |

## 6. Failure Modes

| Failure | Response |
|:---|:---|
| API down (server render) | the page's error shell ([[UI - States and Errors]]) |
| API down (browser fetch) | the rewrite returns the failure; components collapse or show retry |
| External host slow | per-route `AbortController` leash (6–12 s) on top of the 5 s connect timeout |
| Pool exhausted | calls queue behind the four connections rather than being refused upstream |

## 7. Security

The pool is used only for the fixed hosts the knowledge routes name; it is not a general egress
path. Nothing here forwards a reader's headers. The server-only modules (`upstream.ts`,
`thumb.ts`) import `server-only` so a client-component import fails the build instead of shipping
a secret or a Node dependency to the browser.

## 8. Open Questions

- [ ] Generate `api.ts` from the API's OpenAPI description (M1B-T01.3, still open).
- [ ] `connect.family: 4` is a workaround for one host's routing; revisit when IPv6 works.

## Related

[[UI - Frontend Architecture]] · [[API Contract]] · [[Knowledge Store]] · [[Thumbnail Proxy]] ·
[[API Gateway]]
