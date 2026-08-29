---
tags:
  - component
  - ingestion
component-id: C29
binary: xustive-toold
status: built
updated: 2026-08-27
---

# Tool Data Plane

> **ID** C29 · **Binary** `xustive-toold` (crate `crates/xustive-toold`) · **Upstream** external
> publishers · **Downstream** [[Instant Answers]] (weather, currency), [[Knowledge Store]]
> (entity harvest)

## 1. Why this exists as its own process

The serving plane **has no route to the internet**. That is enforced, not aspirational —
`scripts/test-egress.sh` fails the build if `xustive-api` can reach outside, and the `core`
network is `internal: true` ([[Deployment Topology]]).

Weather and exchange rates come from outside. Two ways to reconcile that:

1. Give the serving plane egress. **No.** That constraint is what guarantees a prompt-injected
   summariser or a compromised dependency cannot exfiltrate queries. It is worth more than any
   tool.
2. A separate fetcher on a network that *does* have egress, writing to a cache the serving plane
   reads.

This is (2). Same shape as the crawler, same reason.

The consequence is worth stating plainly: **the serving plane can only ever answer from cache.**
A tool needing data nobody has fetched yet has no answer, and that is correct rather than
unfortunate.

Since M8 the same process is also the **knowledge harvester** ([[Knowledge Store]]): it already
*is* the sanctioned bridge — on `ingest` for egress, on `core` for storage, taking no user input —
and duplicating that in a second binary would mean duplicating the security argument too.

## 2. Responsibilities

**In scope**: scheduled fetching of external tool data; validation; writing to the shared cache;
recording provenance and measurement time; harvesting Wikidata entities into the knowledge index.

**Out of scope**: matching, formatting, rendering (→ [[Instant Answers]]); anything per-query. It
never sees a query. It cannot — it does not share a network with anything that has one.

## 3. Where it lives today

| Piece | Path |
|:---|:---|
| `Cached<T>`, `Dataset` trait, shared `reqwest` client | `crates/xustive-toold/src/lib.rs` |
| Weather (Open-Meteo) | `weather.rs` |
| Exchange rates | `rates.rs` |
| Plausibility checks | `validate.rs` |
| Redis put/get | `store.rs` |
| Wikidata harvest | `knowledge.rs` |
| Loop, CLI flags, demand queue | `main.rs` |
| Readers on the serving side | `crates/xustive-api/src/weather.rs`, `currency.rs`, `dataage.rs` |

The user agent is `XustiveToolFetcher/0.1 (+https://xustive.dz; contact via repository)` — a
publisher seeing unexplained traffic should be able to find out who it is.

## 4. Interface

Writes to Redis, which both planes reach. One key per dataset entry, versioned so a schema change
does not have to reconcile with entries an older build wrote — it writes elsewhere and the old
ones age out:

```jsonc
tool:weather:v2:31             // wilaya code; v2 added the hourly series and days 6–7 (M8-T05.2)
tool:weather:v2:c-paris        // a world city, namespaced so wilaya keys stay bare (2026-08-29)
tool:rates:v1                  // one table, all currencies against USD
{
  "fetched_at": 1786000000,
  "observed_at": 1785998400,   // when the *publisher* measured it, not when we asked
  "source": "open-meteo",
  "licence": "CC-BY-4.0",
  "payload": { … }
}
```

`observed_at` and `fetched_at` are separate on purpose. A rate we fetched a minute ago that the
publisher measured yesterday is a day old, and showing the fetch time would be a lie of exactly
the kind §2 of [[Instant Answers]] forbids. `Cached::age()` and `is_stale()` use `observed_at`.

The Redis TTL is **four times** the staleness limit. Expiry is a backstop; "too old to show" is
decided by the serving plane, and letting Redis delete at exactly the limit would make a stale
entry indistinguishable from one never fetched.

## 5. Datasets

| Dataset | Cadence | Source | Staleness limit | Status |
|:---|:---|:---|:---|:---|
| Weather, 58 wilayas | 30 min | Open-Meteo (CC-BY-4.0, no key) | 3 h | built |
| Weather, ~90 world cities | 2 h (every 4th pass) | Open-Meteo | 3 h | built 2026-08-29 |
| Currency, official | 6 h | `open.er-api.com` (exchangerate-api open access, attribution) | 48 h | built |
| Knowledge entities | weekly per entity | Wikidata `wbgetentities` + Wikipedia extracts (ar/fr/en) | — (Meilisearch, not Redis) | built |
| Currency, parallel | — | no honest source | — | **not built** (2026-08-27) — see §7 |
| Fuel prices | — | compiled into `xustive-tools` with an effective date | — | not a fetch: administered value ([[Instant Answers]] §6.2) |
| Sports fixtures & results | — | — | — | not built (2026-08-27) |
| Exam result portals | — | links compiled into `xustive-tools` | — | not a fetch |

The world cities ride along on every fourth pass (`WORLD_EVERY`), which is a refresh every two
hours — inside the three-hour staleness limit, at a quarter of the request cost of putting them
on the wilaya cadence. Their forecasts are fetched with `timezone=auto` and the response's own
`utc_offset_seconds` is used to read the observation time: reading a Tokyo reading as Algiers
time made it look nine hours old, and every world city was rejected as implausible on the first
run. Their temperature bounds are the world's (−70…60 °C), not Algeria's (−25…58 °C).

Fixed cadence, never per-request. 58 wilayas every 30 minutes is 116 requests an hour: a trivial
load for the publisher, and — the point — **a request pattern that reveals nothing**, because it
is identical whether one person or a million searched for weather. Requests are paced at 250 ms
apart; a burst of 58 is rude to a free publisher and there is no deadline worth being rude for.

**Why not the ECB for rates.** The obvious source is the ECB's daily reference table — free,
authoritative, self-hostable through Frankfurter. **The ECB does not publish the dinar.** An
Algerian converter that cannot do `20 eur dzd` is one nobody here needs. The publisher used quotes
the dinar and the majors together, keyless, on one daily timestamp; there are no derived
cross-rates because a rate computed from two others carries the error of both and the `as_of` of
neither. Twenty currencies are stored, not the publisher's 160.

The main loop runs every `--tick` seconds (300 by default), or once with `--once`. Each pass does
weather, then rates, then — when `--meili` is set — the knowledge harvest. A failed rates pass
keeps the previous table; a failed knowledge pass must not cost the weather cards.

## 6. Validation

Fetched data is checked before it is written, because a bad write silently poisons every
subsequent answer (`validate.rs`):

- **Plausibility bounds** (`bounded`): temperature within −25…MAX °C, humidity 0–100, wind
  0–250 km/h. A temperature of 300 °C or a dinar rate of 4 to the euro is rejected and the
  previous value kept.
- **Movement guard** (`movement`): a value moving implausibly far since the last reading is
  held — a 25 °C jump between weather passes, a decimal-point slip in a rate. Real moves that
  large exist; so do publisher errors, and a slightly late correct value costs far less.
- **Timestamp** (`timestamp`): the publisher's own `observed_at` must not be in the future (300 s
  skew allowed) or older than six hours for weather.
- NaN and infinity are refused.
- A rejection **keeps the previous value** and counts; never a partial write. Rejection labels
  (`out_of_bounds`, `moved_too_far`, `bad_timestamp`, `missing_field`) are stable for metrics.

## 7. Provenance

Every dataset carries its source and licence into the rendered card (`detail.source`,
`detail.licence`). This is not legal box-ticking — it is what lets a reader judge a number we
cannot independently verify.

The parallel exchange rate has **no authoritative publisher**. M1B-T06.7 settled the rule: if no
honest sourcing can be found, **the parallel rate ships disabled** and the card names its rate as
official and says the other is missing for want of a source. That is where it stands. Shipping a
made-up number because the feature looks better with two is not an option.

## 8. The knowledge harvest (M8-T01.2, M8-T09.2)

Reads `data/knowledge/seeds.tsv`, fetches what is due (an entity is re-harvested at most every
`--knowledge-max-age`, default seven days), resolves referenced labels in one batch of fifty,
attaches ar/fr/en extracts, drops anything that is not what the seed said it was, and writes the
renderable entities to Meilisearch. 300 ms between requests, sequential — politer than Wikimedia
asks and free on a job with no deadline.

The **demand queue**: when `--signals` names the ephemeral signals store, names people searched
for that the store does not hold are read from the same k-anonymous counters the serving plane
writes (`--k-anonymity 20`, `--demand-window-days 30`), resolved to ids, and appended to the seed
set *for that pass only*. Nothing is written back to the seed file: promoting a name into a curated
list is a human's decision. Off by default, because it should only run where the operator turned
recording on. The rest of the knowledge side is in [[Knowledge Store]].

## 9. Failure

| Failure | Response |
|:---|:---|
| Publisher down | Keep last good value; it ages and is withheld by [[Instant Answers]] |
| Publisher returns nonsense | Rejected by §6; previous value kept; counted |
| Redis down at start | **Fatal** — the one place in this codebase that is. Without a cache there is nowhere to put anything, and fetching to discard burns a publisher's bandwidth for nothing |
| Redis down later | Serving plane finds no cache and renders no card. Search is unaffected |
| `xustive-toold` down | Data ages out and cards disappear over hours. **Never fatal** to search |
| Meilisearch unreachable | Knowledge harvest skipped; weather and rates continue |

Nothing here can degrade search. That is the test for whether this component is correctly
separated.

## 10. Security

- Runs on the `ingest` network. **No route to `core`** beyond the Redis and Meilisearch it
  writes.
- Receives no user input, ever. Its inputs are a schedule, a fixed list of URLs and a seed file.
  The demand queue reads *counts that cleared k-anonymity*, never a query.
- Holds no credentials for anything the serving plane can reach.

## 11. Observability

The process itself logs pass summaries (`written`, `rejected`, `failed`). The gauge that matters
lives on the **serving** side: `xustive_data_age_seconds{dataset}` (`dataage.rs`) samples the
cache every 60 s regardless of traffic and reports the **oldest** entry.

Sampled rather than recorded on use because the failure this catches is a fetcher that **stops
silently**: `toold` dies at 03:00, nobody searches for weather until 07:00, and a gauge set on
use would be four hours behind at the moment it matters — worse, frozen at a healthy value. The
maximum rather than the mean because if Tamanrasset alone stuck, an average over 58 would move two
per cent and no threshold would fire. **This is the one to alert on.**

## 12. Open questions

- [ ] Parallel-rate sourcing — still the open question that gates the currency card (§7).
- [ ] Do sports feeds need a licence review before use? Moot until one is chosen.
- [ ] The per-dataset `cadence()` is declared but the loop fetches on every tick; should the
      loop honour it, or is the five-minute tick against a 30-minute cadence simply a cheap
      over-fetch nobody minds?

## Related

[[Instant Answers]] · [[Knowledge Store]] · [[Deployment Topology]] · [[Security and Privacy]] ·
[[Web Fetcher]] · [[Observability]] · [[Legal and Compliance]] ·
[[ADR-0019 - The Knowledge Layer]]
