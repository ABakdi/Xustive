---
tags:
  - decision
  - ui
status: accepted
date: 2026-08-07
supersedes: "server-rendered HTML from xustive-api"
---

# ADR-0010 — Next.js for the Frontend

## Context

The UI is server-rendered HTML written by hand in `xustive-api`, with a small progressive-
enhancement layer in `web/public/app.js`. It works, it is fast, and it needs no build step. It is
also at its ceiling:

- Every component exists **twice** — once in Rust `format!` strings, once in JavaScript. The
  filter chips, the suggestion list and the summary block were each written two ways, and the
  language filter shipped broken on the server path precisely because the two drifted.
- There is no component model, so there is no way to build [[UI - Component Library]] as
  specified.
- Instant-answer tools ([[Instant Answers]]) are interactive by nature — a unit converter with
  two dropdowns and a live result is not a `format!` string.

## Decision

**Next.js 15, App Router, TypeScript, React Server Components.** The Rust API becomes purely
JSON; all HTML comes from Next.

Component library is **shadcn/ui**, which is source-copied rather than depended on, so the visual
identity in [[UI - Design Language]] is a rewrite of the primitives rather than a theme layered
over someone else's product.

## Why Next rather than a single-page React app

The results page **must** render on the server. That is not a preference:

- A meaningful share of Algerian traffic is on connections where a 200 KB JavaScript bundle
  parsing before any content appears is the difference between a usable engine and a blank
  screen.
- Search results must be crawlable and linkable.
- The no-JavaScript path is a stated commitment ([[UI - Results Page]]), and today it is the
  only path that is guaranteed to work.

A client-rendered SPA gives all of that up. React Server Components let the results page arrive
as HTML while the interactive parts — suggestions, filters, tool cards — hydrate independently.

## What this costs, honestly

**A Node process in production.** The serving plane was two binaries and a search engine; it is
now three processes. Roughly 120 MB resident and one more thing that can fall over.

**A second network hop.** Browser → Next → `xustive-api` → Meilisearch. Measured budget impact is
15–25 ms server-side, which comes out of the 1500 ms search budget in [[Performance Budgets]].
Acceptable, and partly recovered because Next and the API are colocated while the browser round
trip they replace was not.

**A build step.** `make up` grows a `pnpm build`. The Rust-only workflow was a genuine pleasure
and we are giving it up.

**Two languages in the serving path.** A contributor now needs Rust and TypeScript to change a
feature end to end.

These are real. The alternative — hand-writing an interactive translator, a weather forecast and
a unit converter as Rust string templates — is worse, and the duplication bug in the language
filter is a preview of how that ends.

## Rejected alternatives

| Option | Why not |
|:---|:---|
| **Keep Rust SSR, add React islands** | Preserves the single binary, but the duplication stays: an island still needs its markup written twice for the no-JS path. Solves the smallest part of the problem. |
| **Next.js static export (SSG)** | No Node in production, which is genuinely attractive. But search results cannot be statically generated, so the results page — the whole product — would become client-rendered. Exactly the thing being avoided. |
| **Astro** | Islands with less runtime than Next, and a serious contender. Rejected because the tool cards in [[Instant Answers]] are stateful and interconnected enough that a full React model earns its cost, and because shadcn/ui targets React. |
| **SvelteKit** | Smaller bundles, and on merit a close call. Rejected on ecosystem: shadcn/ui, `react-aria` and the RTL-tested component landscape are React-first, and RTL correctness is not a place to be the first person trying something. |

## Consequences

- `xustive-api` stops rendering HTML. `web.rs` is deleted, not maintained in parallel — two
  renderers is the problem being solved, and keeping one "just in case" recreates it.
- `xustive-api` keeps serving `/healthz`, `/readyz` and `/metrics` on its own port. The Node
  process must not sit in front of liveness checks for a service it depends on.
- The API gains a stable JSON contract obligation it did not have when it was its own only
  consumer. [[API Contract]] becomes load-bearing.
- The no-JavaScript path is now Next's SSR output plus real `<form>` elements — which is a
  stronger guarantee than today, since it is the same markup everyone gets rather than a
  separate code path that can silently rot.

## Related

[[UI - Design Language]] · [[Instant Answers]] · [[UI Specification]] · [[API Contract]] ·
[[Performance Budgets]] · [[Deployment Topology]] · [[ADR-0002 - Meilisearch as System of Record]]
