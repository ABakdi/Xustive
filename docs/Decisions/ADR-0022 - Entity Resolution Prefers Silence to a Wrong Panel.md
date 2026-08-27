---
tags: [adr]
adr-id: "0022"
status: accepted
date: 2026-08-26
---
# ADR-0022 - Entity Resolution Prefers Silence to a Wrong Panel

## Status

Accepted, implemented. Constrains the resolver in `xustive-knowledge`, [[Instant Answers]] (the
entity panel) and the live path of [[ADR-0023 - Live Wikidata Fallback Judged by the Local Resolver]].
Sits under [[ADR-0019 - The Knowledge Layer]], which decided *where* entities come from; this
records *how a query is matched to one*, a set of rules that were each forced by a wrong panel
seen in the browser between 2026-08-26 and 2026-08-27.

## Context

A panel sits in the most trusted space on the page. A confident panel about the wrong thing is
worse than no panel, and worse than it looks, because the reader has no reason to doubt it. The
first resolver scored candidates and picked the top one; a sequence of real misses showed every
way that goes wrong:

- *messi* on the French page resolved to Jesus Christ (French search matched *Messie*;
  prominence did the rest).
- *zidane* resolved to a video game, because the person's names are "Zinedine Zidane" — neither
  an exact nor a prefix match for a bare surname — so he was invisible and a namesake won.
- *cast of the matrix* briefly resolved to a boxer nicknamed The Matrix when the kind lookup
  failed upstream and the preference had nothing to filter on.
- *the matrix* resolved to *The Matrix Reloaded* and *dune* to the 1984 film: every strong
  candidate scored a clamped 1.0 and the tie-break was the id string, where Q189600 sorts before
  Q83495.
- A maximally prominent bare label (a thing with a name and nothing else) beat a real entity when
  thin entities were merely penalised.

## Decision

**Resolve with a precision floor and a fixed rule order; when the rules do not produce a
confident answer, decline (`204`) rather than guess.**

1. **A cheap gate before any index round trip.** 2–60 characters, at most 8 words, no `?` or `؟`,
   and none of the question openers in all four languages — including the Darija forms
   `كيفاش`, `علاش`, `وين` a Modern-Standard list would miss. A question wants a paragraph and
   belongs to the summariser.
2. **Name-match tiers, far above everything else.** Over normalised names and aliases (lowercase,
   leading `ال` stripped, punctuation stripped): **exact** `0.70` → **whole-word** `0.60` (every
   query token is a whole token of some name — *zidane* in "Zinedine Zidane", but *messi* is not
   a whole word of *Messie*) → **prefix** `0.35` → else `0.10`. Prominence (`≤ 0.15`) and
   corpus agreement (`≤ 0.15`) are capped low on purpose: they decide *which* Oran, never
   *whether* this is Oran. The corpus signal is what makes an Algeria-first engine behave like
   one — of two cities sharing a name, the one the crawled Algerian web writes about wins.
3. **Thin entities are excluded before scoring, not penalised within it.** A penalty is a weight
   a strong enough match outvotes; "a bare label is not knowledge" is meant as a guarantee.
4. **A kind preference is a filter, then a lift.** When the relation names the kind (*cast of X*
   wants a film), candidates of other kinds are removed, and `+0.20` is added among what passes.
   If nothing of the expected kind is on offer, the honest answer is no answer. On the live path,
   **an incomplete kind lookup declines the whole shortlist** rather than choosing among the
   candidates whose kind is known.
5. **Order by the unclamped score**, then prominence (how many articles the thing has), then a
   small lift (`0.05`) for a release within the last two years (the conversation is usually about
   the new one), then the **earliest** release — a shared name means the original unless
   something above said otherwise — then id, so the panel cannot flicker between two entities on
   reload. Clamping to 1.0 happens after the sort, for the reported confidence only.
6. **Confidence floor `0.55`;** a runner-up within `0.15` is surfaced as *also*, not silently
   dropped.

## Consequences

**Good**
- Every rule is pinned by a test named for the case that forced it (the sequel never outranks
  the original it is named after; recency never lifts a partial name over an exact one; a
  relation lifts the kind it expects over an exact name of the wrong kind; *messi* ≠ *Messie*).
- Declining is cheap: p95 31 ms against a 100 ms gate, measured on the store path.

**Bad**
- Questions and long queries never get a panel, by design; a reader who types *who is zidane*
  gets the summariser instead.
- The whole-word tier makes bare surnames work but also makes a common surname ambiguous more
  often; the `also` runner-up is the mitigation.
- "Earliest wins on a tie" is a Western-cinema heuristic. It is right for *The Matrix* and
  *Dune*; it will be wrong for a remake that eclipsed its original, and the recency lift only
  covers the last two years.

## Alternatives

| Option | Why not |
|:---|:---|
| Rank by prominence (sitelink count) | what the first live fallback did; produced Jesus for *messi* |
| Penalise thin entities instead of excluding | a bare label with maximal prominence still won |
| Kind preference as a lift only | a wrong-kind exact match still won when the kind lookup failed |
| Ask the model to disambiguate every query | slower, occupies a GPU slot, and reproduces a `match` statement; kept only for near-ties ([[ADR-0019 - The Knowledge Layer]] §7) |

## Revisit when

- A per-language precision corpus shows the floor declining more right answers than it saves.
- Arabic aliases in the store are rich enough that the whole-word tier stops being needed for
  surnames.
- The earliest-wins tie-break is measured to be wrong for a class of titles (remakes, sequels
  that became the reference).

## Where it stands (2026-08-27)

`crates/xustive-knowledge/src/resolve.rs` (`is_panel_shaped`, `score`, `choose_preferring_at`,
`MIN_CONFIDENCE`, `AMBIGUOUS_WITHIN`, `KIND_PREFERENCE`, `RECENCY`, `RECENT_WITHIN_SECS`); the
decline-on-incomplete-kind rule is in `web/app/api/knowledge-live/route.ts` (`instanceOfMany`,
"kind lookup incomplete; declining"). Commits `f396788`, `f3c6dca`, `a231f0b`. Two stale doc
comments still call the kind hint "a lift, never a filter" (`knowledge-live/route.ts` header and
`choose_preferring` in `resolve.rs`); the code filters.

## Related

[[ADR-0019 - The Knowledge Layer]] · [[ADR-0023 - Live Wikidata Fallback Judged by the Local Resolver]] ·
[[Instant Answers]] · [[Milestone 8 - The Answer Layer]] · [[Decision Log]]
