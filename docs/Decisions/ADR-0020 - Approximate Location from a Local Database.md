---
tags: [adr]
adr-id: "0020"
status: partly implemented; amended by ADR-0029 on 2026-08-27 — precise location may be used later with consent under the first-party data ADR; the local database stays the default
date: 2026-08-26
---
# ADR-0020 - Approximate Location from a Local Database

## Status

Accepted; **implemented except rule 8 (attribution)**. Constrains [[Instant Answers]], [[API Gateway]], [[Security and Privacy]] and
[[Tool Data Plane]]. Operates inside [[ADR-0008 - No Query Logging]] and does **not** amend it —
the point of this ADR is that it does not need to.

## Context

*"weather"* with no place in it is the most common way anyone asks about the weather, and today it
answers nothing. The tool detects a wilaya name or gives up.

Every ordinary way to fix this is unacceptable here:

- **Ask the browser** (`navigator.geolocation`) — a permission prompt for a weather card is a bad
  trade, it needs JavaScript, and it yields a precise coordinate when the question needs a city.
- **Call an IP-geolocation service** — sends the reader's IP address to a third party on every
  weather search. This is the exact behaviour the whole architecture exists to prevent, and it
  would need egress the serving plane does not have.
- **Store a location per reader** — a durable per-person attribute, which is a profile.

But the address is *already in the process*. The API terminates the connection, so
`ratelimit.rs` already sees it, and has already established how this codebase handles it: bucket on
`HMAC(salt, ip/24)` with a salt from `/dev/urandom` rotated daily, memory-only, never written, and
`X-Forwarded-For` deliberately ignored — pinned by a test whose comment says a future change to
accept it *"has to delete a test that says why not."*

The observation is that turning an address into *"probably Oran"* needs no network at all. Offline
city-level databases are published under permissive licences — DB-IP City Lite under CC BY 4.0,
updated monthly, in a format Rust reads directly. The lookup is a few microseconds against a
memory-mapped file.

## Decision

**Resolve the client address to an approximate location by an in-process lookup against a locally
bundled database, use it for the current request only, and discard it. The address is never
logged, never stored, never used as a cache key, and never leaves the process.**

The rules, each of which is testable:

1. **Local only.** A bundled database file, refreshed like any other data asset. No lookup service,
   no network call, no egress. The serving plane's no-egress property is untouched.
2. **The connection address only.** `X-Forwarded-For` is not consulted, matching `ratelimit.rs`. A
   deployment behind a proxy is a deployment where this feature degrades to "no location", which is
   the correct failure.
3. **City granularity, then coarser.** The result is immediately mapped to the **nearest wilaya
   seat** — the 58 coordinates [[Milestone 1B - Frontend and Instant Answers|M1B-T07.1]] already
   compiles in — and only the wilaya code travels onward. A coordinate never reaches the cache
   lookup, the response, or anything else.
4. **Request-scoped.** The value lives in one function's stack. It is not attached to a session,
   not memoised, and not carried into any store. There is nothing to expire because nothing is
   written.
5. **Never a cache key.** The weather cache is keyed by wilaya, as it already is — an enumerable
   set of 58, shared by every reader in that wilaya. Keying anything by address would be keying by
   person.
6. **Never in telemetry.** No address, no city, no coordinate in a log line, metric label, or span
   attribute. The existing telemetry lint covers the mechanism; a wilaya code is low-cardinality
   and non-identifying, and is the only part that may appear.
7. **Visible and correctable.** The card names the place it assumed. A reader who is somewhere else
   types the place, and the named-place path — which already works — takes over.
8. **Attribution.** CC BY 4.0 requires crediting the database, and the interface carries it.

## Consequences

**Good**

- *"weather"* answers, which is the point.
- No new egress, no new stored data, no new identifier, and no amendment to
  [[ADR-0008 - No Query Logging]] — the reason this shape was chosen over every alternative.
- Works without JavaScript and without a permission prompt.
- Costs microseconds and scales to any traffic, because it is a memory-mapped lookup rather than a
  request.
- The failure mode is graceful and honest: no match means no assumed location, and the card says
  so instead of guessing.

**Bad**

- City-level accuracy from a *Lite* database is imperfect, and Algerian ISP address allocation is
  not always geographically clean. A reader in Blida may be told Algiers. Mitigated by naming the
  assumption on the card and by the named-place path being one word away.
- Mobile carrier ranges frequently resolve to a national centroid, which will over-report Algiers.
- Readers behind a VPN or a corporate proxy get the exit's location. Naming the place on the card
  is again what keeps this from being confusing.
- The database is a data asset that must be refreshed, and an unrefreshed one degrades silently.
  It gets a staleness gauge like every other dataset.
- The attribution link is a permanent interface obligation for as long as the database ships.

## Alternatives

| Option | Why not |
|:---|:---|
| `navigator.geolocation` | Permission prompt for a weather card; needs JavaScript; over-precise for the question. Remains available as a future opt-in for a reader who wants exactness. |
| Third-party IP-geolocation API | Sends the reader's address to a third party per search and needs serving-plane egress. Precisely what the architecture forbids. |
| Remember a chosen location in a cookie | A durable per-reader attribute — small, but a profile. Rejected on the same grounds as ADR-0018's identifier rules. A one-search, non-persistent override is fine and is what the named-place path already is. |
| Infer location from the query's language | `ar` does not mean Algeria and `fr` certainly does not. Wrong often, and wrong in a way that looks like a bug. |
| Do nothing; require a place name | The status quo, and it fails the most common phrasing of the question. |

## Revisit when

- Measured accuracy for Algerian ranges is poor enough that the card is wrong more often than it
  is right, at which point "no assumed location" is better than a bad one.
- A deployment topology puts a real proxy in front of the API, which would require an explicit,
  ADR-level decision about trusting a forwarded header — not a quiet configuration change.
- A licensed database with materially better Algerian coverage becomes available.

## Where it stands (2026-08-27)

- `crates/xustive-api/src/geoip.rs` opens the bundled DB-IP file with `maxminddb::Reader::open_readfile` (loaded on the heap, not mmap — the "never leaves the process" property is the same), maps the coordinate to the nearest of the 58 wilaya seats, and is request-scoped — pinned by a source-reading test. The weather cache is keyed by wilaya; the card names the assumed place. `scripts/fetch-geoip.sh` refreshes the file.
- **Unmet: rule 8.** No DB-IP CC BY 4.0 attribution anywhere under `web/` (no `db-ip`/`dbip` string in `web/app`, `web/components` or `web/public`). The licence obligation is open until the interface credits the database.
- No staleness gauge for the `.mmdb` file — an operator cannot see from metrics that the database is months old.
- The privacy page does not yet say that the connection address is read for this (see ADR-0018 Where it stands).
