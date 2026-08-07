---
tags:
  - component
  - serving
  - ui
component-id: C28
binary: xustive-api
status: specified
updated: 2026-08-07
---

# Instant Answers

> **ID** C28 · **Binary** `xustive-api` + Next frontend · **Upstream** [[Query Pipeline]] ·
> **Downstream** [[Tool Data Plane]]

## 1. Purpose

Answer the query directly when the query *is* the question. `45 * 1.19` wants a number, not ten
pages about multiplication. `مواقيت الصلاة بجاية` wants today's times. `20 euros en dinar` wants a
rate — and in Algeria, wants **two** rates.

## 2. The rule that governs everything here

**A tool must be right, or absent.** Never approximately right, never confidently stale.

A search result that is mediocre costs the user a click. A calculator that is wrong, or a currency
rate that is three weeks old presented as today's, destroys the reason to use the product at all.
Every design choice below follows from this:

- A tool that cannot answer **renders nothing**. Not an error, not a placeholder — the results
  are already there.
- Any value with a time dimension shows **when it was measured**, always, not only when stale.
- A tool never guesses at ambiguity. `1500` is not a currency conversion.

## 3. Interface

Instant answers arrive **with** the search response, not after it. They are computed in
microseconds to low milliseconds and blocking on them costs nothing:

```jsonc
GET /api/v1/search?q=45*1.19
{
  "query": { … },
  "instant": {
    "tool": "calculator",
    "confidence": 0.99,
    "data": { "expression": "45 × 1.19", "result": "53.55" },
    "as_of": null,              // null = timeless; a value means "measured at"
    "sources": []
  },
  "results": [ … ]
}
```

A tool needing live data ([[Tool Data Plane]]) is served **only from cache** on this path. If the
cache is cold, `instant` is `null` and the client may request it separately — search never waits
on the network for a side feature.

## 4. Routing: which tool, if any

### 4.1 Matching

Each tool declares a matcher: `fn match(normalised: &str, lang: Lang) -> Option<Match>` returning
a confidence in `0..=1`. Matchers are **pure, total and fast** — no I/O, no panics, ≤ 100 µs.

Three matcher kinds, in descending trust:

| Kind | Example | Confidence |
|:---|:---|:---|
| **Structural** — the string parses as the tool's grammar | `45*1.19` parses as an expression | 0.95–1.0 |
| **Keyed** — an explicit trigger word plus an operand | `translate hello to arabic`, `طقس وهران` | 0.8–0.95 |
| **Inferred** — shape suggests intent without a trigger | `20 eur dzd` | 0.5–0.75 |

### 4.2 Arbitration

Matchers run in parallel; the highest confidence wins, ties broken by a fixed precedence order.
Below **0.5, nothing renders** — an unwanted tool card pushing results down is worse than no card.

The order exists because overlaps are real and the wrong resolution is embarrassing:

```
calculator > unit-convert > currency > prayer-times > weather > time-date
  > translate > define > transliterate > reference > utility
```

`5 km en miles` matches both calculator (a number) and unit conversion. Unit conversion wins on
confidence because it consumed the whole string; the calculator only matched a fragment. **A
matcher's confidence must reflect how much of the query it explains**, which is what stops the
calculator from hijacking every query containing a digit.

### 4.3 Explicit invocation

`!calc`, `!tr`, `!weather` force a tool and skip arbitration — for the case where inference gets
it wrong and the user knows what they want. Discoverable from the tool card's overflow menu, not
required.

## 5. The tools

### 5.1 Tier 1 — ship first

These are chosen because they are needed often, served badly today, and we can be authoritative
about them.

| Tool | Triggers | Notes |
|:---|:---|:---|
| **Calculator** | `45*1.19`, `15% of 2000`, `sqrt(64)` | Arbitrary precision on decimals. See §6. |
| **Unit converter** | `5 km en miles`, `30°C to F`, `2 قنطار كيلو` | Length, mass, temperature, area, volume, speed, time, data. Includes **qintar** and **sa'a**, which no international converter carries. |
| **Currency** | `20 eur dzd`, `سعر الأورو`, `100 dollar` | **Shows official and parallel-market rates side by side.** See §7 — this is the single most Algeria-specific thing the product does. |
| **Translator** | `translate X to ar`, `ترجم X`, `!tr` | Any language pair, local model, no text leaves the machine. See §8. |
| **Weather** | `طقس وهران`, `météo Alger`, `weather Batna` | Now plus 5 days, per wilaya. Icons, not photographs. |
| **Prayer times** | `مواقيت الصلاة`, `heure priere Setif` | Five daily times by wilaya, computed locally. See §9. |
| **Time & date** | `what time is it`, `days until ramadan`, `12 août en hijri` | World clock, date arithmetic, **Hijri ↔ Gregorian**. |

### 5.2 Tier 2 — Algeria reference

| Tool | Triggers | Data |
|:---|:---|:---|
| **Wilaya reference** | `code postal bejaia`, `ولاية 06` | 58 wilayas: code, postal range, dial code, seat, coordinates. Static, ships in the binary. |
| **Fuel prices** | `prix essence`, `سعر البنزين` | Regulated and near-static; changes are news. |
| **Dictionary** | `définition X`, `معنى X`, `شنو معنى` | ar / ary / fr / en. **Darija definitions are the differentiator** and the hardest part ← *B7* |
| **Transliterator** | `ch7al fi larabiya`, `!translit` | Arabizi ↔ Arabic, exposed as a visible tool. The engine already does this internally ([[Query Expander]]); surfacing it is nearly free. |
| **Sports** | `résultats CAN`, `نتائج البطولة` | Ligue 1, CAN, national team fixtures. |
| **Exam results** | `نتائج البكالوريا` | Seasonal, enormous traffic, and the query where being unavailable is most conspicuous. Links to official portals; **never mirrors personal results**. |

### 5.3 Tier 3 — cheap utilities

No network, no data plane, a few dozen lines each. They cost almost nothing and each one is a
query that would otherwise leave for another site.

`QR code` · `colour converter` (hex/rgb/hsl/oklch) · `word & character count` ·
`case conversion` · `Base64 encode/decode` · `URL encode/decode` · `JSON formatter` ·
`hash` (sha256/md5) · `random number & dice` · `coin flip` · `timer & stopwatch` ·
`VAT calculator` (Algerian TVA 19 % / 9 %) · `tip split` · `loan repayment` · `BMI` ·
`percentage change` · `Roman numerals` · `what is my IP`

**Deliberately excluded**, with reasons:

| Not building | Why |
|:---|:---|
| Password generator | Generating a secret in a *search box* trains exactly the wrong instinct. |
| Stock tickers | We cannot be authoritative and being late is worse than being absent. |
| Medical / dosage calculators | Wrong output causes physical harm. Not a search-box feature. |
| Live flight tracking | Needs a paid feed we cannot verify; a wrong gate number is worse than none. |

## 6. Calculator

- **Decimal arithmetic**, not binary floats. `0.1 + 0.2` renders `0.3`. A calculator that shows
  `0.30000000000000004` is a calculator nobody trusts again.
- Grammar: `+ - × ÷ ^ % ( )`, `sqrt`, `abs`, `min`, `max`, `log`, trigonometry in degrees by
  default with radians available.
- Percentages read as people write them: `15% of 2000` → 300; `2000 + 19%` → 2380.
- **Arabic-Indic digits accepted** (`٤٥` = 45) and echoed in the script they were typed in.
- **No variables, no state, no user-defined functions.** An expression evaluator in a query string
  is an attack surface; keeping it a pure calculator keeps it one.
- Division by zero, overflow and malformed input render **nothing**.

## 7. Currency — the parallel market

Algeria has two exchange rates: the official Bank of Algeria rate, and the parallel-market rate
("square" / السوق الموازية) which is what people actually transact at and is frequently 40–60 %
different.

**A converter showing only the official rate is wrong in the way that matters.** Someone asking
what 100 € is worth is almost never asking about the official rate.

So the card shows **both, side by side, equally weighted**, each with its own `as_of` and source.
Neither is labelled "the" rate.

Constraints:

- The parallel rate has no authoritative publisher. It is aggregated from observed reporting,
  and the card says so in plain language — the provenance line is not fine print.
- If either rate is older than **6 hours**, it is labelled with its age in words, not hidden.
- Older than **48 hours**: that rate is withheld entirely rather than shown stale.
- Rates are never presented as advice, and no tool ever suggests where to transact.

## 8. Translator

Runs on the **local Qwen model already loaded for [[Summarizer]]** — same engine, same device
setting, same slot pool ([[Deployment Topology]]).

This is the whole point: **no text typed into a translation box leaves the machine.** Every hosted
translator is a service that receives everything you translate, and people translate medical
letters, legal documents and messages from family.

- Any language pair the model handles; ar / ary / fr / en / es / tr are the tested set.
- **Darija is a first-class target**, unlike every mainstream translator, and it is the hardest to
  evaluate ← *B7*
- Long text is refused with a length hint rather than silently truncated.
- Latency is what it is — tens of seconds on CPU (see [[Summarizer]] §8). The card streams and can
  be cancelled; the search results are already on the page.
- Detected source language is shown and can be overridden.

## 9. Prayer times

Computed locally from coordinates and date — no network, no per-request lookup. The algorithm
choice is user-visible because it genuinely differs:

- Default **Umm al-Qura** offsets; **Muslim World League** and **Egyptian General Authority**
  selectable, since Algerian mosques do not all follow one.
- Asr per Shafi'i or Hanafi.
- Wilaya coordinates from the static reference table; never geolocation without consent.
- The calculation method is **displayed on the card**, not buried in settings. Times differing by
  a few minutes from a local mosque is normal and expected; a card that does not say which method
  it used makes that look like an error.

## 10. Rendering

Tool cards sit **above the results and below the search box**, carrying the qalam rule from
[[UI - Design Language]] §6 — the mark that means the engine is asserting rather than listing.

- One card maximum. Never a stack.
- **Reserves no height until it has content.** A card that appears late and pushes results down is
  worse than no card ([[Performance Budgets]] CLS ≤ 0.05).
- Every card carries: the answer at `--text-2xl`, the interpretation of the query in small text
  (`45 × 1.19` — so a misread is visible), source and `as_of` where applicable, and a copy button.
- Interactive cards (converter, translator) update **without a new search**.
- Fully keyboard-operable and labelled ([[UI - Accessibility]]).

## 11. Privacy

- Matchers run **in-process**. A query is never sent anywhere to find out whether a tool applies.
- Tools never make per-query outbound requests. Live data comes from a cache the
  [[Tool Data Plane]] fills on a schedule, so a weather query reveals nothing to anyone outside.
- Translation runs locally. Nothing is logged, including the text.
- Location is **wilaya-level and explicit** — chosen from a list or parsed from the query. No IP
  geolocation, no browser geolocation prompt on page load.
- Tool usage is counted by *tool name only* — never with the operand ([[Security and Privacy]]).

## 12. Failure

| Failure | Response |
|:---|:---|
| Matcher panics | Caught; that tool is skipped. A tool cannot take down search. |
| Data stale beyond limit | Value withheld; card renders without it or not at all |
| Model busy (translate) | Card shows a retry affordance; results unaffected |
| Ambiguous match | Below 0.5 confidence, nothing renders |
| Tool disabled by config | As if it did not exist |

Every one is invisible beyond a missing card.

## 13. Testing

- **Golden expressions**: several hundred `(input, expected)` pairs for the calculator, including
  the float traps. Any mismatch fails the build.
- **Matcher precision**: a corpus of ordinary queries that must match **no** tool. Regression here
  means the product started interrupting normal searches, which is the failure mode users notice
  most.
- Unit conversions checked against reference values including the Algerian units.
- Prayer times cross-checked against published tables for 5 wilayas across a year.
- Stale-data behaviour tested with a frozen clock.

## 14. Open questions

- [ ] Where does the parallel rate come from, and can it be sourced without depending on a party
      that could manipulate it? This gates the currency tool's credibility.
- [ ] Should the calculator accept a leading `=` like a spreadsheet? Cheap, discoverable, and
      possibly confusing.
- [ ] Does the translator card need a "report a bad translation" affordance, given no logging?
- [ ] Do exam-result queries during results week need special capacity handling? That is one day
      of extreme, predictable traffic.

## Related

[[Tool Data Plane]] · [[UI - Tool Cards]] · [[UI - Design Language]] · [[Query Pipeline]] ·
[[Summarizer]] · [[Query Expander]] · [[Security and Privacy]] · [[Performance Budgets]] ·
[[API Contract]]
