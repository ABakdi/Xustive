---
tags:
  - ui
---

# UI — Admin Console

> The operator's whole surface: settings, the crawler, the index, the queue. One application at
> `/admin` with a sidebar, not a page of unrelated forms.
>
> Behaviour and endpoints for the crawler half live in [[Crawler Console]]. This is the interface.

---

## 1. What it is for

One question, asked constantly: **is the crawler actually working, and is it collecting the right
things?**

A document count answers that badly. A count rises identically whether the crawler is finding
Algerian news or four hundred copies of one calendar page, and it keeps rising for a while after
the crawl has gone wrong. So the console has to show *what* is being collected, not only how much —
which means the indexed documents themselves, searchable, filterable, and removable when they are
wrong.

Everything else here — device settings, the politeness bypass, the queue — exists because an
operator who has spotted a problem needs somewhere to act on it without a deploy.

## 2. It stays server-rendered, in the Rust API

Not Next.js. Three reasons, and the first is decisive:

1. **It must work when the frontend is down.** This is the tool for diagnosing a broken system, and
   a diagnostic that shares a failure domain with the thing it diagnoses is not a diagnostic.
2. **Speed.** The requirement is "extremely fast", and the fastest page is one with no bundle to
   download, parse and hydrate. The current admin ships ~4 KB of CSS and ~2 KB of JS. A React admin
   would ship 175 KB before rendering a row.
3. The data lives here. Redis, Meilisearch and the process's own state are all one hop away; going
   through the frontend would add a network hop to every list.

The cost is real and worth naming: no component library, no shared design tokens with the product
UI, and every interactive control written by hand. That is an acceptable trade for a tool used by
one person on a good connection, and a bad trade for the product UI — which is why they differ.

## 3. Shell

```
┌────────────────────────────────────────────────────────────────┐
│ XUSTIVE  admin                              ● crawling  12/min │  status bar
├──────────────┬─────────────────────────────────────────────────┤
│ Overview     │                                                 │
│              │                                                 │
│ CRAWLER      │                                                 │
│  Live        │            section content                      │
│  Documents   │                                                 │
│  Queue       │                                                 │
│  Discovered  │                                                 │
│  Sources     │                                                 │
│              │                                                 │
│ INDEX        │                                                 │
│  Search      │                                                 │
│  Health      │                                                 │
│              │                                                 │
│ SYSTEM       │                                                 │
│  Compute     │                                                 │
│  Politeness  │                                                 │
│  Logs        │                                                 │
└──────────────┴─────────────────────────────────────────────────┘
```

The **status bar is on every section**, not only the live one. It carries crawler state and current
throughput, because the question "is it still running" comes up while you are looking at something
else, and making someone navigate away to answer it is how they stop checking.

Sections are real URLs (`/admin/crawler/live`, `/admin/index/search`) — bookmarkable, linkable in
an incident, and each loads on its own. A single-page app that fetched everything up front would be
slower at exactly the moment it matters, when one subsystem is unwell.

## 4. Sections

### 4.1 Overview

Answers "is anything wrong" in one screen: crawler state, documents indexed today, queue depth,
dead letters, tool-data age, index size. Each tile links to the section that explains it.

Every number that could be *unknown* says so rather than showing zero. A zero and an unreachable
Redis look identical, and the second is the one that needs attention.

### 4.2 Crawler → Live

The real-time view. One SSE stream, one frame per second:

- **Counters**: fetched, parsed, indexed, discovered, failed, skipped — cumulative and per minute.
- **A sparkline** of documents per minute over the last hour. A rate that has quietly gone to zero
  is invisible in a cumulative count and obvious in a line.
- **Per-host activity**: the hosts being worked, their last fetch, their queue depth, their delay.
  A host that stopped answering shows as the row that stopped moving.
- **A rolling feed** of the last ~50 URLs, with outcome. This is the one that tells you the crawler
  is collecting *articles* rather than tag pages, and no aggregate can.
- **Skip reasons**, broken down. "Collecting nothing" has to resolve to *which* rule is eating
  everything.

Start, stop and restart live here.

### 4.3 Crawler → Documents

Everything indexed, newest first, and the section the whole console exists for.

- **Search** across title, URL and body. Backed by Meilisearch, so it is the same engine the
  product uses and it is fast at any corpus size.
- **Filter** by host, source, language, date crawled, and whether it has a date at all.
- **Per row**: title, host, language, crawled-at, and word count. Word count is there because a
  page of navigation and a real article look identical by title.
- **Click through** to the document: extracted text, metadata, outlinks, the raw fetch record.
  Rendered as **text, never HTML** — a crawled page is untrusted input, and an admin console that
  renders it is stored XSS aimed at the most privileged account in the system.
- **Remove**, per document or per selection, with the host and count confirmed first. Removal is
  from the index; the URL goes on a suppression list so the crawler does not simply re-add it on
  the next pass, which would make the button feel broken.
- **Refetch** and **reindex**, distinct — see [[Crawler Console]] §4.3.

### 4.4 Crawler → Queue

What is waiting: total, per host, oldest entry, and what is in flight. Enqueue a URL here,
optionally at the front. Ordering only — a pushed URL passes every check a discovered one does.

### 4.5 Crawler → Discovered

Hosts the crawler has *seen* but is not crawling, because they are off-seed. This is the answer to
"what would it find if I let it", and the place a new source gets promoted into the registry from.

Sorted by how many times each was linked to, since that is the closest thing to evidence that a
site matters.

### 4.6 Crawler → Sources

The seed list with per-source counts, last crawl, error rate and trust tier. A source with a high
error rate is either blocking us or broken, and both are worth seeing before the corpus skews.

### 4.7 Index → Search / Health

Search is the product's own search, run as an operator: same ranking, no personalisation, with the
score and the raw document beside each result. Health covers document counts by language and
source, index size, settings drift, and the last migration.

### 4.8 System

Compute device and the politeness bypass, which exist today, plus a log tail with a level filter.
The log tail never shows query text — that is enforced elsewhere and stated here so nobody adds it.

## 5. Performance

The requirement is that it feels instant, so the budgets are explicit:

| | Budget | How |
|:---|:---|:---|
| First render, any section | < 200 ms | server-rendered HTML, no bundle |
| CSS | ≤ 8 KB | hand-written, one file |
| JS | ≤ 10 KB | SSE, sorting, selection. No framework |
| Document list, 1M docs | < 300 ms | Meilisearch paging, never `SELECT *` |
| Live frame | 1/s | one SSE connection for the whole page |

Rules that keep those true:

- **Paged, never "all".** A list that loads everything is fine at a thousand documents and unusable
  at a million, and the failure arrives exactly when the crawler starts working.
- **One stream.** Every live number on a page comes from the same SSE connection. Several
  connections is several reconnect storms when the API restarts.
- **Absolute values, not deltas.** A dropped frame then costs nothing; with deltas it corrupts the
  count silently until reload.
- **No polling.** A one-second poll for a page nobody is looking at is a request per second forever.

## 6. Interaction rules

- **Destructive actions confirm with a count**: "Remove 412 documents from horizons.dz?" A
  confirmation that does not say how much is about to happen is a rubber stamp.
- **Every action is logged** with the peer that took it.
- **Nothing auto-refreshes under the cursor.** A list that reorders while you are reaching for a
  row is how the wrong document gets deleted; new rows queue behind a "12 new" button.
- **Keyboard**: `/` focuses search, `j`/`k` move rows, `Enter` opens. This is a tool used
  repetitively by one person, which is exactly when shortcuts pay for themselves.

## 7. Not building

| Not building | Why |
|:---|:---|
| Editing document content | The index must reflect what the site published. Wrong extraction is a parser bug; fix it and reindex. |
| Charts beyond the sparkline | Grafana exists and is better at it. This is for acting, not analysing. |
| Multi-user accounts and roles | One operator. Authorisation is the existing admin guard. |
| Public access | Not exposed; same guard as the rest of `/admin`. |

## 8. Open Questions

- [ ] How much history does the document list need before "recent plus search" beats "all, paged"?
      At target volume nobody pages through a million rows, and the paging UI may be dead weight.
- [ ] Should removal suppress the URL permanently, or for a period? Permanent is safer and
      accumulates a list nobody prunes.

## 9. Related

[[Crawler Console]] · [[Admin and Source Submission]] · [[Crawler Orchestrator]] ·
[[UI - Design Language]] · [[Observability]] · [[Security and Privacy]]
