---
tags:
  - adr
  - privacy
status: accepted
date: 2026-08-27
updated: 2026-08-27
amends: ADR-0008, ADR-0018, ADR-0020, ADR-0028
---
# ADR-0029 - Raw Queries May Leave, Identities Never; First-Party Data Comes Later

> Part of [[Decision Log]] · Amends [[ADR-0008 - No Query Logging]],
> [[ADR-0018 - Anonymous Search History]], [[ADR-0020 - Approximate Location from a Local
> Database]], [[ADR-0028 - Reverse Image Search Sends Words to the Web, Never the Picture]] ·
> Audit of what this unblocks: [[Privacy Relaxation Audit]]

## Context

The privacy posture this system was built under — [[ADR-0008 - No Query Logging]] and the rules
that followed it — was a strength on day one and has become the limiting factor on day
twenty-one. It kept every model local (a 3B summariser under a non-commercial licence, a Whisper
that runs on a shared 4 GB card, a CLIP that cannot see the web), kept every third party out of
the query path except a self-hosted metasearch engine, kept the reverse image search from ever
showing the web a picture, and kept the product from learning anything from the people using it:
no query history, no synonyms mined from what Algerians actually type, no "did you mean", no
personalisation, no evaluation on real queries — only k-anonymous counts, off by default.

The operator has decided that the line was drawn in the wrong place. What must never leave is the
**person**; what may leave is the **question**.

## Decision

1. **Raw query data may be sent to third-party services.** The text of a query, an uploaded
   image, a voice recording, the words of a page — any of it may go to an external API (a hosted
   model, a metasearch or reverse-image service, a knowledge or ratings API) when it makes the
   answer better. This supersedes the parts of ADR-0008 and ADR-0028 that forbade it.
2. **Never with an identity.** No request to a third party carries the reader's IP address,
   device or browser identifiers, a session or account identifier, precise location, or any
   combination of fields that could be traced back to a person. Third-party calls go through our
   own servers, from our own addresses, with our own credentials. The parts of ADR-0008 that
   protect the *person* — no IP in any store, the salted-hash rate limiter, the thumbnail proxy —
   stand.
3. **The privacy policy says so, plainly.** It names that queries, images and recordings may be
   processed by third parties on our behalf, that we send them without identifying data, and that
   **despite our best efforts some data may leak** — a third party is a party we do not control.
   Honesty about the risk is the policy; a policy that promises what it cannot enforce is not.
4. **First-party collection begins, and it is never shared.** The product will start keeping
   user data — including identifiable data, where a reader is signed in or consents — for its own
   AI systems and for improving and personalising search results. It is used here and never sold,
   shared or sent outside. *This part is decided now and built later*; its design (what is
   collected, the lawful basis under Law 18-07, consent, retention, deletion on request, access)
   gets its own ADR before a byte is stored.
5. **Local stays the default where local is good enough.** Nothing about this decision requires
   a third party; it permits one. A local path that meets the bar stays local — it is cheaper, it
   works offline, and it leaks nothing at all.

## Consequences

- The build's own enforcement changes shape, not purpose: `scripts/lint-telemetry.sh` keeps
  forbidding query text in **logs and metrics** (rule 2 needs it more than ever — a log line
  with a query beside a peer address is exactly the combination that identifies), while
  query-keyed **stores and caches** become permitted once rule 4's ADR says how they are kept.
  `scripts/test-egress.sh` must learn the allow-list of third-party hosts rather than asserting
  "gateway and nothing else".
- The serving plane's "no egress" rule ([[ADR-0001 - Two-Plane Architecture]]) is relaxed to
  "egress only through named clients to named hosts, with identity stripped" — one place to
  audit, not zero.
- [[Legal and Compliance]] §5 (Law 18-07) becomes live rather than hypothetical: sending queries
  abroad and keeping identifiable data both need a lawful basis, a controller, and a retention
  schedule. The policy text in rule 3 is the first visible artefact; the register that follows
  this ADR lists the rest.
- The README's "Guarantees enforced by the build" and the privacy page must be rewritten in the
  same change that first sends anything out; a page that still says "nothing leaves the machine"
  after this decision would be a lie the build could not catch.
- What this buys is listed, with evidence and cost, in [[Privacy Relaxation Audit]]: better
  summaries and translation from hosted models, a reverse image search that can show the web the
  picture, discovery driven by real queries, synonyms and spelling from what people type,
  evaluation on real traffic, and — later — personalisation.
