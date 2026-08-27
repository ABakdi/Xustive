---
tags:
  - adr
adr-id: "0003"
status: partly implemented
date: 2026-08-06
---

# ADR-0003 - Comments in a Separate Index

## Status

Accepted; **partly implemented** — the index exists, the query-time fold does not. Constrains [[Data Model]], [[Query Pipeline]], [[Search Index]].

## Context

Comments matter here more than in most search engines. On Algerian social media, the *answer* to a
question is frequently in the replies, not the post: someone posts a job listing, and the comments
contain the actual application details. The functional spec requires indexing them
([[Xustive Search Engine – Technical Specification]] §2.3).

Two shapes are possible: nest comments inside their parent document, or index them separately.

Nesting is attractive — one query, one result, natural grouping. But it has three problems at our
scale:

1. Comments outnumber documents roughly **5:1** (50M vs 10M). A post with 500 comments makes one
   enormous document.
2. Meilisearch cannot facet or filter efficiently on nested array elements, so "negative comments in
   the last week" becomes impossible.
3. Comments arrive *after* their parent and keep arriving. Nesting means rewriting the whole document
   — including its body — every time a comment appears. At social volume that is a continuous
   rewrite storm against the same documents.

## Decision

Comments are a **separate Meilisearch index** with `document_id` as a foreign key
([[Data Model]] §3).

[[Query Pipeline]] issues a federated `multi-search` (documents leg + comments leg), groups comment
hits by `document_id`, fetches any parent documents not already in the result set, and folds up to
two matching comments into each result card ([[Query Pipeline]] §4.4).

Threading is one level; deeper replies are flattened with `parent_comment_id` retained.

## Consequences

**Good**
- Comments are independently searchable, filterable, and facetable by sentiment and date.
- A new comment is a small insert, not a large document rewrite.
- Document size stays bounded and predictable, which keeps index size and search latency predictable.
- Comment sentiment can be aggregated separately from post sentiment — a positive post with 200 angry
  replies is a meaningfully different result ([[Enrichment Pipeline]] §12).

**Bad**
- Two retrieval legs instead of one; the merge is our code, not the engine's.
- A parent fetch may be needed for comment-only matches — one extra round trip in the worst case.
- Deleting a document must also delete its comments; the ordering is now our responsibility
  ([[Indexer Worker]] §4.5).
- Ranking a document by its comments requires an explicit signal (`comment_evidence`,
  [[Ranking and Relevance]] §3) rather than falling out of the engine.

**Commits us to**
- Owning the merge logic and its latency budget (15 ms for 200 candidates).
- Keeping the two indexes consistent on delete — which is a privacy requirement, not just hygiene.

## Alternatives

| Option | Why not |
|:---|:---|
| Nest comments in the document | rewrite storm, no faceting, unbounded document size |
| Concatenate comment text into `body` | destroys per-comment sentiment, dates, and attribution; makes highlighting meaningless |
| Don't index comments | loses the answer for a large class of Algerian social queries |
| Separate index but no query-time merge (comments as their own results) | a results page mixing posts and orphan comments is confusing |

## Revisit when

- The comment leg's latency exceeds its budget at scale and the merge becomes the bottleneck.
- We find that comment matches almost never change which documents surface — in which case the leg
  could become an enrichment-time signal instead of a query-time one.

## Related

[[Data Model]] · [[Query Pipeline]] · [[Search Index]] · [[Ranking and Relevance]] ·
[[Indexer Worker]] · [[Decision Log]]

## Where it stands (2026-08-27)

- The `comments` index and its settings exist (`crates/xustive-search/src/settings.rs`: `COMMENTS`, `comments_settings()`), and the indexer can route a job to it (`IndexJob.index` in `crates/xustive-queue/src/indexer.rs`).
- The serving side does **not** run a comments leg: `crates/xustive-api/src/search.rs` always sets `matched_comments: Vec::new()`, there is no `multi_search` call anywhere in `crates/xustive-search` or `crates/xustive-api`, and `COMMENTS` is referenced only inside `xustive-search`. No connector writes comments today (the social connectors of [[ADR-0009 - Direct Collection for Social Platforms]] are not built), so the fold has had nothing to fold.
