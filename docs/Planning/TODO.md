---
tags:
  - planning
  - moc
type: index
status: living
updated: 2026-08-27
---

# TODO — Master Implementation Plan

> The single place to see where the project is. Each milestone has its own note with the actual
> checkboxes; this note holds the shape, the honest state of each milestone, what is confirmed
> open, and what comes next.
> Parent: [[Home]]

---

## 1. How to Use This

- **Milestone notes** hold the checkboxes. Tick them there. Each carries `status`, `updated` and
  (once closed) `closed` in its frontmatter, plus a dated audit block; this note only summarises.
- A milestone is **done** when every task is ticked or deliberately settled (`[~]` with the decision
  recorded inline) **and** its exit gate passes or the shortfall is written down. A gate is a
  measurement, not a feeling.
- Task ids are `M<milestone>-T<task>.<subtask>` → `M8-T11.3`. Commits carry the id, so
  `git log --oneline | grep M8-T11` finds the work.
- Anything blocked lands in §4 with an owner. A blocker without an owner is a wish.
- Every entry below was re-verified against code and git history on 2026-08-27; anything that
  could not be confirmed says **unverified**.

---

## 2. Where Each Milestone Stands

| # | Milestone | Status | Scope, in one line |
|:--|:---|:---|:---|
| M0 | [[Milestone 0 - Foundations]] | **closed** 2026-08-07 | repo, compose, index, `/search`, a search box |
| M1 | [[Milestone 1 - Text Search MVP]] | **closed** 2026-08-16 | the text-search product: language, expansion, ranking, summariser, eval harness |
| M1B | [[Milestone 1B - Frontend and Instant Answers]] | **closed** 2026-08-16 | Next.js UI, Rust renderer deleted, instant-answer tools and `xustive-toold` |
| M2 | [[Milestone 2 - Ingestion at Scale]] | **closed** 2026-08-21 (web track); social track deferred | crawler, frontier, dedup, enrichment, registry, admin console; the 1M gate not met (~85k docs) |
| M3 | [[Milestone 3 - Multimodal Input]] | **open** — building done, gates unmeasured | OCR (tesseract + optional sidecar), image similarity (CLIP + Qdrant, model not provisioned), **voice live** with partials and GPU (ADR-0024); WER/CER/recall corpora missing |
| M4 | [[Milestone 4 - Quality and Operations]] | **open** — tooling built, gate not run | breakers, shutdown, loadgen, backup/restore, reindex drill, alerts + runbooks; no 10M run, no chaos, no pen test, no dashboards in git |
| M5 | [[Milestone 5 - Beta Launch]] | **not started** | legal, accessibility AA, source submission, static pages (`/privacy`, `/bot` exist), takedown, beta programme |
| M6 | [[Milestone 6 - Adaptive Ranking from Interaction Signals]] | **closed** 2026-08-25 | anonymous k-anonymous CTR loop, re-crawl pull (T06), eval replay uplift +0.0048 nDCG (T09); **off by default** |
| M7 | [[Milestone 7 - Federated Retrieval and External Tools]] | **closed** 2026-08-25 | stemming, hybrid dense recall, related searches, SearXNG gateway, crawl-feed convergence, calibration, integrations console, identifier-free history; PROB-001/002/003 solved |
| M8 | [[Milestone 8 - The Answer Layer]] | **closed** 2026-08-26 | knowledge store + resolver (ADR-0022/0023), entity panel, weather, currency (ADR-0025), fend calculator, demand-driven harvest, list answers (T11) with today's tie-break fix and See-also row |
| M9 | [[Milestone 9 - Images and Videos]] | **closed** 2026-08-26 | Images/Videos tabs, signed thumbnail proxy (ADR-0021), metadata-only video, media repass, SearXNG media federation (T06) |

```
M0 ──► M1 ──► M1B ──► M2 ──► M6 ──► M7 ──► M8 ──► M9        (closed, in this order)
                       │
                       ├──► M3 (open: gates)   M4 (open: gates)   M5 (not started)
                       └──► M2 social track (deferred: needs proxies, sessions, fingerprints)
```

The original plan ran M3 → M4 → M5 after M2. What actually happened: the retrieval, answer and
media milestones (M6–M9) were built first because they compound on the corpus, while M3 and M4
were built far enough to work and then left with their *measurement* gates open. Nothing in M3 or
M4 is a redesign away; each is a corpus, a run, or a schedule away.

---

## 3. Confirmed Open Items

Verified open on 2026-08-27. Grouped by what unblocks them.

### Bugs and code gaps
- [ ] **BUG-042** (high) — behind the Next.js rewrite proxy every reader shares **one rate-limit
  bucket**: `ratelimit.rs` keys on the peer address and ignores `X-Forwarded-For` (correctly), but
  the peer is always the web tier. Needs a trusted-proxy hop count or a web-tier limiter
  ([[2026-08-25 - Code Audit Findings]])
- [ ] **`queue.signals_url` is only set in `config/dev.toml`**; outside dev the accessor falls back
  to the queue Redis, so interaction signals (ephemeral by design, ADR-0015) would land in the
  **persistent** queue Redis. Wire it in every non-dev profile before interaction is ever enabled
- [ ] **No DB-IP CC BY attribution** on the weather card or privacy page, though ADR-0020 relies
  on the DB-IP Lite database. A licence term, not a nicety
- [ ] **No geoip staleness gauge** — the location database has no `xustive_data_age_seconds`
  series, so a stale download is invisible (`scripts/fetch-geoip.sh` has no monitor)
- [ ] **OCR sidecar writes a tempfile** (`services/ocr-sidecar/app.py`: `NamedTemporaryFile` +
  `mkdtemp`) — against the zero-disk-write rule of ADR-0008/ADR-0016 that the STT sidecar and
  the in-process OCR path honour. Opt-in, GPU-only, but still wrong
- [ ] **`scripts/test-egress.sh` gaps** — the assertion covers the serving plane and the gateway;
  the web tier's own outbound (wiki-image, thumb proxy, knowledge-live, Wikidata SPARQL from
  M8-T11.3) is allowlisted by host in ADRs but not all of it is asserted by the script. Exact
  coverage **unverified**; treat as owed
- [ ] **No `LICENSE` file at the repo root**
- [ ] M9-T02.5 — the thumbnail proxy's forged-signature / private-host / mid-chain-redirect tests
  do not exist (the guards do)
- [ ] M4-T02.4 — DLQ retention (stats/peek/replay exist)

### Corpus and data (the M3 gates)
- [ ] Audio fixture corpus, 100 recordings × 4 languages, with reference transcripts → runs
  `score-transcripts --metric wer` (M3-T02.10, T08.1) ← **B7**
- [ ] Labelled screenshot set → CER (M3-T04.8, T08.2)
- [ ] CLIP model provisioned into `clip-embed`, then recall/latency and transform robustness
  (M3-T05.6/.9, T08.3)
- [ ] Silence auto-stop and the 25 s announce on voice (M3-T03.5); retry keeps the blob (T03.8)

### Runs and schedules (the M4 gate)
- [ ] The 10M-document scale-up and every load test in M4-T03/T05; the Redis 1 GB `maxmemory`
  finding (M4-T05.6) is the first thing that will bite
- [ ] Chaos exercises (M4-T02.8), restore drill with measured RTO/RPO (M4-T04.6), Grafana
  dashboards in git (M4-T01.5), nightly `scan-logs.sh` (M4-T08.4), security review (M4-T08)
- [ ] Relevance re-run on the real corpus (M4-T05.7) — the eval harness's drift rule means every
  cross-corpus nDCG comparison so far measures the crawl, not the ranker

### Discovery and coverage
- [ ] **M2-T16 discovery channels** — the SERP channel (T16.10/.12/.14) is blocked on the proxy /
  session / fingerprint stack it shares with the social track; query-driven discovery has no
  source until then. Federation (M7-T06) and the Brave connector are the live channels today
- [ ] **Wikimedia throttles the live path** (M8-T11 status): the answer is to harvest — a subject
  in the store never takes the live path. Keep the demand queue draining
- [ ] **Arabic bare surnames miss on the live path** (*أفلام سبيلبرغ*) because Wikidata's search
  is prefix-on-label; store aliases resolve in any script — one more reason to harvest
- [ ] **Goodreads ratings cannot be had legitimately** (no public API since 2020, scraping
  forbidden) — books show cover, year and links, no rating. Settled, not fixable
- [ ] The 1M-document gate of M2 (~85k at the M7 close; growth is bounded by politeness and host
  diversity, PROB-002)

### Licensing and launch
- [ ] **Summariser licence** — the default `qwen-research` (Qwen2.5-3B) is non-commercial; swap to
  an Apache-2.0 size (1.5B / 7B, `config/dev.toml` already documents the 7B download) before any
  commercial launch (M3-T01.3, M5-T01.8)
- [ ] All of M5: legal entity (B8), privacy policy verified line-by-line against ADR-0008 *and*
  its M7-T10.6 amendment, AA pass, `/about` `/terms` `/submit`, `POST /sources`, takedown owner,
  beta feedback loop that works without query logs

### Human review (**B7**, **B5**)
- [ ] Native-speaker review of the mined synonym candidates (M7-T01.2) before they land in
  `data/expansion/*.tsv`
- [ ] Native Darija review of the `ary` locale (M5-T06.2) — the strings exist, nobody who speaks
  Darija has read them
- [ ] Curation owner for lexicons, registry and spam list — still unassigned

---

## 4. Blockers and Decisions Needed

| # | Item | Blocks | State |
|:--|:---|:---|:---|
| B1 | Account acquisition + pool sizing for FB/IG; warm-up is 10+ days wall-clock | M2 social track | open, no owner |
| B2 | Residential/mobile proxy provider — DZ coverage, exit-node consent | M2-T07, M2-T16 SERP channel, social | open, no owner |
| B3 | Monthly bandwidth budget for residential pools | M2-T07 cost gate | open |
| ~~B4~~ | Is a 3B model good enough for Arabic summaries? | — | closed: yes on quality; latency answered by the GPU path |
| B5 | Who owns lexicon and registry curation? | quality of everything | open, no owner |
| ~~B6~~ | Is a GPU in the budget? | — | closed: Quadro T1000 4 GB is the reference, CPU-only must work, device switchable from admin |
| B7 | Native Darija speakers for strings, synonym review and the audio corpus | M3 gates, M5-T06, M7-T01.2 | open, no owner |
| B8 | Legal entity + Law 18-07 position | M5-T01 | open |
| B9 | Owner for the collection-maintenance tail | M2 social track, permanently | open, no owner |

B1, B2, B5, B7 and B9 are people and procurement, not engineering, and they are what slips.

---

## 5. Candidate Next Work

In rough order of value per day, given what §3 says.

1. **Close the code gaps in one pass** — BUG-042, `signals_url` outside dev, DB-IP attribution,
   geoip gauge, OCR tempfile, thumb-proxy tests, `LICENSE`. Half a day; every one is a
   correctness or licence matter, not a feature.
2. **Harvest, not fetch** — drain the M8 demand queue on a schedule so the answer layer stops
   depending on Wikimedia's mood; this also retires the Arabic-surname limitation for anything
   people actually ask for.
3. **M4's measurement gate** — run loadgen at the current corpus, profile the 1 GB Redis, do the
   restore drill for real, provision the dashboards. The numbers will change decisions.
4. **M3's corpora** — the smallest is the screenshot set (CER); the audio corpus needs B7.
5. **Summariser licence swap** — a config change plus a quality check; do it before anyone says
   the word "launch".
6. **M5 groundwork that needs no lawyer** — `/about`, `/terms`, the accessibility pass, the
   "report this result" feedback design (M5-T07.2).
7. **Discovery** — once B2 exists, M2-T16's SERP channel and the social track together; until
   then, widen federation and the Brave connector.

---

## 6. Cross-Cutting Tracks

Continuous work that must not become a "polish" phase.

- **Data curation** (B5): Darija markers, expansion lexicon, sentiment lexicons, per-domain parser
  rules, gazetteer, spam phrases — owner unassigned.
- **Evaluation**: the golden set is at 200 judged queries (regenerated for M6); language-detection,
  sentiment and summary-faithfulness sets are still owed; every quality complaint becomes a
  golden row.
- **Security**: `SafeUrl` on every fetch, telemetry lint, egress test and `cargo deny` are in CI;
  keep them green and widen the egress script (§3).
- **Documentation**: component `status` frontmatter, an ADR per resolved question (0027 today),
  a runbook before an alert goes live (`lint-runbooks.sh` enforces this).

---

## 7. Definition of Done (per task)

- [ ] Code merged with tests at the levels [[Testing Strategy]] specifies
- [ ] The component note's §11 test plan is implemented; its §9 metrics and log events emitted
- [ ] Its [[Performance Budgets]] entry is met, measured
- [ ] The top three failure modes from the note's §7 are handled and tested
- [ ] The note's `status` frontmatter is updated
- [ ] Any decision made along the way is an ADR or a note edit

## Related

[[Home]] · [[Component Map]] · [[Testing Strategy]] · [[Performance Budgets]] · [[Decision Log]] ·
[[Legal and Compliance]] · [[2026-08-25 - Code Audit Findings]]
