---
tags:
  - adr
status: accepted, amended by ADR-0029 (the picture may now go to a reverse-image service without identity; words-only stays the default until one is wired)
date: 2026-08-27
updated: 2026-08-27
---
# ADR-0028 - Reverse Image Search Sends Words to the Web, Never the Picture

> Part of [[Decision Log]] · Milestone: [[Milestone 10 - Reverse Image Search]] · Related:
> [[ADR-0008 - No Query Logging]], [[ADR-0017 - Query-Time Federation with External Metasearch]],
> [[ADR-0021 - Proxied Thumbnails with Signed URLs]]

## Context

A reverse image search wants two things the architecture answers differently. *Where does this
picture appear on the Algerian web* is a local question: the index holds the images the crawler
saw, CLIP vectors for them, and a pHash for exact copies. *What else on the web looks like it*
needs the web, and our only door to the web at query time is the federation gateway to a
self-hosted SearXNG ([[ADR-0017 - Query-Time Federation with External Metasearch]]) — which takes
a string of text and nothing else. There is no reverse-image engine behind it, and if there were,
handing a reader's photograph to a third party would be a disclosure [[ADR-0008 - No Query Logging]] exists to prevent. A photograph is the most identifying query a person can make.

## Decision

1. **The picture is read locally and never leaves.** CLIP on our own sidecar embeds it, scores it
   against a reviewable vocabulary of subjects and styles, and OCR reads any text. The bytes live
   on the request's stack and are gone when it returns; nothing is stored, logged or cached by
   image.
2. **The web leg is a text query made of labels.** The top subjects, the style when it is
   telling, and proper names from OCR become one query to SearXNG's image category through the
   gateway, exactly like a typed query. The federator's contract stays `text in`, which is what
   `make egress-test` can prove.
3. **Visual ranking is local only.** Federated hits are shown in the engine's order, thumbnails
   signed and proxied. They are not fetched and embedded at query time — that would put a hundred
   hosts on the reader's critical path. They enter the eager index as every federated hit does,
   the crawler fetches and embeds them, and the next reader asking for that picture gets them
   ranked visually, locally.
4. **Descriptors are computed, not curated per query.** The style chips are whatever styles the
   result set actually contains, counted; the query's own style is named. A vocabulary file,
   not a model per style, so a reviewer can add "calligraphy" without a deploy.

## Consequences

- The web group is only as good as the labels. A picture with no recognisable subject and no
  text gets a weak web query; the page says "from the web, by description" so the reader knows
  what was asked.
- The sidecar gains the CLIP text tower, which also unblocks text-to-image ranking on the Images
  tab — the item [[Milestone 9 - Images and Videos]] parked.
- Two vocabulary files become part of the lexicon review (B7-style), in four languages.
- Face search is still refused; similarity is whole-image, and the dependency lists are audited
  for detectors so it stays that way.
