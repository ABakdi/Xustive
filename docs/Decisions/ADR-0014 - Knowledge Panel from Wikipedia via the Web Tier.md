---
tags: [adr]
adr-id: "0014"
status: accepted
date: 2026-08-20
---
# ADR-0014 - Knowledge Panel from Wikipedia via the Web Tier

## Status

Accepted. Constrains [[UI - Results Page]] (the right rail), [[API Contract]] (it deliberately adds *no* API endpoint), and relates to [[ADR-0001 - Two-Plane Separation]] and [[ADR-0010 - Next.js for the Frontend]].

## Context

A results page should answer an entity query — a person, a place, a thing, a concept — with a compact panel of facts and an image, the way a reader expects from a modern engine. The corpus does not contain encyclopaedic entity records, and building one is out of scope; the pragmatic source is Wikipedia.

The obstacle is [[ADR-0001 - Two-Plane Separation]]: the **serving plane** (`xustive-api`) has no route to the internet, on purpose. An inline API call to Wikipedia on the search path would break that invariant and put a third-party dependency on the latency and privacy of every search. The existing outbound pattern — a `xustive-toold` fetcher on a cadence writing to Redis — fits bounded, enumerable data (weather for 48 wilayas), not the open set of every entity a person might type.

## Decision

**The knowledge panel is a Web-tier feature. The Rust API is not involved.**

- Two Next.js route handlers, server-side: `GET /api/knowledge?q=&lang=` resolves the query against Wikipedia (search → REST summary) and returns `{title, description, extract, thumb, url}` or `204`; `GET /api/wiki-image?u=` proxies the thumbnail. The browser talks only to this origin.
- **Privacy by proxy.** The reader's browser never contacts Wikimedia: the extract is fetched by the Next server, and the image is streamed through `/api/wiki-image`, whose `u` parameter is validated against a fixed host allow-list (`upload.wikimedia.org`) so it cannot become an open proxy or an SSRF vector.
- **Conservative trigger.** A panel appears only for short, entity-shaped queries (≤ 8 words, not a how-to/question) that resolve to a real article (`type: "standard"` with an extract). Everything else returns `204` and the rail stays empty — the panel is absent, never a guess.
- **Client render, after paint.** `KnowledgePanel` fetches after the results render and is silent until it resolves, so it never delays the list and reserves no space it does not fill.
- **Placement.** The panel claims the 260–300 px sticky right rail that [[UI - Results Page]] already specified; on narrow screens it falls below the results.

Two smaller, related changes ship alongside it: the AI **summary now shows a loading state** until it resolves (previously it rendered nothing while generating — [[ADR-0004 - Stream Summary Separately from Results]]), and the **translation tool is registered** and given an editable input, surfacing on an explicit `translate`/`traduire`/`ترجم` verb (its model-quality caveat into Arabic is stated on the card, per the `translator` module docs).

## Consequences

**Good**
- The serving plane keeps its no-internet invariant; the new dependency lives in the tier that already faces the network to serve pages.
- No reader IP or lookup reaches Wikimedia — consistent with the engine's no-logging, `no-referrer` posture.
- No new Rust endpoint, no crawl, no storage: the panel is stateless and cache-fronted (Next `revalidate`).

**Bad**
- The Web tier now makes outbound calls, which a deployment that firewalls it must allow (or the panel simply never appears — it fails closed).
- Entity coverage and text are Wikipedia's, with its biases and gaps; the panel is labelled "From Wikipedia" so this is explicit.
- A per-search Wikipedia round-trip (cached) is added for entity-shaped queries; mitigated by the cheap pre-network gate and Next caching.
