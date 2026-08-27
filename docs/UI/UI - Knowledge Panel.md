---
tags:
  - ui
type: ui
status: built
updated: 2026-08-27
---

# UI - Knowledge Panel

> The right-hand entity panel and the relation row above the results. Both are client components
> fetched after paint, both additive: nothing on the page waits for them, and an empty answer
> collapses to nothing.
> Parent: [[UI Specification]] · Decisions: [[ADR-0014 - Knowledge Panel from Wikipedia via the Web Tier]],
> [[ADR-0019 - The Knowledge Layer]], [[ADR-0021 - Proxied Thumbnails with Signed URLs]] ·
> Milestone: [[Milestone 8 - The Answer Layer]] (T08, T11)

> Written 2026-08-27 from `web/components/search/{EntityPanel,ListPanel}.tsx`,
> `web/lib/relations.ts` and `web/app/api/{knowledge-live,knowledge-list,wiki-image}/route.ts`.

---

## 1. Where they sit

`web/app/[lang]/search/page.tsx` passes two slots to its `Shell`:

- `banner` — `<ListPanel>` when `detectRelation(q)` says the query is relation-shaped. Full width,
  above both columns, across the space the rail would otherwise leave empty.
- `aside` — `<EntityPanel>` in a wrapper `self-start lg:sticky lg:top-20`, the right-hand
  360 px column at `lg`, under the results below that ([[UI - Results Page]] §1).

The server decides cheaply (a handful of regexes) *whether* to mount the row; the cards themselves
arrive after paint. On a relation query the rail is told the **subject** — the film beside its
cast — not whatever "cast of the matrix" happens to resolve to, which is nothing.

---

## 2. `EntityPanel` — `components/search/EntityPanel.tsx`

Props: `q`, `lang`, `t`, `className?`, `subject?` (what to look up instead of `q`), `kinds?`
(what the subject must be — `SUBJECT_KINDS[relation]`, e.g. `['film','series']` for cast).

### Data path: store first, then live

```
knowledgePanel(lookup, lang, kinds)      GET /api/v1/knowledge?q&lang[&kind=…]   (the Rust store; 204 = not an entity)
  └─ null → fetch /api/knowledge-live?q&lang[&kind=…]                            (Next route → Wikidata, web tier)
```

The store answers in a millisecond for anything harvested. The live route runs on the Next server
— the one tier with egress under ADR-0014 — resolves candidates on Wikidata (name pages,
disambiguations and given/family-name items removed; ranked by the API's own resolver), fetches
the reader's-language Wikipedia extract, and hands the raw document to `/api/v1/knowledge/render`
so the panel is built by the **same parser and templates** as a harvested entity. A store miss is
recorded so the harvester catches up and the second hop stops being taken. Both routes answer
`Cache-Control: private, max-age=300`.

`kind=` is a **lift in the resolver, never a filter** — "films by spielberg" prefers the director
over the town, but a query with no candidate of that kind still gets its best entity.

### Following the relation row

The panel listens for the window event `xustive:subject` (`SUBJECT_EVENT`, exported from
`ListPanel`). When the reader picks another part or season in the row, the panel fetches
`/api/knowledge-live?id=Q…&lang` — **by id, no re-resolution, no page load** — and shows it at
once.

### States

`panel` is `undefined` ("still asking") or `null` ("asked, nothing") or a panel — a distinction a
single nullable would lose. `asking` only becomes true inside an effect, so with JavaScript off the
skeleton is **never server-rendered** (M8-T08.5: a permanently loading frame is worse than no rail).

| State | Rendering |
|:---|:---|
| not asking (SSR / no JS) | nothing |
| loading | `PanelSkeleton`: an `<aside aria-busy aria-live="polite" aria-label={t.knowledgeLoading}>` with four `animate-pulse` bars sized like a real panel, plus an `sr-only` label |
| empty | nothing |
| resolved | the panel below |

### Anatomy

```
<aside class="rise rounded-lg border" style="bg: --bg-sunk" aria-label={title}>
  <img src="/api/wiki-image?u=…" alt="" loading="lazy" referrerpolicy="no-referrer">   ← images[0], proxied
  <span chip accent-wash> [kind icon] {t.kind_<kind>} </span>                          ← the kind chip
  <h2 dir=auto>{title}</h2>  <p muted>{description}</p>
  <dl grid "auto 1fr">  <dt>[fact icon] {t.f_<key>}</dt> <dd>value(s)</dd> … </dl>     ← facts
  <p>{blurb.text} <span faint>{t.entityGenerated}</span></p>                           ← only when no extract
  <p>[quote icon] {extract.text}</p>                                                    ← Wikipedia extract
  <ul> <li><a chip> ● IMDb ↗ </a></li> … </ul>                                          ← authorities
  <p muted>{t.entityAlso}: {also.title} — {also.description}</p>                        ← near-tie "did you mean"
  <p faint>[check] {t.entitySources}: wikidata · wikipedia · {image.licence} ↗</p>      ← attribution
</aside>
```

- **Kind chip**: `KIND_ICON` maps `person user · film film · series tv · place pin · organisation
  building · product box · book book · music music · event calendar · species leaf · concept
  bulb`, default `bulb`. Label from `t["kind_"+kind]`, falling back to the raw kind. Accent and
  wash only — the design language allows one colour and this chip is where it goes.
- **Facts**: grouped by key (`groupByKey`) so "Genre: drama, war, historical" is one row, joined by
  `, ` in fr/en and `، ` in ar/ary. A key with **no translated `f_<key>` label is not rendered** —
  a raw machine key would be worse than nothing, and the missing translation is a build-time fix.
  `FACT_ICON` covers ~28 keys (`occupation briefcase`, `birth_date cake`, `death_date cross`,
  `director camera`, `population people`, `area ruler`, `review_score star`, …); a key without one
  shows no icon rather than a wrong one.
- **Values** are typed (`text | entity | number | quantity | score | date`) and formatted in the
  reader's locale with `Intl`. A `date` renders only the precision the publisher asserted — a
  year-precision date is a year. A **score** draws a 48 px accent bar (`aria-hidden`) proportional
  to `value/best` beside `value/best — reviewer`; the reviewer is part of the fact and never
  dropped, because 99/100 from Metacritic means something different from an audience score.
- **Generated blurb**: shown only when there is no encyclopedic extract, always followed by
  `t.entityGenerated` ("(description generated from the data)"). Unlabelled machine prose beside
  human prose is the thing to avoid.
- **Authorities**: one pill per `{key, url}` with a coloured dot (`AUTHORITY_MARK`: IMDb
  `#F5C518`, Rotten Tomatoes `#FA320A`, TMDB `#01B4E4`, Metacritic `#FFCC33`, MusicBrainz
  `#BA478F`, Facebook `#1877F2`, X `#000`), a brand name (`AUTHORITY_NAMES` — brands are not
  translated), and a link icon; `target="_blank" rel="noopener nofollow noreferrer"`. The dot is
  the one place a second colour earns its keep, and it stays a dot, never a surface.
- **Attribution** is not decoration: the claims are CC0, the extract is share-alike, each image
  carries its own licence. Sources are the de-duplicated `provenance.source` of every fact plus
  the extract's; the image credit links to `credit_url`.
- **Image** through `/api/wiki-image?u=` — server-side fetch and stream, so the reader's IP never
  reaches Wikimedia; a plain `<img>` because the host is deliberately not a Next image domain.

---

## 3. `ListPanel` — the relation row (`components/search/ListPanel.tsx`)

Props: `q`, `lang`, `t`.

### Which queries

`lib/relations.ts` — `detectRelation(raw)` — matches four relations in English, French and Arabic
patterns: **cast** (cast of X, X cast, actors in X, casting de X, طاقم X…), **books** (books by X,
livres de X, كتب X…), **films** (films by/with/starring X, filmographie de X, أفلام X…), **albums**
(albums/discography/songs by X, ألبومات X…). Query 4–80 chars, subject 2+ chars and ≤ 8 words.
**The subject is kept exactly as typed**: a first version stripped a leading article and "cast of
the matrix" became a search for "matrix" — the mathematics, not the film. `SUBJECT_KINDS` names
what the subject must be per relation (`cast → film, series`; `books/films → person`; `albums →
person, music`). Precision over recall — a list of the wrong people is worse than no list.

### Data path

`GET /api/knowledge-list?q&lang[&subject=Q…]` (Next route, `Cache-Control: private, max-age=300`):

1. **Subject** — the store (`/api/v1/knowledge`), skipped over if the stored kind is wrong, then
   the live route with `kind=`; or, when `subject=Q…` was picked from "see also", the entity by id.
2. **Members** — one Wikidata SPARQL query (`P161` cast; `P50` author + written-work classes;
   `P57`/`P161` director-or-starring + film classes; `P175` performer + album), 6 s leash. When
   SPARQL stalls (6.8 s for a trivial lookup was observed) the subject's **own claims** answer
   instead: exact for cast (`P161`), approximated by `P800` notable works for the reverse ones —
   shorter than the truth and honest about it. At most 16 cards, in SPARQL's order.
3. **Family** (cast only) — the other parts of the subject's series (`P179` → the series' `P527`)
   or a series' seasons (its own `P527`), at most 10, `null` when fewer than two.
4. **Cards** built three at a time (Wikimedia etiquette; a dozen parallel calls came back as
   connection resets): links by identifier — Wikipedia article (else the Wikidata page), IMDb
   (`nm…` → `/name/`, else `/title/`), Goodreads (`P2969`), Open Library (`P648`), Google Books
   (`ISBN`). Thumbs: the Wikidata `P18` image via `commonsThumbUrl` (the MD5-laid-out
   `upload.wikimedia.org` path, 250 px — one request instead of `Special:FilePath`'s three
   redirects), or for **books with no P18, the Open Library medium cover** (3 s, then no picture
   rather than late); every thumb is signed with `signThumb` and served by `/api/thumb`.

**No ratings.** Goodreads has had no public API since 2020 and forbids scraping, so a Goodreads
rating cannot honestly be shown, and one source's number beside another source's link invites the
wrong reading. A book is its cover, its year and its doors.

### States

| State | Rendering |
|:---|:---|
| not asking (SSR / no JS) | nothing |
| loading | `<section aria-busy aria-live="polite">` sparkle + `t.knowledgeLoading` |
| empty and no family | nothing |
| resolved | below |

### Anatomy

```
<section class="rise mb-8" aria-label={heading}>
  <p> {t.listSeeAlso} · {t.listSeries | t.listSeasons}  [chip aria-pressed]{title} {year} … </p>   ← family chips
  <h2> <span chip accent-wash>[relation icon] {t.listCast|listBooks|listFilms|listAlbums}</span> <bdi>{subject.title}</bdi> </h2>
  <div class="group relative">
    <ul class="list-row" style="scroll-snap-type: x proximity">
      <li class="w-44 shrink-0 rounded-lg border" dir=auto>
        <a href={links[0].url} target=_blank rel="noopener noreferrer nofollow">
          <img src="/api/thumb?u=…&s=…" class="h-44 object-cover">   ← or a relation icon placeholder
          {title} <span muted>{year}</span>
        </a>
        <span class="line-clamp-2">{description}</span>
        <span> [● Wikipedia] [● IMDb] [● Goodreads] [● Open Library] [● Google Books] </span>
      </li> …
    </ul>
    <button aria-label={t.listScrollBack}  class="absolute start-1 …">[chevron-start]</button>   ← only when there is room
    <button aria-label={t.listScrollForward} class="absolute end-1 …">[chevron-end]</button>
  </div>
</section>
```

- **See also · The series / Seasons**: chips (`aria-pressed` on the current one) that call
  `pick(id, title)` — the row refetches with `subject=Q…` and dispatches `xustive:subject` so the
  entity panel swaps too. The family row **survives a pick**: it belongs to the family, not the
  member shown. Reset on a new query.
- **Hidden-scrollbar row**: `.list-row` sets `scrollbar-width: none` and hides the WebKit
  scrollbar; the wheel and a swipe move it. Two arrow buttons sit at the edges, `opacity-0` until
  `group-hover` or `focus-visible`, and only the edge that has somewhere to go is rendered —
  measured with `scrollLeft`/`scrollWidth` in absolute terms because in RTL the position runs
  negative, re-measured on scroll and `ResizeObserver`.
- **Logical start/end**: the buttons are `start-1` / `end-1` with `chevron-start` / `chevron-end`
  `.rtl-flip` icons, and `nudge()` flips the sign under `direction: rtl` — so "forward" is
  leftward in Arabic without a direction-specific rule.
- **Link marks** (`LINK_MARK`): IMDb `#F5C518`, Goodreads `#553B08`, Open Library `#0C7DB0`,
  Google Books `#4285F4`; Wikipedia/Wikidata have no dot. Names from `LINK_NAMES`, untranslated.
- Placeholder when there is no thumb: the relation's icon (`user` for cast, `book`, `film`,
  `music`) at 28 px, faint.

---

## 4. Privacy and egress

The browser talks only to this origin. `/api/knowledge-live` and `/api/knowledge-list` run on the
Next server through one keep-alive `undici` agent (`lib/upstream.ts`: four connections, 5 s
connect timeout, IPv4 only — Wikimedia refused the surplus connections a fresh-TLS-per-call
approach opened, and this host has no working IPv6 route). Upstream hosts are logged on failure;
the query is not. Images go through `/api/wiki-image` (panel) or the signed `/api/thumb` (cards)
so no reader address reaches Wikimedia, Open Library or a crawled host.

---

## 5. Open questions

- [ ] `KnowledgePanel.tsx` + `/api/knowledge` (the Wikipedia-only rail) are still in the tree,
      unmounted since the entity panel folded the extract in. Delete, or keep as the documented
      floor?
- [ ] The relation regexes are en/fr/ar only; Darija forms (e.g. *ممثلين تاع*) are not matched.
- [ ] `related()` (series/seasons) runs for cast only — "films by X" on a franchise director gets
      no family row.
- [ ] Authority and link brand colours are literals in two components; a token map would keep
      them in one place if a third component needs them.

## Related

[[UI - Results Page]] · [[UI - Component Library]] · [[UI - RTL and Localization]] ·
[[UI - Accessibility]] · [[ADR-0014 - Knowledge Panel from Wikipedia via the Web Tier]] ·
[[ADR-0019 - The Knowledge Layer]] · [[ADR-0021 - Proxied Thumbnails with Signed URLs]]
