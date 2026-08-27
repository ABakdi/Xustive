---
tags:
  - adr
  - privacy
  - data
status: accepted
date: 2026-08-27
updated: 2026-08-27
follows: ADR-0029
---
# ADR-0030 - First-Party Search Data, Kept to Learn From

> Part of [[Decision Log]] · Follows [[ADR-0029 - Raw Queries May Leave, Identities Never;
> First-Party Data Comes Later]] · Supersedes the storage half of [[ADR-0008 - No Query
> Logging]] and [[ADR-0018 - Anonymous Search History]] · Milestone: [[Milestone 11 - Learning
> from Readers]] · Audit items: [[Privacy Relaxation Audit]] PRIV-006/007/008/013

## Context

ADR-0029 decided that the product will keep user data for its own AI and for improving and
personalising search, and deferred the how. Every improvement the [[Privacy Relaxation Audit]]
ranks highest — spelling, autocomplete and synonyms from what people type, evaluation on real
traffic, a ranking that learns from clicks, personalisation — needs the same three things and
has none of them: **what was searched**, **what was shown**, and **what was opened**, per event,
durable, and joinable. Today the closest thing is a k-anonymous counter in an ephemeral Redis
that forgets the query text on purpose.

One more signal is missing entirely: readers have no way to say *this result is wrong*. Clicks
say what looked good; nothing says what was bad, and a ranker trained on clicks alone learns to
be clicked, not to be right.

## Decision

1. **Every search and every hit is an event, kept.** A `search` event: when, a visitor id, a
   session id, the query as typed and as normalised, the interface language, the vertical, the
   page, how many results there were, the ids of the results shown in order, and the latency. A
   `click` event: when, visitor, session, the query, the document opened, its rank. A `report`
   event: the same, with the reader's reason — *not relevant*, for now. Events live in their
   own Meilisearch index (`events`, [[ADR-0002 - Meilisearch as System of Record]]), searchable
   by query and filterable by everything else, so the operator can look and the learners can
   read.
2. **Documents remember being opened.** Each document carries `hits { opens, reports,
   last_opened_at }`, updated on the event — the record the operator asked for on the document
   itself, and the cheapest possible ranking feature.
3. **Readers can report a result as irrelevant**, from the result card, one tap; it records a
   `report` event and counts against the document. What is done with reports is a ranking
   decision for later; the signal is collected now.
4. **The visitor id is a first-party cookie**, random, one year, set by our page and read only
   by our servers; the session id is a session cookie. Neither is sent to any third party
   (ADR-0029 rule 2 stands). No account yet; when accounts come, they bind to the visitor id.
5. **What is still never kept.** The reader's IP address, user agent or device details are not
   in any event. The query text still never appears in logs or metrics
   (`scripts/lint-telemetry.sh` stands); the events index is a store, not a log, and is read
   through the admin API and the learners only.
6. **Retention and rights.** Events are kept `collection.retention_days` (default 365) and
   swept; a reader may ask for deletion by visitor id, which removes every event carrying it
   (`xustive events forget <visitor>`). The privacy page names all of this — what is kept, for
   what, for how long, and how to be forgotten — in the same change that starts keeping it.
7. **Off by default, on in dev.** `[collection] enabled = false` unless the operator turns it
   on; no k floor, because this is first-party data under the operator's own lawful basis.

## Consequences

- [[Legal and Compliance]] §5: the operator becomes a controller of personal data under Law
  18-07 (a query can name a person; a visitor id is an identifier). Lawful basis, ANPDP
  registration and the retention schedule are owed before a public deployment turns this on;
  the dev box may run it now. This ADR fixes the technical facts the lawyer needs.
- The interaction signals of [[ADR-0015 - Anonymous Interaction Signals for Ranking]] keep
  working unchanged beside this — they are derived, k-anonymous, and feed the ranker today; the
  events are the raw material for the next ranker. When a learner replaces the CTR term, ADR-0015
  gets its supersession note.
- Autocomplete, synonyms and evaluation ([[Privacy Relaxation Audit]] PRIV-006/007/008) now have
  a source; each is its own task and is not built by this ADR.
- The index grows with traffic: one search event is ~1 KB; a million searches a month is a
  gigabyte a month before retention. The sweep and the retention default exist for that.
