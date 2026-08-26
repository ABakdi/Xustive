---
tags:
  - planning
  - milestone
milestone: 8
status: planned
updated: 2026-08-26
---
# Milestone 8 - The Answer Layer

> **Goal:** answer the query on the page. Search returns links; this milestone returns the thing
> the person actually wanted — who someone is, what a film scored, what the weather will do
> tomorrow, what 20 EUR is in dinars — beside the links rather than instead of them.
> **Exit gate:** for the thousand most-searched entities, a panel renders from local storage at
> p95 ≤ 100 ms with **zero serving-plane egress**; weather answers for an unnamed location without
> the client's IP being stored anywhere; `20 eur to dzd` answers with a dated rate or declines;
> `make egress-test` and the telemetry lint stay green.
> Parent: [[TODO]] · Previous: [[Milestone 7 - Federated Retrieval and External Tools]] ·
> Governed by [[ADR-0019 - The Knowledge Layer]] and [[ADR-0020 - Approximate Location from a Local Database]] ·
> Components: [[Instant Answers]], [[Tool Data Plane]], [[Knowledge Layer]]

## Why This Milestone Exists

[[Milestone 1B - Frontend and Instant Answers]] said it first and it is still true: *"Someone using
Google will not move for slightly better Algerian results. They will move for a product that
answers `20 eur dzd` with both rates, translates without sending the text anywhere, and knows what
a qintar is. The tools are the reason to exist; the search is the foundation under them."*

The foundation is now built. [[Milestone 7 - Federated Retrieval and External Tools]] closed with
retrieval quality settled, the frontier bounded, and the console honest. What is conspicuously
unfinished is the half a person actually looks at:

- **Currency does not exist.** [[Instant Answers]] §5.1 calls it Tier 1 and §7 calls it the single
  most Algeria-specific thing the product does. There is no `currency.rs`, no rates dataset, and no
  slot in the tool registry. Someone typing `20 eur dzd` today gets ten blue links.
- **The five-day forecast is computed, serialised, and never drawn.** `weather.rs` puts
  `detail.days` on the wire; `ToolCard.tsx` renders `value` and stops. The data has been crossing
  the network unused. There is no hourly series, and no way to answer *"weather"* with no place in
  it — the most common way anyone asks.
- **The knowledge panel knows one authority.** [[ADR-0014 - Knowledge Panel from Wikipedia via the Web Tier]]
  ships a Wikipedia extract and a thumbnail. Ask about a film and you get a paragraph, not the year,
  the director, the runtime, and what it scored. Ask about a person and you get prose where a face,
  a birth date and an occupation belong.
- **The calculator stops at arithmetic.** Precise decimal arithmetic, seven unit dimensions, and no
  bridge between them: `5 km + 3 miles` is not a question it can answer, and `20 eur + 5 usd in dzd`
  is not a question anyone can ask.

This milestone finishes the answer layer. It adds one genuinely new subsystem — a **knowledge
layer** that resolves a query to an entity and describes it from aggregated authorities — and
completes three tools that were specified, half-built, and left.

## The shape of the thing

A request from the operator that framed this work: *do everything normally, then hand the query to
the AI, let it decide which authorities to pull from, run those separately while a placeholder sits
in front of the reader, and fill it in a couple of seconds later.*

The asynchronous shape is exactly right and this milestone builds it: the panel never blocks the
results, it announces itself while it loads, and it arrives when it arrives. Two things change on
the way to implementation, and both change because of what the architecture already guarantees.

**The serving plane cannot fetch, so the fetching moves in time rather than in place.**
[[ADR-0001 - Two-Plane Architecture]] gives `xustive-api` no route to the internet, and
[[ADR-0008 - No Query Logging]] forbids a cache keyed by a query — *"a query log with extra
steps."* A per-search fan-out to IMDb and friends is therefore not available and would not be
wanted: it would be slow, it would be rate-limited, it would break for every user at once when a
third party had a bad day, and it would put a live query in front of five companies. What replaces
it is the pattern [[Tool Data Plane]] already sets out — **fetch on a schedule into a store the
serving plane reads.** The unit of caching is the *entity*, not the query, which is both permitted
and better: `Q42` is enumerable, shareable across every person who asks, and reveals nothing about
who asked.

**The router is a lookup before it is a model.** Which authorities describe a film is not a
judgement call — it follows from the entity's type, and the type is a fact we store. Wikidata's
*instance of* says `film`, and `film` selects a fixed authority set. Spending a language model on
that would add a second of latency and a GPU slot to reproduce a table lookup, on a
[[Hardware Profile|4 GB Quadro]] that has better things to do. The model earns its place where
there is genuine ambiguity — *which* `Q` a bare name means when the store holds six candidates, and
writing a readable line when no encyclopedic extract exists — and both of those results are cached
against the entity, so the model runs once per entity rather than once per search.

The visible behaviour is what was asked for. The mechanism is a lookup with a model in the two
places a model is actually better than a table.

---

## M8-T01 — The knowledge store

> The spine. Everything else in T02-T04 reads from here, so it lands first.

- [ ] M8-T01.1 `xustive-knowledge` crate: `Entity { id, kind, names, description, claims,
      authorities, images, updated_at }`, where `id` is a Wikidata QID and `kind` is a closed enum
      resolved from *instance of* (`P31`). A closed enum, not a free string: the panel template is
      chosen by exhaustive match, so a new kind is a compile error rather than a blank card
- [ ] M8-T01.2 Harvest on the ingestion plane — a new `xustive-knowledged` (or a `Dataset` in
      [[Tool Data Plane|xustive-toold]], whichever keeps `toold` free of its current single-dataset
      assumption): Wikidata entity data + Wikipedia extract + Commons image reference, for a seed
      set of Algeria-relevant and globally notable entities. Fixed cadence, never per-request,
      never any user input — the `toold` contract, unchanged
- [ ] M8-T01.3 Storage: a **Meilisearch index**, not Redis. Resolution is a name lookup with
      aliases, transliteration and typo tolerance across four scripts — which is a search problem,
      and we already run a search engine that is good at it. Redis holds no entity text
- [ ] M8-T01.4 Multilingual by construction: `names` carries `ar`, `ary`, `fr`, `en` labels and
      aliases from Wikidata, so `سبيلبرغ`, `Spielberg` and `spielberg` resolve to one entity.
      Darija falls back to Arabic, never English — the [[Milestone 1B - Frontend and Instant Answers|M1B-T08.4]] rule
- [ ] M8-T01.5 Licensing carried per field, not per entity: Wikidata claims are CC0, Wikipedia
      extracts are CC BY-SA, Commons images each carry their own licence and author. The panel
      renders the attribution the licence requires, so a licence that changes cannot silently become
      an unattributed reproduction
- [ ] M8-T01.6 Refresh policy: an entity is re-harvested on a schedule proportional to how often it
      is asked for, floored so nothing goes stale forever. Dead entities age out
- [ ] M8-T01.7 `xustive_data_age_seconds{dataset="knowledge"}` — which first requires
      `dataage.rs::sample()` to loop over datasets instead of hard-coding weather

## M8-T02 — Resolution: query → entity

- [ ] M8-T02.1 A resolver that is *quiet by default*. Precision over recall: a panel on the wrong
      entity is worse than no panel, and far worse than it looks — it is a confident wrong answer
      in the position readers trust most
- [ ] M8-T02.2 Gate before resolving, reusing the shape `web/app/api/knowledge/route.ts` already
      proved: length bounds, word count, no question markers in any of the four languages. A
      question is for the summariser; a noun phrase is for the panel
- [ ] M8-T02.3 Score candidates on exact-alias match, script match, entity prominence, and
      agreement with the local corpus — a name the crawled Algerian web talks about outranks a
      same-named entity it has never mentioned
- [ ] M8-T02.4 Ambiguity is surfaced, not guessed: when the top two candidates are close, the panel
      shows the leader with a "did you mean" line for the runner-up rather than silently picking
- [ ] M8-T02.5 Below the confidence floor, render nothing at all. Assert it: a precision corpus of
      ordinary queries that must resolve to no entity, alongside per-kind positive cases — the
      [[Milestone 1B - Frontend and Instant Answers|M1B-T04.6]] matcher-corpus discipline applied to entities

## M8-T03 — Authorities and what each kind shows

> The aggregation the operator asked for: a film shows IMDb and Rotten Tomatoes; a person shows a
> face and the facts. The mechanism is that **Wikidata already stores the cross-references** —
> `P345` is the IMDb id, `P1258` the Rotten Tomatoes id, `P4947` TMDB, `P1712` Metacritic, and
> `P444` carries review scores with the reviewer named in `P447`. We read the identifiers from a
> CC0 source and build the links from them. We never scrape IMDb or Rotten Tomatoes: their terms
> forbid it, and [[ADR-0013 - Direct SERP Collection for Discovery]] confined that whole class of
> behaviour to the ingestion plane for discovery only.

- [ ] M8-T03.1 A `Kind → template` table, exhaustive over the enum, each naming the facts to show
      and the authorities to link
- [ ] M8-T03.2 **Film / series** — year, director, runtime, genres, cast head; scores from `P444`
      with each reviewer named and dated; links out to IMDb, Rotten Tomatoes, TMDB built from the
      stored ids
- [ ] M8-T03.3 **Person** — image, birth and death, nationality, occupation, notable works,
      positions held; for footballers and public figures the club or office, because that is what
      the question usually means
- [ ] M8-T03.4 **Place** — image, wilaya or country, population, coordinates, and a link into the
      local corpus for what the Algerian web says about it. Algerian places also carry the wilaya
      code, which [[Milestone 1B - Frontend and Instant Answers|M1B-T07.1]] already compiles in
- [ ] M8-T03.5 **Organisation, product, work (book/album/song), event, species, concept** — one
      template each, each honest about having fewer facts than a film
- [ ] M8-T03.6 **Concept fallback** — when the kind is unknown or has no template, render the
      description and extract, which is exactly today's Wikipedia panel. The floor never gets worse
      than what ships now
- [ ] M8-T03.7 Every fact carries its source and every panel names them. A fact with no attributable
      source is not shown — the [[Instant Answers]] §2 rule, extended from tools to entities
- [ ] M8-T03.8 Images: proxied same-origin through the existing allowlisted route, `<img>` not
      `next/image`, licence and author rendered beneath. Extend the allowlist deliberately, one
      host at a time, in the ADR

## M8-T04 — The model's two jobs

- [ ] M8-T04.1 **Disambiguation** — when T02 leaves a close call the local summariser is asked to
      choose, given only the candidate labels and descriptions. Bounded, optional, and skipped
      entirely when the model is unavailable; the deterministic leader ships instead
- [ ] M8-T04.2 **Blurb** — for an entity with facts but no encyclopedic extract, one or two
      sentences composed from the stored claims only. Grounded in the same way summaries are:
      nothing in the text that is not in the claims, validated before it is stored
- [ ] M8-T04.3 Both results cached **against the entity id**, never the query. The model runs once
      per entity in its lifetime, not once per search — which is what makes this affordable on the
      target hardware
- [ ] M8-T04.4 Off is a first-class state. `knowledge.model_assist = false` by default; the panel is
      fully useful without it, and every test runs both ways

## M8-T05 — Weather, finished

- [ ] M8-T05.1 **Approximate location without asking, without storing** — the client address is
      resolved against a local database in-process, mapped to the nearest wilaya seat, and dropped.
      Never logged, never cached, never sent anywhere. Governed by
      [[ADR-0020 - Approximate Location from a Local Database]]; the discipline is the one
      `ratelimit.rs` already applies to the same value
- [ ] M8-T05.2 Extend the fetcher to hourly (48 h) and seven days, with the existing validation,
      movement guard and partial-write refusal. Cadence and cache key version bump together
- [ ] M8-T05.3 Render the forecast that has been on the wire since M1B: today in full, a day strip,
      and a **week** toggle
- [ ] M8-T05.4 Graphs — temperature and precipitation, drawn as **server-rendered SVG**. A canvas
      chart library would fail the no-JS path and cost more than the whole page budget; an inline
      SVG polyline costs bytes we can count
- [ ] M8-T05.5 Named places keep working and gain the same depth: `weather oran`, `طقس وهران`,
      `météo à Alger`
- [ ] M8-T05.6 A location we cannot place falls back to the largest nearby wilaya and **says so**.
      A wrong city stated confidently is the failure mode to avoid
- [ ] M8-T05.7 Custom line icons per WMO code, the [[Milestone 1B - Frontend and Instant Answers|M1B-T05.5]] loose end

## M8-T06 — Currency and rates

- [ ] M8-T06.1 A `rates` dataset in the tool data plane: ECB reference rates for the majors, plus a
      DZD-carrying source, fetched on a fixed daily cadence with a 48 h staleness limit. Both
      candidates are free, keyless, and self-hostable, which is what makes this survive traffic
- [ ] M8-T06.2 A `currency` tool in the registry, slotted where [[Instant Answers]] §4.2 says:
      after the unit converter, before prayer times
- [ ] M8-T06.3 Every rate carries `as_of` from the publisher's own timestamp, and a stale rate is
      **withheld rather than shown aged** — the weather rule, for the same reason
- [ ] M8-T06.4 **The parallel rate ships disabled.** [[Milestone 1B - Frontend and Instant Answers|M1B-T06.7]]
      settled this: if no honest source exists, it ships off rather than invented. None exists; the
      square-market rate is quoted by no publisher we can verify. The card names the rate it shows
      as official and says plainly that the parallel rate is absent for want of a source, which is
      more useful than a confident wrong number and is the whole of [[Instant Answers]] §2
- [ ] M8-T06.5 Arabic, French and English phrasings, Arabic-Indic digits, and the dinar formatted
      the way Algerian print formats it

## M8-T07 — The deep calculator

- [ ] M8-T07.1 Adopt `fend-core` (MIT, no dependencies, arbitrary precision, unit-aware) as the
      **evaluation engine behind** the existing calculator and converter. Matching, confidence,
      localisation and digit folding stay ours — they are the parts that decide whether a card
      appears at all, and they are well tested
- [ ] M8-T07.2 What that buys, none of which is expressible today: mixed-unit arithmetic
      (`5 km + 3 miles in m`), unit-aware exponents, number bases, physical constants, and chained
      percentages
- [ ] M8-T07.3 **Currency inside expressions** — T06's rates injected as the engine's exchange-rate
      source, so `20 eur + 5 usd in dzd` is one answer with one `as_of`, not two cards
- [ ] M8-T07.4 Bounded evaluation: the engine's interrupt wired to a hard time budget, expression
      length and depth capped. A calculator is an arbitrary-expression evaluator facing the open
      internet, and it gets treated as one
- [ ] M8-T07.5 The existing golden expressions must pass unchanged, and the decimal guarantee holds:
      `45*1.19` is `53.55`, not `53.549999999999997`
- [ ] M8-T07.6 Localised rendering of engine output — the engine computes, our layer formats. An
      English unit name reaching an Arabic card is a bug the M1B unit table already tests for

## M8-T08 — Delivery and rendering

- [ ] M8-T08.1 The panel is fetched **out of band**, like the summary: the search response carries a
      token, the browser asks for the panel, and the search path costs nothing. This is the grain
      the codebase already has — instant answers are never deadline-gated and the knowledge panel
      already fetches after paint
- [ ] M8-T08.2 A real loading state, unlike today's panel. The operator asked for the placeholder
      and they are right *for this panel*: it is wide, it is expected, and a reader who is going to
      get a face and five facts should be told they are coming. `Summary.tsx`'s three states —
      loading, resolved-empty (collapse silently), resolved-full — with `aria-live` and
      `aria-busy`, and no layout shift when it lands
- [ ] M8-T08.3 The rail takes a stack, not a single node: `Shell({ aside })` widens to accept the
      entity panel above the existing Wikipedia panel, with the latter becoming a fallback rather
      than a peer once T03.6 lands
- [ ] M8-T08.4 RTL and bidi correctness throughout, `scripts/lint-bidi.sh` green: Latin titles,
      years and scores inside Arabic panels are `<bdi>`-isolated
- [ ] M8-T08.5 No-JS: the tool cards (weather, currency, calculator) are server-rendered and must
      work with JavaScript off, as they do now. The entity panel is the one part that legitimately
      requires it, and its absence must degrade to nothing visible rather than to a broken frame
- [ ] M8-T08.6 Bundle budget holds. `scripts/bundle-budget.sh` is the gate; an SVG sparkline and a
      panel component must not cost what a chart library costs

## M8-T09 — Demand-driven coverage

- [ ] M8-T09.1 A resolution miss is recorded through the **existing k-anonymous weak-coverage
      mechanism** — the same floor, the same window, the same ephemeral signals instance. An entity
      asked for by fewer than `k` people is never written anywhere
- [ ] M8-T09.2 The harvester works that queue: what people actually ask for gets fetched, so the
      store converges on this audience rather than on a guess about it
- [ ] M8-T09.3 Surfaced on the admin console beside weak coverage — including, honestly, when
      nothing is chasing the queue

## M8-T10 — Gates

- [ ] M8-T10.1 `make egress-test` green with every new component in the topology: the serving plane
      still reaches nothing. A new fetcher on `ingest` gets its own assertion, per the
      [[ADR-0017 - Query-Time Federation with External Metasearch]] commitment that no-egress cannot
      silently widen
- [ ] M8-T10.2 Telemetry lint green: no entity name, place name, or expression anywhere near a log
      line, metric label, or span attribute. A resolved entity id is not exempt — it is a query with
      a lookup applied
- [ ] M8-T10.3 A privacy test that the client address reaches the location lookup and **no further**,
      pinned the way `ratelimit.rs:390` pins its own rule, so a future change to accept
      `X-Forwarded-For` or to cache by IP has to delete a test that says why not
- [ ] M8-T10.4 Panel latency: p95 ≤ 100 ms served from the local store, measured cold
- [ ] M8-T10.5 Precision: zero wrong-entity panels across the T02.5 corpus. Not "few"
- [ ] M8-T10.6 Every new dependency clears `cargo deny check licenses` — the
      [[Qwen licence|non-commercial-model]] lesson, applied before the dependency lands rather than
      after

---

## Deliberately not in this milestone

- **Live scores and live sports.** Free sources cover fixtures, squads and history; live results
  are a paid feed everywhere we looked. [[Milestone 1B - Frontend and Instant Answers|M1B-T07.5]]
  stays open rather than shipping a scoreboard that is quietly an hour behind
- **Scraping any authority.** IMDb, Rotten Tomatoes and their peers are read *by identifier* from
  CC0 data and linked to. If a fact is not available under a licence we can honour, the panel does
  not have it
- **The parallel exchange rate**, per T06.4, until a source exists that can be named
- **A general web-tier egress allowance.** T03 widens an allowlist by named host, in an ADR, one at
  a time
