---
tags:
  - engineering
  - quality
type: guide
status: specified
updated: 2026-08-27
---

# Testing Strategy

> What we test, at which level, and which gates block a merge.
> Parent: [[Home]] · Per-component test plans live in each component note's §11.
>
> **Verified against `Makefile`, `.github/workflows/ci.yml`, `scripts/`, `eval/` and
> `tests/fixtures/`, 2026-08-27.** Each section keeps the plan and adds an "as built" line; suites
> that do not exist yet say so with the date rather than reading as if they run.

---

## 1. The Shape of the Problem

Conventional pyramids assume correctness is binary. Half of Xustive's risk is not — "are these the
right results?", "is this summary faithful?", "is this Darija correctly detected?" are *quality*
questions with no pass/fail assertion. So the strategy has two tracks:

| Track | Question | Gate |
|:---|:---|:---|
| **Correctness** | does it do what it says? | hard pass/fail, blocks merge |
| **Quality** | is the output good? | metric thresholds + regression bands, blocks merge on regression |
| **the deployment images still build** | CI, every push | the tool fetcher's image failed for two weeks and nobody noticed until world-city weather went quiet; the API's failed on a missing `rustfmt`, a missing `make`, and a runtime base one Debian release behind the builder |

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

As built (2026-08-27): Unit + integration are one `cargo test --workspace --all-features` in CI;
contract and fault-injection tests live in `crates/xustive-api/tests/`; pipeline E2E is the CI
`end-to-end` job (§6); quality is `make eval-check`, run by hand; load is `make load`, run by
hand; the frontend gates are `make ui-gates`, run by hand against a running web server.

---

## 3. Unit

Standard `#[test]`, no network, no containers, deterministic. Notable focus areas:

- `xustive-text` normalisation tables (the highest-leverage tests in the repo)
- ranking formula and diversity caps ([[Ranking and Relevance]])
- transliteration rules ([[Query Expander]])
- date parsing across Arabic/French/relative formats ([[Content Parser]] §4.3)
- URL canonicalisation and SimHash banding ([[Deduplication Service]])
- retry/backoff classification ([[Error Handling and Resilience]] §1)

**Property tests** (`proptest`, in `xustive-text` and `xustive-lang` today) where invariants are
clean:
- `normalize(normalize(x)) == normalize(x)`
- `parse_normalize(x) == query_normalize(x)` ← the symmetry test; its failure means silent search breakage
- SimHash distance is symmetric and `d(x,x) == 0`
- ranking is a total order with no NaN scores
- no input of any bytes causes a panic in the parser, the URL validator, or the media decoders

---

## 4. Integration

Real dependencies, no mocks — a mocked Meilisearch tests our idea of Meilisearch, which is exactly
the thing that turns out to be wrong.

As built (2026-08-27): not `testcontainers`. The `*_redis.rs` tests in `crates/xustive-ingest/tests/`
(`frontier`, `dedup`, `simhash`, `raw_store`, `budget_store`, `bandwidth`, `commoncrawl`,
`crawl_stats`, `embed_cache`, `interaction`, `proxy_breaker`) connect to `REDIS_URL` (default
`redis://127.0.0.1:6390`, the compose dev port) and **skip with a message when Redis is absent**, so
`cargo test` passes on a box without infra and only proves the invariant when it is up. **The CI
`unit · integration` job does not start Redis, so those tests skip there** — they are only exercised
on a developer box with `make dev-up` running. Closing that gap is a one-line CI change.

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

Against the local fixture site ([[Running Xustive]] §8; `tests/fixtures/site/` with its own
`robots.txt`, sitemap, feed, a `private/` tree and a `trap/`), fully offline — the
`fixture_site.rs` test in `xustive-ingest`.

The CI `end-to-end` job is the other half: `scripts/gen_corpus.py --count 2000` → `migrate` →
`migrate --check` (settings drift) → `seed` → start the API → `scripts/smoke.sh` (some forty
assertions: a known query returns known documents, filters narrow, errors match the contract, the
privacy headers are present). `make smoke` runs the same suite against any running API.

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

**As built (2026-08-27) — only the relevance row exists, and it is a regression detector, not a
quality measure.** `eval/golden/v1.jsonl` holds 201 queries, every one `judged_by: "machine"`
(`eval/build_golden.py` grades documents by term overlap — see `eval/README.md` for why that is
circular for most queries and genuinely informative for the Arabizi-vs-Arabic and orthographic-
variant ones). `make eval` writes `eval/reports/YYYY-MM-DD.json`; `make eval-check` fails when
nDCG@10 drops against `eval/reports/baseline.json`; `make eval-ab` A/B-tests index-settings
variants; `make calibrate` tunes the side-weights against SearXNG's ordering (needs the
`federation` profile); `make golden` regenerates the set. `eval/serp-queries.txt` (48 queries) is
the Google yardstick ([[Ranking and Relevance]]). **None of this runs in CI** — it is run by hand
before a ranking change lands. The other rows (expansion, language detection, sentiment, OCR,
speech, faithfulness, spam) have no suite yet; dedup precision/recall and extraction accuracy have
tests (`dedup_quality.rs`, `extraction_accuracy.rs`, `freshness_eval.rs`, `robots_conformance.rs`)
without a dated report.

Golden sets are versioned in git and grow by rule: **every real-world quality complaint becomes a new
row**. That is how the suites stay relevant instead of becoming a fossil of launch-day assumptions.

---

## 8. Security Tests

| Suite | Asserts |
|:---|:---|
| SSRF | private IPs, redirects to private IPs, DNS rebinding, decimal/IPv6 literals — all blocked ([[Security and Privacy]] §4) — *a dedicated suite is not verified to exist (2026-08-27)* |
| Egress | the `core` network cannot reach the public internet — **passes only if the connection fails**. `scripts/test-egress.sh`, CI job `egress guarantee`. Caveat: CI brings the topology up `--no-start`, so the real-container and container-log probes skip there; and it proves the *containers* are sealed, not the host-run API |
| Telemetry lint | no query/transcript/OCR identifiers inside `tracing::` calls — `scripts/lint-telemetry.sh`, in `make lint` and CI |
| Log scan | `scripts/scan-logs.sh` (`make scan-logs LOG=…`): forbidden field names + corpus grep. Operator-run nightly, **not in CI** |
| Disk scan | ❌ not built (2026-08-27) |
| Prompt injection | hostile passages produce a clean summary or none ([[Summarizer]] §11) — ❌ no `injection/` fixtures yet |
| Upload bombs | decompression bombs, malformed media, wrong extensions → clean 4xx, no panic — hostile *markup* is covered (`adversarial.rs`: the parser must terminate); media bombs are not |
| XSS | crawled `<script>` in a title renders as text |
| Dependencies | `cargo-deny` advisories + licences + bans + sources: `make audit`, CI job `dependency audit` |
| Topology | `scripts/lint-compose.sh`: the base compose file publishes no port for a backing service |
| Alerts | `scripts/check-alerts.sh` (rule tests) and `scripts/lint-runbooks.sh` (every alert has a runbook and vice versa) |

---

## 9. Frontend Tests

| Layer | Tool | Gate |
|:---|:---|:---|
| Unit | vitest — ❌ not set up (2026-08-27; `web/package.json` has `build` and `lint` only) | logic: URL state, formatting, escaping |
| Accessibility | `axe-core`, 4 languages × 2 themes — ❌ not set up | zero violations ([[UI - Accessibility]] §9) |
| Contrast | `scripts/contrast-audit.mjs` (in `make ui-gates`) — WCAG AA over the oklch tokens, both themes | fails on a failing token pair |
| Visual regression | Playwright screenshots — ❌ not set up | manual approval on diff |
| Bundle size | `scripts/bundle-budget.sh` (in `make ui-gates`) | JS home 185 KB · results 195 KB · CSS 20 KB · fonts RTL 95 / LTR 50 KB gz ([[Performance Budgets]] §6) |
| Lighthouse CI | ❌ not set up | LCP ≤ 2.0 s, CLS ≤ 0.05 |
| No-JS | `scripts/no-js-check.sh` (in `make ui-gates`): fetch the results page with no script execution | core search works |
| RTL | `scripts/rtl-icons.sh` (directional icons mirrored) in `make ui-gates`; `scripts/lint-bidi.sh` in `make lint` | no un-mirrored glyph, no physical-direction properties |

`make ui-gates` needs the web server running on :3000 and is run by hand — it is not in CI.

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

As built (2026-08-27): `make load S=search|suggest|summary|mixed [RPS=n DUR=s]` drives
`xustive-loadgen` against the API on :8080 ([[Load Generator]]), run by hand on the dev box. There
is no nightly staging run. The "indexing while serving" case is the one that produced BUG-041 and
[[ADR-0027 - Narrow the Search Under Load Instead of Failing]] — found by using the box, not by a
suite.

---

## 11. CI Pipeline

```
PR:      fmt → clippy → unit → contract → integration → security → frontend → bundle
         + quality gates for touched areas (data/** → relevance; ml → its suite)
Nightly: full quality track + load + Lighthouse + dependency audit
Release: nightly + restore drill + manual a11y pass + native-speaker string review
```

As built — `.github/workflows/ci.yml`, 2026-08-27, five jobs on every PR:

```
fmt · clippy · lint   cargo fmt --check → clippy -D warnings → build without the summariser
                      → lint-telemetry → lint-compose → lint-docs → lint-runbooks
dependency audit      cargo-deny (advisories, licences, bans, sources)
egress guarantee      compose up --no-start → scripts/test-egress.sh
unit · integration    cargo test --workspace --all-features (Redis-backed tests skip without Redis)
end-to-end            2 000-doc corpus → migrate → migrate --check → seed → API → scripts/smoke.sh
```

`make check` (= `lint` + `test`) is the local equivalent. There is no nightly workflow; quality
(`make eval-check`), load (`make load`), the UI gates (`make ui-gates`), the log scan
(`make scan-logs`) and the restore drill (`make restore-drill`) are run by hand.

Target: PR feedback in **≤ 10 minutes**. Anything slower gets ignored or worked around, which is
worse than not having it.

---

## 12. Fixtures

`tests/fixtures/` — the project's most valuable asset after the code:

| Directory | Contents | 2026-08-27 |
|:---|:---|:---|
| `site/` | the offline fixture web site (`serve.py`, `robots.txt`, `sitemap.xml`, `feed.xml`, `articles/`, `private/`, `trap/`) | ✅ |
| `corpus/` | sample documents for seeding — `documents.jsonl` is **generated** by `scripts/gen_corpus.py` (2 000 in CI), plus `queries.txt` (13) | ✅ |
| `pages/` | real Algerian pages with expectations — one so far (`aps.dz-article.html`) | partial (the spec said `html/`, 200 pages) |
| `serp/` | recorded search-engine result pages for the discovery parsers (`bing-elkhabar.html`, `ddglite-paracetamol.html`) | ✅ |
| `facebook/ instagram/ tiktok/` | recorded API payloads including error envelopes | ❌ |
| `audio/` | 100 recordings + reference transcripts | ❌ |
| `images/` | 200 images including adversarial ones | ❌ |
| `poison/` | payloads that once crashed something — **every DLQ investigation adds one** | ❌ (hostile markup is inline in `adversarial.rs`) |
| `injection/` | prompt-injection passages | ❌ |
| `bidi/` | mixed-direction strings | ❌ (`scripts/lint-bidi.sh` covers the source, not fixtures) |

The eval assets live beside, not under, `tests/`: `eval/golden/v1.jsonl`, `eval/reports/`,
`eval/serp-queries.txt`.

---

## 13. Open Questions

- [ ] Who produces the judged relevance set, and how do we handle judge disagreement on Darija?
- [ ] Can we measure real-world relevance without logging queries? (Proposal: zero-result rate by
      language only, aggregate and k-anonymous — [[Observability]] §8.)
- [ ] Is 95 % summary faithfulness acceptable, or should the bar be higher before beta given that a
      wrong summary is the most visible possible failure?

## Related

[[Performance Budgets]] · [[Ranking and Relevance]] · [[Running Xustive]] ·
[[Security and Privacy]] · [[UI - Accessibility]] · [[Observability]] · [[TODO]] ·
[[Load Generator]] · [[Operating Xustive]]
