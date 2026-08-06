---
tags:
  - engineering
  - quality
type: guide
status: specified
updated: 2026-08-06
---

# Testing Strategy

> What we test, at which level, and which gates block a merge.
> Parent: [[Home]] · Per-component test plans live in each component note's §11.

---

## 1. The Shape of the Problem

Conventional pyramids assume correctness is binary. Half of Xustive's risk is not — "are these the
right results?", "is this summary faithful?", "is this Darija correctly detected?" are *quality*
questions with no pass/fail assertion. So the strategy has two tracks:

| Track | Question | Gate |
|:---|:---|:---|
| **Correctness** | does it do what it says? | hard pass/fail, blocks merge |
| **Quality** | is the output good? | metric thresholds + regression bands, blocks merge on regression |

Ignoring the second track is how a search engine passes all its tests and returns bad results.

---

## 2. Levels

| Level | Scope | Runtime | When |
|:---|:---|:---|:---|
| Unit | one function/module, no I/O | < 30 s total | every save, every PR |
| Integration | one component + real dependency (container) | < 5 min | every PR |
| Contract | HTTP surface vs [[API Contract]] | < 1 min | every PR |
| Pipeline E2E | fixture site → crawl → index → search | < 10 min | every PR |
| Quality | relevance, sentiment, OCR, WER, faithfulness | < 20 min | nightly + before merge on relevant paths |
| Load | throughput and latency vs [[Performance Budgets]] | < 30 min | nightly + pre-release |
| Manual | screen reader, native-speaker review, restore drill | — | per milestone |

---

## 3. Unit

Standard `#[test]`, no network, no containers, deterministic. Notable focus areas:

- `xustive-text` normalisation tables (the highest-leverage tests in the repo)
- ranking formula and diversity caps ([[Ranking and Relevance]])
- transliteration rules ([[Query Expander]])
- date parsing across Arabic/French/relative formats ([[Content Parser]] §4.3)
- URL canonicalisation and SimHash banding ([[Deduplication Service]])
- retry/backoff classification ([[Error Handling and Resilience]] §1)

**Property tests** (`proptest`) where invariants are clean:
- `normalize(normalize(x)) == normalize(x)`
- `parse_normalize(x) == query_normalize(x)` ← the symmetry test; its failure means silent search breakage
- SimHash distance is symmetric and `d(x,x) == 0`
- ranking is a total order with no NaN scores
- no input of any bytes causes a panic in the parser, the URL validator, or the media decoders

---

## 4. Integration

Real dependencies via `testcontainers`: Meilisearch, Qdrant, Redis. No mocks at this level — a mocked
Meilisearch tests our idea of Meilisearch, which is exactly the thing that turns out to be wrong.

Representative cases:

| Test | Asserts |
|:---|:---|
| Index 10k fixture docs, run golden queries | expected top hits |
| Malformed doc inside a batch of 1 000 | 999 index, 1 to DLQ ([[Indexer Worker]] §4.3) |
| Kill a consumer mid-message | redelivery, no loss ([[Task Queue]] §11) |
| Delete a document with images and comments | all three stores clean + URL blocklisted |
| Redis at `maxmemory` | writes fail loudly, nothing evicted |
| Meilisearch down | search returns 503, ingestion buffers, nothing lost |

---

## 5. Contract

Every row of [[API Contract]] §8 has a test asserting status, `code`, and body shape. Response schemas
are snapshot-tested so an accidental field rename fails CI rather than the UI.

The UI consumes the same fixtures the contract tests produce — one source of truth for what the API
returns.

---

## 6. Pipeline E2E

Against the local fixture site ([[Local Development]] §5), fully offline:

```
fixture site → crawl → parse → dedup → enrich → index → search → assert
```

Asserts: a known page is findable by a known query within 60 s; robots-disallowed pages are absent;
a duplicate page is collapsed; a `noindex` page is fetched but not indexed; a 429 endpoint triggers
backoff rather than hammering.

---

## 7. Quality Track

| Suite | Data | Metric | Gate |
|:---|:---|:---|:---|
| **Relevance** | 200 judged queries, 4 languages ([[Ranking and Relevance]] §6) | nDCG@10 | no drop > 1 % absolute |
| **Expansion** | Darija/Arabizi slice | recall@50 | +15 % vs no expansion; nDCG not down > 1 % |
| **Language detection** | 1 000 labelled strings | accuracy | ≥ 92 % overall, ≥ 85 % `ary` |
| **Sentiment** | 1 000 labelled items | macro-F1 | ≥ 0.70 lexicon; no language < 0.60 |
| **OCR** | 200 images | CER on screenshots | ≤ 15 % |
| **Speech** | 100 recordings | WER | ar ≤ 25 %, fr ≤ 20 %, ary ≤ 45 % |
| **Summary faithfulness** | 100 (query, passages) | % with no unsupported claim | ≥ 95 %, sampled human review |
| **Dedup** | 500 dup + 500 distinct pairs | precision / recall | ≥ 0.95 / ≥ 0.85 |
| **Spam** | 300 labelled posts | precision @ 0.8 threshold | ≥ 0.90 |

Results are written to `eval/reports/{date}.json` and plotted over time. **A quality gate failure
blocks the merge in the same way a failing unit test does** — this is the mechanism that stops
"small lexicon tweaks" from quietly degrading the product.

Golden sets are versioned in git and grow by rule: **every real-world quality complaint becomes a new
row**. That is how the suites stay relevant instead of becoming a fossil of launch-day assumptions.

---

## 8. Security Tests

| Suite | Asserts |
|:---|:---|
| SSRF | private IPs, redirects to private IPs, DNS rebinding, decimal/IPv6 literals — all blocked ([[Security and Privacy]] §4) |
| Egress | `xustive-api` and `xustive-ml` cannot reach the public internet — **passes only if the connection fails** |
| Telemetry lint | no query/transcript/OCR identifiers inside `tracing::` calls |
| Log scan | run the query corpus, grep 24 h of logs for any corpus string → zero hits |
| Disk scan | after voice/image requests, no new files in the writable layer |
| Prompt injection | hostile passages produce a clean summary or none ([[Summarizer]] §11) |
| Upload bombs | decompression bombs, malformed media, wrong extensions → clean 4xx, no panic |
| XSS | crawled `<script>` in a title renders as text |
| Dependencies | `cargo-audit`, `cargo-deny` (licences + advisories) |

---

## 9. Frontend Tests

| Layer | Tool | Gate |
|:---|:---|:---|
| Unit | vitest | logic: URL state, formatting, escaping |
| Accessibility | `axe-core`, 4 languages × 2 themes | zero violations ([[UI - Accessibility]] §9) |
| Visual regression | Playwright screenshots, all languages, both directions | manual approval on diff |
| Bundle size | `bundlesize` | ≤ budgets in [[UI Specification]] §4 |
| Lighthouse CI | throttled mid-range Android profile | LCP ≤ 2.0 s, CLS ≤ 0.05 |
| No-JS | Playwright with JS disabled | core search works |
| RTL lint | CSS scan | no physical-direction properties |

---

## 10. Load Tests

Nightly against staging, per [[Performance Budgets]]:

- 500 rps search for 10 min → p95 ≤ 200 ms
- 2 000 rps suggest → p95 ≤ 40 ms
- 20 concurrent summaries → drop rate ≤ 2 %, **search latency unaffected**
- 2 000 docs/s indexing **while** serving 500 rps → search p95 holds
- crawler at 100 pages/min/worker → politeness never violated

The "while" cases matter most: components are fine alone and interfere under contention. Indexing
starving search is the failure mode [[Search Index]] §5 caps threads to prevent, and this is where it
gets verified.

---

## 11. CI Pipeline

```
PR:      fmt → clippy → unit → contract → integration → security → frontend → bundle
         + quality gates for touched areas (data/** → relevance; ml → its suite)
Nightly: full quality track + load + Lighthouse + dependency audit
Release: nightly + restore drill + manual a11y pass + native-speaker string review
```

Target: PR feedback in **≤ 10 minutes**. Anything slower gets ignored or worked around, which is
worse than not having it.

---

## 12. Fixtures

`tests/fixtures/` — the project's most valuable asset after the code:

| Directory | Contents |
|:---|:---|
| `site/` | the offline fixture web site |
| `corpus/` | 10k sample documents for seeding |
| `html/` | 200 real Algerian pages with labelled expectations |
| `facebook/ instagram/ tiktok/` | recorded API payloads including error envelopes |
| `audio/` | 100 recordings + reference transcripts |
| `images/` | 200 images including adversarial ones |
| `poison/` | payloads that once crashed something — **every DLQ investigation adds one** |
| `injection/` | prompt-injection passages |
| `bidi/` | mixed-direction strings |

---

## 13. Open Questions

- [ ] Who produces the judged relevance set, and how do we handle judge disagreement on Darija?
- [ ] Can we measure real-world relevance without logging queries? (Proposal: zero-result rate by
      language only, aggregate and k-anonymous — [[Observability]] §8.)
- [ ] Is 95 % summary faithfulness acceptable, or should the bar be higher before beta given that a
      wrong summary is the most visible possible failure?

## Related

[[Performance Budgets]] · [[Ranking and Relevance]] · [[Local Development]] ·
[[Security and Privacy]] · [[UI - Accessibility]] · [[Observability]] · [[TODO]]
