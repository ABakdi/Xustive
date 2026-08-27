---
tags: [adr]
adr-id: "0026"
status: accepted
date: 2026-08-26
---
# ADR-0026 - The Reader's Language as a Bounded Ranking Signal

## Status

Accepted, implemented. Constrains [[Ranking and Relevance]] and [[Query Pipeline]]; touches
[[Summarizer]] (the summary's output language follows the same choice). Keeps the invariant
[[ADR-0015 - Anonymous Interaction Signals for Ranking]] stated for the `interaction` term:
textual relevance dominates by construction.

## Context

The nav-bar language (`ar`, `ary`, `fr`, `en`) set the interface and nothing else. For a query
that exists in two languages — *ronaldo*, *oran* — a French reader got the same order as an
English one, which is wrong in a specific way: among pages that answer equally well, the one
written in the reader's language is the one they can read. Meanwhile the AI summary followed the
*detected* language of the query, so a French reader asking about an Arabic topic got the answer
in Arabic.

The tempting fix is to filter or hard-sort by language. Both are wrong here: a French reader
searching an Arabic name still wants the best Arabic page when no French page is as good, and a
filter would make the engine look empty for exactly the bilingual queries Algerians run.

## Decision

**Add a `ui_language` re-ranking term: binary (the document's language equals the reader's chosen
language), bounded, and small enough that it reorders equals and nothing else. The summary is
generated in the reader's language, not the query's.**

- Darija (`ary`) and Arabic (`ar`) count as each other's language on both sides of the comparison.
- Weight `0.10`, folded into the side budget by rebalancing the others down (freshness `0.10`,
  trust `0.06`, authority `0.09`, quality `0.05`, interaction `0.07`) so the additive side weights
  still sum to `0.47`, under the `~0.48` relevance gap across twenty positions. That bound is what
  makes "relevance dominates" true by construction; the term may not be raised past it without
  rebalancing.
- The weight is tunable at startup from `config/ranking.toml` like the other weights, and shown
  in `--explain` output.
- No filter, no hard sort, no per-language index.

## Consequences

**Good**
- For *ronaldo*, `ui=fr` puts seven French pages in the top ten where `ui=en` puts four, and the
  best Arabic page is still on the page, just after the French pages that are as good.
- The summary reads in the language the reader chose.

**Bad**
- The rebalance took a point from freshness, not authority (the first attempt took it from
  authority and a tie-break test caught it). Any future signal must find its budget the same way.
- A document's `language` field must be right for this to help; a mis-detected page is quietly
  demoted for a reader of its real language.

## Alternatives

| Option | Why not |
|:---|:---|
| Filter results to the UI language | empties the page for bilingual queries; the Arabic web is most of the Algerian web |
| Hard sort by language, then relevance | puts a weak French page above a strong Arabic one |
| Use the query's detected language instead | already what the summary did, and it was the complaint: the reader chose a language and the engine ignored it |

## Revisit when

- Language detection quality on the corpus is measured to be poor enough that the term demotes
  more correct pages than it lifts.
- A per-language evaluation set shows the `0.10` weight is too timid or too strong — retune in
  `config/ranking.toml`, keeping the side-budget bound.

## Where it stands (2026-08-27)

`crates/xustive-search/src/rank.rs` (`Weights::ui_language`, `lang_match` in `rerank`, the
`ary`→`ar` fold), called from `crates/xustive-api/src/search.rs` with `params.ui`; the summary
language switch is in the same file via `xustive_ml::OutputLang::from_ui`. Commit `29de58f`.

## Related

[[Ranking and Relevance]] · [[Query Pipeline]] · [[Summarizer]] ·
[[ADR-0015 - Anonymous Interaction Signals for Ranking]] · [[Decision Log]]
