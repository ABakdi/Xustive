---
tags:
  - planning
  - milestone
milestone: 0
status: complete
updated: 2026-08-06
---

# Milestone 0 - Foundations

> **Goal:** a developer can clone the repo, run one command, and search 10 000 documents in a
> browser. Nothing clever — just the skeleton every later milestone hangs off.
> **Exit gate:** 10k documents indexed and searchable end-to-end; CI green; `make check` passes on a
> clean machine.
> Parent: [[TODO]] · Next: [[Milestone 1 - Text Search MVP]]

---

## Why This Milestone Exists

Two things get locked in here that are expensive to change later: the **shared text normalisation**
([[Content Parser]] §4.4) and the **security primitives** (`SafeUrl`, telemetry lint). Retrofitting
either is painful, and getting normalisation wrong means Arabic search silently fails for months.

---

## M0-T01 — Repository and workspace skeleton

- [x] M0-T01.1 Cargo workspace with the crate layout from [[Local Development]] §1
- [x] M0-T01.2 `rustfmt.toml`, `clippy.toml` with `unwrap_used`/`expect_used` denied
- [x] M0-T01.3 `Makefile` with the targets in [[Local Development]] §3
- [x] M0-T01.4 Layered config loader (defaults → toml → env), typed structs, no direct `env::var`
- [x] M0-T01.5 `config/dev.toml`, `staging.toml`, `prod.toml`
- [x] M0-T01.6 `.env.example`, `.gitignore`, pre-commit hook running `fmt` + `clippy`
- [x] M0-T01.7 `README` pointing at this vault

## M0-T02 — Docker Compose infrastructure

- [x] M0-T02.1 `deploy/docker-compose.yml`: meilisearch, qdrant, redis, prometheus, grafana
- [~] M0-T02.2 Networks `core` / `obs` with `internal: true`; `edge`/`ingest` land with the
      services that need them (Caddy in M4, crawler in M3)
      ([[Deployment Topology]] §3)
- [x] M0-T02.3 Named volumes and healthchecks for every service
- [x] M0-T02.4 Resource limits matching [[Performance Budgets]] §7, scaled to a development
      machine, with a lint that fails on any uncapped service
- [x] M0-T02.5 CI check: **no `ports:` mapping on meilisearch, qdrant, or redis**
      ([[Security and Privacy]] T5)
- [x] M0-T02.6 `make dev-up` / `dev-down` verified on a clean machine

## M0-T03 — `xustive-core` types

- [x] M0-T03.1 `Document`, `Comment`, `Media`, `Source` structs per [[Data Model]]
- [x] M0-T03.2 `serde` round-trip tests for every type
- [x] M0-T03.3 `schema_version` handling and forward-compat rules
- [x] M0-T03.4 `ErrorClass` enum and per-crate `thiserror` scaffolding
      ([[Error Handling and Resilience]] §1)
- [x] M0-T03.5 `SafeUrl` newtype with scheme, IP-range, port, and redirect validation
      ([[Security and Privacy]] §4)
- [x] M0-T03.6 SSRF test suite against `SafeUrl` — private IPs, IPv6, decimal literals, rebinding

> `SafeUrl` lands in M0 even though nothing fetches anything yet. It exists so that when
> [[Web Fetcher]] is written in M3, there is no plausible path that bypasses it.

## M0-T04 — `xustive-text` normalisation ★

- [x] M0-T04.1 NFKC, tatweel strip, harakat strip, digit folding
- [x] M0-T04.2 Alef/ya/ta-marbuta secondary folding
- [x] M0-T04.3 Whitespace collapse, length caps
- [~] M0-T04.4 Golden table — ~40 cases plus property tests over arbitrary Unicode. The 200-case
      table grows with M1's lexicon work, when there is real Darija material to draw on.
- [x] M0-T04.5 Property test: `normalize(normalize(x)) == normalize(x)`
- [x] M0-T04.6 **Symmetry test**: index-time and query-time paths produce byte-identical output
- [x] M0-T04.7 `xustive-cli text normalize` for debugging

> This is the highest-leverage crate in the repo. [[Content Parser]] §4.4 and
> [[Query Pipeline]] §4.1 both call it, and divergence between them is invisible until users report
> that Arabic search "doesn't work".

## M0-T05 — Meilisearch settings and migration

- [x] M0-T05.1 Index settings JSON in git per [[Search Index]] §4.2
- [x] M0-T05.2 Idempotent migration job (`make migrate`) applying settings by alias
- [x] M0-T05.3 Alias scheme `documents` → `documents_v1` ([[Data Model]] §7). Meilisearch has no
      alias primitive, so it is a naming convention resolved at startup: highest `_vN` wins,
      except that a pre-alias index named exactly the alias always wins over any versioned one —
      without that ordering, deploying this change points a live system at an empty index
- [x] M0-T05.4 Scoped API keys: search-only and index-only ([[Security and Privacy]] §7),
      provisioned idempotently by `xustive-cli keys`. Needs `MEILI_MASTER_KEY` set; development
      runs without one, and the command says so rather than reporting a bare 401
- [x] M0-T05.5 Test: live settings match git settings

## M0-T06 — Minimal `xustive-api`

- [x] M0-T06.1 Axum server, `/healthz`, `/readyz`, `/metrics`
- [x] M0-T06.2 `GET /search` — query in, Meilisearch out, no expansion, no ranking, no summary
- [x] M0-T06.3 Response shape matching [[API Contract]] §2 (fields may be empty, shape must be right)
- [x] M0-T06.4 Error object and code mapping ([[API Contract]] §8)
- [x] M0-T06.5 Graceful shutdown on SIGTERM

## M0-T07 — Sample corpus and seeding

- [x] M0-T07.1 Collect 10k Algerian documents (news, gov, blogs) as fixtures
- [x] M0-T07.2 Normalise into `Document` JSON with real dates and languages
- [x] M0-T07.3 `make seed` indexes them in under 60 s
- [x] M0-T07.4 Corpus covers all four languages and both scripts, including Arabizi content

## M0-T08 — Minimal UI

- [~] M0-T08.1 Static HTML shell. CSS is hand-authored against the design-system tokens; the
      Tailwind/esbuild pipeline arrives with the component library in M1-T13, where it earns its
      keep. A build step for 380 lines of CSS would not.
- [x] M0-T08.2 Search box → `/search` → render a plain result list
- [x] M0-T08.3 `dir="auto"` on the input and on every content slot
- [x] M0-T08.4 CSP and security headers ([[Security and Privacy]] §3)
- [x] M0-T08.5 Works with JavaScript disabled ([[UI Specification]] §8)

## M0-T09 — CI pipeline

- [x] M0-T09.1 fmt → clippy → unit → build
- [~] M0-T09.2 Integration coverage via CI service containers + `scripts/smoke.sh` against a real
      Meilisearch. `testcontainers` in-process is an M1 convenience, not a gap in coverage.
- [x] M0-T09.3 **Telemetry lint** — fails on query-shaped identifiers in `tracing::` calls
- [x] M0-T09.4 **Egress test** — `xustive-api` cannot reach the public internet
- [x] M0-T09.5 `cargo-deny` (licences) and `cargo-audit` (advisories)
- [ ] M0-T09.6 Total PR feedback under 10 minutes — unverified until CI runs on a real PR

## M0-T10 — Offline fixture site

- [x] M0-T10.1 Static site at `tests/fixtures/site/` served by `make fixture-site`
- [x] M0-T10.2 Includes: sitemap, RSS, an SPA page, redirect chain **and a cycle**, 429 endpoint,
      a slow endpoint, `robots.txt` with `Crawl-delay` and `Disallow`, malformed HTML,
      `windows-1256` page, Maghrebi dates, a prompt-injection page, and a crawler trap.
      Exercised by 11 tests running the real `Fetcher` against it
- [x] M0-T10.3 Documented in [[Local Development]] §5

---

## Exit Gate

| Check | Threshold |
|:---|:---|
| Clean-machine setup | `make setup && make dev-up && make seed && make run-api` works, documented time ≤ 30 min |
| Search works | a known Arabic query returns a known document in the browser |
| Normalisation symmetry test | passing |
| SSRF suite | passing |
| Telemetry lint + egress test | passing in CI |
| CI duration | ≤ 10 min |

## Risks

| Risk | Mitigation |
|:---|:---|
| Normalisation decisions made casually now, painful later | the symmetry test and golden table are M0 deliverables, not M1 |
| "We'll add security tests later" | telemetry lint, egress test, and SSRF suite are M0 gate items |
| Corpus is too clean to be representative | deliberately include malformed, mixed-script, and Arabizi content |

## Related

[[TODO]] · [[Local Development]] · [[Data Model]] · [[Search Index]] · [[Security and Privacy]] ·
[[Milestone 1 - Text Search MVP]]
