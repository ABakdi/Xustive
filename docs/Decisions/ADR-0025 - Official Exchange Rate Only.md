---
tags: [adr]
adr-id: "0025"
status: accepted
date: 2026-08-26
---
# ADR-0025 - Official Exchange Rate Only

## Status

Accepted, implemented. Constrains [[Instant Answers]] (the currency card) and [[Tool Data Plane]].
Applies the rule [[Milestone 1B - Frontend and Instant Answers|M1B-T06.7]] set — *if no honest
source exists, ship disabled rather than invented* — to the one number most Algerians actually
want.

## Context

The currency converter is the most Algeria-specific thing the product does, and the question a
reader here most often has is not the Bank of Algeria rate but the parallel ("square") market
rate at Port Saïd, which can differ from the official one by half again. Every local news site
quotes it. None of them is a publisher we can verify: the figures are hearsay from traders,
differ between sites on the same day, carry no timestamp we can trust, and no one stands behind
them.

Two other facts shaped the source choice. The European Central Bank — the obvious free reference
— does not publish the dinar at all, so a converter built on it cannot do `20 eur dzd`. And a
cross-rate computed from two other rates carries the error of both and the `as_of` of neither.

## Decision

**The card shows one rate, the official reference rate, from one keyless publisher that quotes
the dinar and the majors on a single daily timestamp. The parallel rate is deliberately absent,
and the card says so.**

- No derived cross-rates: one source, one timestamp, stored per currency pair as published.
- The card names its rate as *official* and states that the parallel rate is missing for want of
  a source — the absence is explained, not hidden, so a reader who wanted the square rate knows
  they have not been given it.
- Fetched on the ingestion plane by `toold` every 6 h (rates publish once a working day), kept for
  48 h so a weekend or a publisher holiday does not blank the card, and **withheld** past that
  rather than shown aged — the weather rule, for the same reason.
- A curated list of currencies (the dinar, what Algerians hold or receive, the neighbours, the
  majors) rather than the publisher's 160.

## Consequences

**Good**
- Nothing on the card is a number nobody stands behind. A confident wrong number is the failure
  [[Instant Answers]] §2 exists to prevent.
- The dinar works, which the ECB route could not deliver.

**Bad**
- The card does not answer the question many readers meant. This is a real gap, and it is the
  price of the honesty rule.
- One publisher is one point of failure; a two-day staleness window is the only buffer.

## Alternatives

| Option | Why not |
|:---|:---|
| Scrape a news site's daily square rate | unverifiable hearsay, inconsistent between sites, no publisher stands behind it |
| Show the parallel rate with a "unofficial" caveat | a caveat does not make an invented number honest; readers convert with it anyway |
| ECB reference rates via Frankfurter | authoritative, but no dinar |
| Compute DZD via cross-rates | error and staleness of both legs, labelled as neither |

## Revisit when

- A publisher with a name, a method, and a timestamp starts quoting the parallel rate — then it
  ships as a second, clearly labelled rate, never as a replacement for the official one.
- The chosen publisher drops the dinar or adds a key requirement.

## Where it stands (2026-08-27)

Implemented in `crates/xustive-toold/src/rates.rs` (dataset `tool:rates:v1`, cadence 6 h,
staleness limit 48 h, `CURRENCIES` list, and the module comment that records this decision) and
answered on the search path by `crates/xustive-api/src/currency.rs` (commit `12ff07a`, M8-T06).

## Related

[[Instant Answers]] · [[Tool Data Plane]] · [[Milestone 8 - The Answer Layer]] ·
[[Milestone 1B - Frontend and Instant Answers]] · [[Decision Log]]
