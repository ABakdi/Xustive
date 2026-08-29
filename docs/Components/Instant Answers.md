---
tags:
  - component
  - serving
  - ui
component-id: C28
binary: xustive-api
status: built
updated: 2026-08-27
---

# Instant Answers

> **ID** C28 · **Crate** `xustive-tools` (matchers) + `xustive-api` (cache-backed cards) + Next
> frontend (`web/components/tools`) · **Upstream** [[Query Pipeline]] · **Downstream**
> [[Tool Data Plane]] (weather and rates cache), [[Summarizer]] (translation model)

## 1. Purpose

Answer the query directly when the query *is* the question. `45 * 1.19` wants a number, not ten
pages about multiplication. `مواقيت الصلاة بجاية` wants today's times. `20 euros en dinar` wants a
rate — and in Algeria, wants **two** rates. (The second one is still missing; §7 says why.)

The entity panel — "who is X", "what is Y" — is a different mechanism with its own note:
[[Knowledge Store]]. This note is about tools that compute or look up.

## 2. The rule that governs everything here

**A tool must be right, or absent.** Never approximately right, never confidently stale.

A search result that is mediocre costs the user a click. A calculator that is wrong, or a currency
rate that is three weeks old presented as today's, destroys the reason to use the product at all.
Every design choice below follows from this:

- A tool that cannot answer **renders nothing**. Not an error, not a placeholder — the results
  are already there.
- Any value with a time dimension shows **when it was measured**, always, not only when stale.
- A tool never guesses at ambiguity. `1500` is not a currency conversion.

## 3. Where it lives today

| Piece | Path |
|:---|:---|
| `Tool` trait, `Answer`, `registry()`, `best_in()`, `fold_digits()` | `crates/xustive-tools/src/lib.rs` |
| Pure tools | `calculator.rs`, `deep.rs` (fend), `units.rs`, `datetime.rs`, `prayer.rs`, `fuel.rs`, `exam.rs`, `wilaya.rs` + `wilaya_data.rs`, `utilities.rs`, `translator.rs`, `transliterate.rs` |
| Pure *detectors* whose answer needs the cache | `weather.rs`, `currency.rs` |
| Cache-backed answers | `crates/xustive-api/src/weather.rs`, `currency.rs`, `geoip.rs` |
| Dispatch on the search path | `crates/xustive-api/src/search.rs` (`instant` field) |
| Tool inventory for the settings page | `crates/xustive-api/src/tools.rs` → `GET /api/v1/tools` |
| Translation stream | `crates/xustive-api/src/translate.rs` → `POST /api/v1/translate`, `GET /api/v1/languages` |
| Cards | `web/components/tools/ToolCard.tsx`, `Calculator.tsx`, `Converter.tsx`, `WeatherDetail.tsx`, `TranslateCard.tsx`, `DismissTool.tsx`, `CopyButton.tsx`; `web/lib/calc.ts`, `web/lib/units.ts`; opt-out cookie in `web/lib/tools.ts` |

## 4. Interface

Instant answers arrive **with** the search response, not after it. Pure matchers answer in
microseconds and blocking on them costs nothing:

```jsonc
GET /api/v1/search?q=45*1.19&ui=ar
{
  "query": { … },
  "instant": {
    "tool": "calculator",
    "confidence": 0.98,
    "interpretation": "45 × 1.19",   // how the query was read — a misread must be visible
    "value": "53.55",
    "detail": { … },                  // optional, structured, for interactive cards
    "as_of": null                     // absent = timeless; a unix time means "measured at"
  },
  "results": [ … ]
}
```

`Answer` has exactly these fields; there is no `sources` array — provenance travels in `detail`
(`source`, `licence`) for the cache-backed tools. `ui` is the interface language, distinct from
`lang` which filters results: it picks the unit names and labels an Arabic reader sees
(`2 قنطار → كيلوغرام`, not `2 qintar → kilogram`).

The matchers run on the **raw** query, before normalisation, because normalisation folds the very
characters an expression is made of.

A tool needing live data ([[Tool Data Plane]]) is served **only from cache** on this path. If the
cache is cold or stale, `instant` is `null`. There is no separate "ask again" endpoint — search
never waits on the network for a side feature, and nothing else asks either.

## 5. Routing: which tool, if any

### 5.1 Matching

Each tool implements `Tool::answer(&self, query) -> Option<Answer>` (and `answer_in(query, lang)`
where the output has language-dependent labels). Matchers are **pure, total and fast** — no I/O,
no panics. A panic is caught with `catch_unwind` and that tool is skipped; a unit converter
tripping over a malformed number must not 500 a search.

Confidence is a number in `0..=1`, and **must reflect how much of the query the tool explains**.
Structural matches (the whole string parsed as an expression) sit near 0.98; an explicit verb with
a guessed operand split (translate) at 0.85; shape-only inferences lower. Below
`MIN_CONFIDENCE = 0.5` nothing renders — an unwanted card pushing results down is worse than no
card.

### 5.2 Arbitration

Every matcher runs; the highest confidence wins; ties fall to registry order:

```
calculator > unit-converter > date > prayer-times > fuel > exam > wilaya
  > utility > translate > transliterate
```

`5 km en miles` matches both calculator (a number) and the converter. The converter wins because
it consumed the whole string; the calculator matched a fragment.

**Weather and currency are not in the registry.** Their detectors are pure (`weather::detect`,
`currency::detect`) but their *answers* need the Redis tool cache, and a matcher that did I/O
would put a Redis round trip on every search that is not about weather. So `search.rs` runs the
pure registry first and consults the cache-backed pair **only when nothing pure matched** —
currency before weather, since `20 eur dzd` names no weather word and the two cannot both match.

### 5.3 Explicit invocation

`!calc`, `!convert`, `!date`, `!salat`, `!fuel`, `!exam`, `!wilaya`, `!util`, `!tr`, `!translit`
force a registry tool and skip arbitration. `GET /api/v1/tools` lists every id and keyword
(registry plus `weather` and `currency`) so the settings page never carries its own copy of the
list.

### 5.4 Opting out

A reader can switch a tool off from its card (`DismissTool`). The disabled list lives in a
cookie, `xustive-tools-off`, not `localStorage`, so the server render already knows not to show
it and the card never flashes in and out — a card that appears and disappears is layout shift,
which [[Performance Budgets]] forbids.

## 6. The tools

### 6.1 Built

| Tool | id / keyword | Triggers | Notes |
|:---|:---|:---|:---|
| Calculator | `calculator` / `calc` | `45*1.19`, `15% of 2000`, `2000 + 19%`, `sqrt(64)`, `٤٥ × ٥` | Exact decimal (`rust_decimal`) first; `fend-core` fallback for unit-aware expressions (§7). |
| Unit converter | `unit-converter` / `convert` | `5 km en miles`, `30°C to F`, `2 قنطار كيلو` | Length, mass, temperature, area, volume, speed, data. **Qintar (100 kg) and sa'a (400 m²)** which no international converter carries. Names rendered per `ui`. |
| Currency | `currency` / `currency` | `20 eur dzd`, `100 dollar en dinar`, `20 eur + 5 usd in dzd` | Cache-backed. **Official rate only** — see §8. Twenty currencies. |
| Weather | `weather` / `weather` | `طقس وهران`, `météo Alger`, `weather paris`, `weather` | Cache-backed. Now, 48 h hourly, 7 days, for the 58 wilayas **and ~90 world cities** (§Weather places). `طقس` alone means "here": wilaya guessed from a **local GeoIP database**, coarsened to a wilaya, never stored ([[ADR-0020 - Approximate Location from a Local Database]]). The card always says which place it assumed, and a city carries its country. |
| Prayer times | `prayer-times` / `salat` | `مواقيت الصلاة`, `heure priere Setif` | Computed locally; §10. |
| Date | `date` / `date` | `12 août 2026 hijri`, `days between 1/1/2026 and 1/7/2026` | Gregorian → Hijri (tabular civil calendar, and the card says so) and day arithmetic. Maghrebi month names (`أوت`, not `أغسطس`). No world clock. |
| Wilaya reference | `wilaya` / `wilaya` | `code postal bejaia`, `ولاية 06` | 58 wilayas: code, postal, dial, seat coordinates. Compiled in; also the coordinate source for prayer and weather. |
| Fuel prices | `fuel` / `fuel` | `prix essence`, `سعر البنزين` | Administered values; see below. |
| Exam results | `exam` / `exam` | `نتائج البكالوريا`, `resultat bem`, `cinquième` | **Links to the official ONEC portals only.** Never fetches, stores or shows a result. |
| Translator | `translate` / `tr` | `translate X to arabic`, `traduire`, `ترجم` | Explicit verb required; answer streams from `/translate`. §9. |
| Transliterator | `transliterate` / `translit` | `arabizi`, `franco`, `en arabe`, `بالحروف العربية` | Arabizi ↔ Arabic via `xustive_lang::translit` — the engine's own mapping, surfaced. Offered, never applied to the query: Arabizi is ambiguous. |
| Utilities | `utility` / `util` | `base64 …`, `url encode`, `tva 19% 1000`, `sha256 …`, `roman 2026`, `bmi`, `tip`, `loan` | Base64, URL codec, Algerian TVA (19 % / 9 %), percentage change, Roman numerals, SHA-256, JSON formatter, tip split, loan repayment, BMI (number only), hex→rgb, case conversion, word/character count. All pure. |

### 6.2 Administered values

Fuel prices are set by an authority, not measured. That makes them a different kind of value from
a temperature or an exchange rate, and the difference is visible on the card: an administered price
carries **no `as_of`**, because nothing was observed at any particular moment. It carries an
**effective date** (`2026-01-01`) and the body that set it (`ARH`).

The failure mode is inverted. A stale temperature is detectable — its age climbs and the serving
plane withholds it. A stale administered price looks exactly like a correct one, forever. The ARH
changed fuel prices at midnight on 1 January 2026 with no announcement from itself or Naftal; a
table compiled before that would have kept answering confidently.

Where no feed exists, the defence is a **review date that fails the build**: `fuel::REVIEW_BY`
is `2027-03-01` and a test fails once it passes. A broken build is cheap. A search engine quoting
a price that changed eight months ago is not.

### 6.3 Not built (as of 2026-08-27)

| Wanted | What it needs |
|:---|:---|
| Parallel-market exchange rate | An honest source. None found; §8. |
| Dictionary / Darija definitions | A Darija lexicon — the B7 human work. |
| Sports fixtures and results | A feed and a licence review. |
| Time-of-day / world clock | Trivial, just not written. |
| Time units in the converter | The converter has no `Time` dimension yet. |
| QR code | An encoder and an SVG renderer — its own task. |
| Timer & stopwatch | A client component with no server side. |
| `what is my IP` | The only tool needing request context; the `Tool` trait is a pure function of the query. |
| hsl / oklch | hex→rgb covers the common case. |
| Prayer method / Asr rule selectable on the card | The engine supports Umm al-Qura, MWL, Egyptian and Shafi'i/Hanafi Asr; the card exposes only the default. |

**Deliberately excluded**, with reasons:

| Not building | Why |
|:---|:---|
| Password generator | Generating a secret in a *search box* trains exactly the wrong instinct, and the box is the field most likely to end up in a history. |
| Random numbers, dice, coin flips | Unreproducible. A card showing a different number on every reload reads as a bug; one showing the same number is not random. |
| MD5 and SHA-1 | Someone reaching a search box for a hash is as likely to be hashing something that matters as verifying a download. SHA-256 only. |
| A BMI *category* | The number is arithmetic. A band attached to it reads as a judgement, which is the medical-calculator line below. |
| Stock tickers | We cannot be authoritative and being late is worse than being absent. |
| Medical / dosage calculators | Wrong output causes physical harm. Not a search-box feature. |
| Live flight tracking | Needs a paid feed we cannot verify. |

## 7. Calculator

- **Decimal arithmetic**, not binary floats. `0.1 + 0.2` renders `0.3`.
- The hand-written `rust_decimal` parser runs first and keeps its exact results: `+ - * / ^ ( )`,
  `sqrt`, `abs` and friends, thousands grouping in the output.
- Percentages read as people write them: `15% of 2000` → 300; `2000 + 19%` → 2380.
- **Arabic-Indic digits accepted** (`٤٥` = 45) — `fold_digits` also folds `٫`, `٬`, `×`, `÷`, `٪`.
- A bare number is not a calculation (`2026` wants the year), and a lone leading minus is a
  search operator (`-covid`), so an operator is required.
- **`fend-core` fallback** (`deep.rs`, M8-T07) for what the decimal parser cannot take:
  `5 km + 3 miles`, `20 eur + 5 usd in dzd` (rates injected from the cache by `currency.rs`).
  Bounded on purpose — `MAX_LEN = 200` characters, a 120 ms interrupt, arithmetic only —
  because an expression evaluator facing the open internet needs a leash.
- **No variables, no state, no user-defined functions.**
- Division by zero, overflow and malformed input render **nothing**.

## 8. Currency — official only, and why

Algeria has two exchange rates: the official Bank of Algeria rate, and the parallel-market rate
("square" / السوق الموازية) which is what people actually transact at and is frequently 40–60 %
different. A converter showing only the official rate is wrong in the way that matters.

**The card nevertheless shows only the official rate.** M1B-T06.7 settled the rule: if no honest
source exists, the parallel rate ships disabled rather than invented. No publisher we can verify
quotes the square rate, so `detail.rate_kind = "official"` and `parallel_available = false`, and
the card says in words that the other rate is missing for want of a source. A confident wrong
number is the failure §2 exists to prevent.

What is there: twenty currencies from one keyless daily publisher (`open.er-api.com`, stored
against USD, the card divides), `as_of` = the publisher's own timestamp, source and licence in
`detail`, `unit_rate` so a reader can sanity-check the arithmetic, six decimals when the answer is
below one (`1 DZD in EUR` is 0.0075, and `0.01` is not an answer). Older than **48 hours** the
rate is withheld ([[Tool Data Plane]]).

## 9. Translator

Runs on the **local Qwen model already loaded for [[Summarizer]]** — same engine, same device
setting, same slot pool. Nothing typed into the translation box leaves the machine, and the text
is never logged at any level; it is a POST body precisely so it never reaches a URL or an access
log.

- `POST /api/v1/translate` streams tokens over SSE (unlike summaries, there is nothing to validate
  after the fact). 60 s budget, 512 output tokens, closing the connection cancels generation and
  frees the slot. No response-timeout layer on this route, because it streams.
- Languages (`GET /api/v1/languages`): ar, ary, fr, en, es, de, it, tr. Source may be omitted for
  auto-detection.
- The prompt restates the instruction **in the target language**: a fully English prompt asking
  for Arabic left the model with no Arabic in context and it drifted mid-sentence
  (`صباحك Okم يا друг`). A target-language sentence primes the right token space.
- Output is labelled as machine translation and names the model. **Into Arabic is still weak**
  on the 3B Q4 model — the card states this rather than hiding the feature; into fr/en/es is
  good. Darija as a target is offered and marked approximate.
- The matcher demands an explicit verb (`translate`, `traduire`, `ترجم`, `معنى`): every query is
  text in some language, so "looks translatable" describes all of them.

## 10. Prayer times

Computed locally from the wilaya's seat coordinates and the date — no network, no cache, nothing
that can go stale. Default **Umm al-Qura**, Asr per the Shafi'i/Maliki shadow rule (Maliki is
dominant in Algeria). MWL, Egyptian and Hanafi Asr exist in `prayer.rs` but are not yet
selectable on the card.

The method is **displayed on the card**, in the interpretation line. Fajr and Isha depend on a
twilight angle that authorities set differently, often by fifteen minutes; a card that does not
say which reckoning it used makes an ordinary disagreement with the local mosque look like a
defect. Where they disagree, the mosque is right.

## 11. Rendering

**Two tools are tools, not answers** (2026-08-29). The calculator and the unit converter render
as working instruments loaded from the query: `Calculator.tsx` — the expression in an editable
field, a keypad (`( ) % ⌫ / 7 8 9 ÷ / … / √ C =`), keyboard input, the result live on every
keystroke, `=` committing it as the next expression; `Converter.tsx` — the amount in a box, the
two units in menus grouped by dimension (changing the source unit keeps the target inside the
same dimension), a swap, the result and the unit rate live. Both evaluate locally
(`web/lib/calc.ts`, `web/lib/units.ts`, a mirror of the API's grammar and table) so the first
number shown equals the API's answer and every change after it costs no request. Labels and
unit names follow the interface language (ar/fr/en; Darija uses the Arabic names).

Tool cards sit **above the results and below the search box**, carrying the assert rule from
[[UI - Design Language]] — the mark that means the engine is asserting rather than listing.

- One card maximum. Never a stack.
- **Reserves no height until it has content** ([[Performance Budgets]] CLS ≤ 0.05).
- Every card carries: the answer, the interpretation in small text (so a misread is visible),
  source and `as_of` where applicable, a copy button and a dismiss control.
- The weather body (`WeatherDetail`) renders the hourly strip and the week; the WMO code is mapped
  to icon and label client-side so wording stays with the translations.
- The translate card is interactive: languages can be corrected and the stream re-run without a
  new search.

## 12. Privacy

- Matchers run **in-process**. A query is never sent anywhere to find out whether a tool applies.
- Tools never make per-query outbound requests. Live data comes from a cache the
  [[Tool Data Plane]] fills on a schedule.
- Translation runs locally. Nothing is logged, including the text.
- Location is **wilaya-level**: named in the query, or guessed from the connecting address by a
  memory-mapped local database, coarsened immediately, never written down and never a cache key.
  `X-Forwarded-For` is not consulted; behind a proxy this degrades to "no location". No browser
  geolocation prompt, ever.
- Tool usage is counted by *tool name only*: `xustive_instant_answers_total{tool}`.

## 13. Failure

| Failure | Response |
|:---|:---|
| Matcher panics | Caught; that tool is skipped |
| Cache cold, stale, or Redis down | Weather / currency card absent; search unaffected |
| Model busy or not loaded (translate) | `model_unavailable`; the card offers a retry |
| Ambiguous match | Below 0.5 confidence, nothing renders |
| Tool dismissed by the reader | Cookie; as if it did not exist |

Every one is invisible beyond a missing card.

## 14. Testing

- Golden expressions for the calculator, including the float traps.
- **Matcher precision**: `an_ordinary_query_matches_no_tool` — a corpus of ordinary queries
  that must match nothing. Regression here means the product started interrupting normal
  searches, which is the failure users notice fastest.
- Conversions checked against reference values including the Algerian units.
- Prayer times checked against published tables; the Hijri calendar round-trips every day of a
  year.
- `fuel::tests::the_table_is_due_for_review` fails the build after `REVIEW_BY`.
- `tools::inventory` is asserted to list every registered tool.

## 15. Open questions

- [ ] Where does the parallel rate come from, and can it be sourced without depending on a party
      that could manipulate it? Still the gate on the currency card's usefulness.
- [ ] Expose the prayer method and Asr rule on the card.
- [ ] Should the calculator accept a leading `=` like a spreadsheet?
- [ ] Does the translator card need a "report a bad translation" affordance, given no logging?
- [ ] Exam-results week is one day of extreme, predictable traffic — capacity handling?

## Weather places, and the one thing this tool must never do

Until 2026-08-29 the detector knew only the 58 wilayas, and anything else it did not recognise
fell through to the default: **`weather paris` answered with Algiers**, confidently, with no hint
that the question had been changed. Reported by the operator; it is the exact failure mode
[[ADR-0022 - Entity Resolution Prefers Silence to a Wrong Panel]] rules on for panels, and the
same rule applies here.

Three cases, and each has one correct behaviour:

| The query | What happens |
|:---|:---|
| Names a place we hold — a wilaya, or a world city | answer for **that** place; a city is labelled with its country (`Paris, France`), in the interface language |
| Names nothing (`طقس`, `weather today`) | answer for here — GeoIP coarsened to a wilaya, and the card says it assumed |
| Names a place we do not hold (`weather kinshasa`) | **no card at all.** The web results below are already about the place they asked for; a card about somewhere else is worse than none |

"Names nothing" and "names something unknown" are told apart by what is left of the query once
the trigger and the connective words of three languages are removed
(`weather::names_somewhere`).

**The world list is curated, not geocoded** (`xustive_tools::city`, ~90 cities): the Maghreb, the
Arab world, the diaspora's cities in France and beyond, and the capitals that appear in a
newsroom — with aliases for the short forms people type (`مكة` for `مكة المكرمة`, `Kuwait` for
`Kuwait City`). A geocoder would mean a live lookup on the search path, and the serving plane has
no route to the internet by design ([[Tool Data Plane]]). Adding a city is one line and a fetch
cycle.

## Related

[[Tool Data Plane]] · [[Knowledge Store]] · [[UI - Tool Cards]] · [[UI - Design Language]] ·
[[Query Pipeline]] · [[Summarizer]] · [[Query Expander]] · [[Security and Privacy]] ·
[[Performance Budgets]] · [[API Contract]] ·
[[ADR-0020 - Approximate Location from a Local Database]]
