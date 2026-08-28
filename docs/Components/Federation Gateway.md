---
tags:
  - component
  - ingestion
  - serving
component-id: C31
binary: xustive-federator
status: built
updated: 2026-08-27
---

# Federation Gateway

> **ID** C31 · **Binary** `xustive-federator` · **Upstream** [[Query Pipeline]] (serving side), self-hosted SearXNG + allowlisted external tools (egress side) · **Downstream** [[Query Pipeline]] (blended results), [[Crawler Orchestrator]] (crawl hints) · **Governed by** [[ADR-0017 - Query-Time Federation with External Metasearch]]

## 0. Where it lives today

Built in M7 (T04–T09), extended for Images/Videos in M9-T06. Audited against code 2026-08-27.

| Piece | Where |
|:---|:---|
| SearXNG client + pure response parser (`FederatedHit`, `FederatedMedia`, `Category`) | `crates/xustive-federation/src/lib.rs` — re-exported as `xustive_ingest::federation` |
| External summariser client (OpenAI-compatible `/chat/completions`) | `crates/xustive-federation/src/llm.rs` |
| The gateway binary: `GET /healthz`, `POST /federate`, `POST /summarise` | `crates/xustive-federator/src/{lib,main}.rs`, image `deploy/Dockerfile.federator` |
| Serving-side client with one breaker per route | `crates/xustive-api/src/federate.rs` (`FederatorClient`) |
| Detached fetch, strip wait, blend, eager index, crawl-feed | `crates/xustive-api/src/search.rs` (`ingest_federated`, `merge_federated`) |
| External summary attempt before the local model | `crates/xustive-api/src/summary.rs` |
| Runtime switches + stats: `GET/POST /api/v1/admin/integrations` | `crates/xustive-api/src/admin.rs`, page `web/app/(operator)/admin/integrations/page.tsx` |
| SearXNG override (JSON format on, limiter off, image proxy off) | `services/searxng/settings.yml` |
| Compose: `searxng` + `federator` under the `federation` profile, off by default | `deploy/docker-compose.yml`; dev publishes the gateway on `127.0.0.1:8095` |
| Config `[federation]` | `crates/xustive-core/src/config.rs` (`FederationConfig`), `config/*.toml` |

## 1. Why this exists as its own process

The serving plane **has no route to the open internet** — enforced, not aspirational ([[ADR-0001 - Two-Plane Architecture]], `scripts/test-egress.sh`, `core` network `internal: true`). But [[ADR-0017 - Query-Time Federation with External Metasearch]] wants a live user query to borrow recall from a metasearch aggregator and blend it with our own results.

The only way to have both is a **single, narrow, allowlisted hop**. `xustive-federator` is that hop: a stateless sidecar on a bridged tier — one interface on `core` facing [[Query Pipeline]], one on an egress network facing a **self-hosted SearXNG** and a fixed endpoint allowlist. The API gains exactly one new outbound target (this gateway) and still cannot reach anything else. Egress lives here, behind an allowlist we own, so the serving plane's no-egress property survives as *"one allowlisted internal hop"* — provable in CI.

It is its own binary for the same reason [[Tool Data Plane]] is: **so it cannot acquire capabilities the plane it serves must not have.** The API links no HTTP-egress client; the federator holds the only one.

## 2. Responsibilities

**In scope**
- Accept a sanitised query from [[Query Pipeline]] over `core`; fan it out to enabled tools (SearXNG first) within a hard latency budget.
- Normalise each tool's response into a common `FederatedHit { url, title, snippet, engine, rank }`.
- Hand each hit back to the serving API, which (not the gateway) feeds the URL to the shared
  frontier as a `Federation`-channel discovery and, optionally, eager-indexes it (§3). *Superseded
  2026-08-27: the design said the gateway would write `federation:hint:<url>` Redis keys; in code
  the gateway is stateless and the API's `ingest_federated` does the feeding, through the ordinary
  bounded frontier ([[PROB-001 - Bounded Frontier and Queue]]) — no bypass around its ceilings.*
- Enforce budgets and hold the only two outbound clients (SearXNG, external LLM). The endpoint
  **allowlist** in the original design was never enforced by code and the config key was removed
  (BUG-004): the gateway's reach is bounded *topologically* — it can only dial the two endpoints its
  own environment names.
- Expose `/healthz`; the admin **Integrations** page reads liveness, breaker state and the blend
  counters from the API side.

**Out of scope**
- Ranking. The gateway returns tagged candidates; [[Ranking and Relevance]] blends and caps them. **No.** — the federator does not decide final order.
- Storing results. It holds no index and no cache at all — not even the short in-process TTL cache
  the design allowed for; there was no need.
- Fetching page content. It receives URL lists (and snippets), never bodies — content acquisition is the crawler's job, so provenance and politeness stay in one place.
- Being on the answer's critical path. Federation is additive and fail-open (§7); a missing gateway degrades to index-only.

## 3. Interface

- **In (from serving):** `POST /federate {query, budget_ms?, category?}` on `core` only, where
  `category` is `web` (default), `images` or `videos` (M9-T06) and maps to SearXNG's `general`,
  `images`, `videos` categories. Returns `{hits: [FederatedHit], partial: bool}`;
  `FederatedHit {url, title, snippet, engine, rank, media?}` and an image or video hit carries
  `media {kind, src, thumb?, detail?}` — for video, `src` is the watch page and never an embed
  ([[ADR-0021 - Proxied Thumbnails with Signed URLs]]). `partial=true` when the budget cut SearXNG
  short or it errored; an unconfigured gateway answers empty and *not* partial. No `lang` field —
  SearXNG is called without `language`/`country` so mixed-script Darija queries are not
  over-constrained. Bodies are capped at 64 KB (BUG-008): the gateway is unauthenticated on `core`
  and a compromised container must not be able to pump megabyte prompts through a paid key.
- **`POST /summarise {prompt, budget_ms?}` → `{text: string|null}`** (M7-T08): the API builds the
  grounded prompt; the gateway only carries it to an OpenAI-compatible `/chat/completions` endpoint
  (DeepSeek, Qwen/DashScope, OpenRouter… — a config choice), `max_tokens` 512, temperature 0.3, no
  redirects followed (BUG-026). `null` is the fail-open answer and the API falls back to the local
  [[Summarizer]]. *Superseded 2026-08-27: the design named a Parallel-AI MCP; what shipped is the
  generic chat-completions shape.*
- **`GET /healthz`** → `ok`.
- **Out (egress):** exactly two clients — SearXNG at `SEARXNG_URL`, the LLM at `EXTERNAL_LLM_URL`.
- **To crawler:** none directly. The API's `ingest_federated` canonicalises each URL, optionally
  eager-indexes a thin document (`source_id = "federation"`, `DiscoveryChannel::Federation`,
  `quality_score 0.1`, `EnrichmentLevel::Partial`, the media block kept for the Images/Videos
  tabs), then adds it to the frontier at `trust 40`, `depth 0` and front-promotes it. The full
  crawl shares the URL-derived id, so it overwrites the thin document.
- **To admin:** `GET /api/v1/admin/integrations` returns `federation {enabled, configured,
  gateway_reachable, breaker, …}`, `external_summariser {…}` and the convergence counters;
  `POST /api/v1/admin/integrations {integration: "federation"|"external_summariser", enabled}`
  flips the runtime `AtomicBool`s on `AppState` and refuses to arm anything with no
  `federator_url` configured. The client is built whenever a URL is set — even with federation
  off — so the toggle works without a restart.

**Gateway environment** (`xustive-federator`): `FEDERATOR_BIND` (`0.0.0.0:8095`), `SEARXNG_URL`
(empty = inert), `FEDERATION_MAX_HITS` (10, clamped 1–50), `FEDERATION_TIMEOUT_MS` (15000 — the
transport backstop; it sat at 2000 once and cut every image search short, M9-T06),
`FEDERATION_BUDGET_MS` (250, used when a request carries none), `EXTERNAL_LLM_URL`,
`EXTERNAL_LLM_MODEL`, `EXTERNAL_LLM_TIMEOUT_MS` (30000), `EXTERNAL_LLM_KEY` or — preferred, wins
when set — `EXTERNAL_LLM_KEY_FILE` (BUG-040: a plain env var shows in `docker inspect`).

**API config `[federation]`** (`FederationConfig`): `enabled` (false), `searxng_url` (shown on the
console; the API never calls it), `federator_url` (dev `http://127.0.0.1:8095`, containers
`http://xustive-federator:8095`), `budget_ms` (900 in dev — the strip wait, validated to sit inside
`api.timeout_search_ms`), `fetch_budget_ms` (6000 — the detached fetch), `max_hits` (10),
`eager_index` (true in dev, false by default). *Superseded 2026-08-27: no per-tool tables, blend
`cap`, or `allowlist` exist.*

## 4. Datasets

None owned. Transient inputs (query, tool responses) and transient outputs (crawl hints, which the crawler drains). SearXNG's own engine set and settings are its configuration, not ours to model.

## 5. Validation

- There is no runtime allowlist (see §2). What bounds egress is that the binary holds two clients
  and nothing reads any other destination; `scripts/test-egress.sh` check 5 proves that from `core`
  SearXNG itself is unreachable, so the API cannot bypass the gateway.
- SearXNG's JSON is parsed defensively and **one result at a time**: a `null` thumbnail or a
  malformed `url` type drops that result, not the response (M9-T06 — 118 video hits once became
  zero because one strict field failed the whole body). Blank URLs are dropped; `engine` falls back
  to the first of `engines[]`; an `images.html` result without `img_src` is a web hit, not an image.
- Before a hit becomes a frontier entry it goes through `SafeUrl::parse` and the frontier's
  canonicaliser; the frontier's own limits (`FrontierLimits::from_config`) apply.

## 6. Provenance

Each hit carries `engine` (the credited one), `engines` (every engine that returned the URL),
`rank` and SearXNG's merged `score` (M13-T01.1); a blended card is flagged `from_web` in the
search response, and the eager document is `source_id = "federation"` / `discovery = Federation`,
so a federated result is distinguishable in ranking explanations and on the console. Since
[[ADR-0031 - The Web's Verdict Is a Signal on Our Own Documents]] every hit is also
**distilled**: the API's endorse sink writes the sighting onto the document's `web` record and
`endorsement` signal — for a page we already held as much as for a new one — and the ranking
reads it ([[Ranking and Relevance]] §3.2). A crawl-feed entry records the channel,
not the query — consistent with the `weak_coverage` discovery funnel ([[Crawler Orchestrator]]).

**Blend** (`merge_federated`): page 1 only; each hit not already present becomes a `from_web`
card — *leading* the page in the engine's order when `federated_first` is on (the default),
appended otherwise — deduplicated by the URL-derived id **and** by canonical URL (BUG-007 — a page the
crawler found on its own carries a ULID id, so id-only dedup showed the same URL twice). The strip
wait is `budget_ms` minus a 150 ms shaping margin; the detached fetch keeps running and indexes
regardless, so a slow SearXNG costs the *next* search nothing.

## 7. Failure

**Fail-open, always.** The gateway runs concurrently with local retrieval; on timeout, error, rate-limit, or disabled tool the pipeline ships index-only results — today's behaviour. On the API side a `xustive_core::circuit::SharedBreaker` (3 failures, cooldown 5 s doubling to 60 s) wraps each **route separately** (BUG-006): a dead LLM provider must not shed federation, nor a dead SearXNG suppress external summaries. The budget bounds the whole exchange including body decode (BUG-015). The gateway being **down** is indistinguishable to the user from federation being **off**.

## 8. Security

- One interface on `core`, one on egress; the API can reach the gateway and nothing beyond it (`test-egress.sh` asserts *only* the gateway is reachable from `xustive-api`).
- No query text logged; `query`/`token` stay forbidden telemetry field names. `FederationError`
  strips the URL off every wrapped `reqwest` error (BUG-033) because the SearXNG request carries
  the query as `?q=` and reqwest's `Display` embeds it. SearXNG's *own* loggers print that URL on
  engine failures and are third-party code, so the container runs with `logging: driver: "none"`
  (BUG-038): its stdout is never kept anywhere. Query terms transit **through our SearXNG** to
  engines with no client IP, cookie, session, or identifier — the exposure
  [[ADR-0013 - Direct SERP Collection for Discovery]] accepted, now at request time and reconciled in
  [[ADR-0008 - No Query Logging]].
- The external summariser is opt-in, off by default, and switched separately because it is genuine
  third-party SaaS; its key lives only in the gateway's environment or a mounted secret file.

## 9. Observability

- **Metrics** (API side, `crates/xustive-api/src/metrics.rs`): `xustive_federation_duration_seconds`
  (detached fetch, spawn to hits), `xustive_federation_searches_total{outcome=hits|empty}`,
  `xustive_federation_urls_fed_total`, `xustive_federation_blend_cards_total{source=web|local}` — the
  convergence measure, whose `web` share is expected to fall as the crawl-feed fills the index. The
  Integrations page shows these without a Grafana round trip. *Superseded 2026-08-27: the
  `federation_requests_total{tool}` family in the design was never emitted; the gateway itself
  exposes no metrics.* Bounded-cardinality labels only; **no query label**.
- **Log events:** toggles via admin (with peer), breaker trips, budget overruns — never the query.

## 10. Open questions

- Blend order — *settled 2026-08-27 in code*: federated cards are appended after local results on
  page 1, flagged `from_web`; they are not re-ranked into the local list. Whether interleaving
  would beat the labelled tail for trust is unmeasured.
- Live external summarisation shipped (M7-T08) as an opt-in runtime switch tried *before* the local
  model, within one shared budget (BUG-005). Whether to keep it on is a cost question.
- Whether the crawl-hint feed should prioritise by federation frequency the way [[Interaction Signals]] prioritises re-crawl.

## 11. Test plan

- Egress test: `scripts/test-egress.sh` — `core` is internal, no outbound HTTP or DNS, and (check 5,
  when the profile is up) SearXNG is unreachable from `core`, so only the gateway can talk to it.
- Fail-open: unit tests in `xustive-federator` cover empty query, unconfigured gateway (empty, not
  partial) and serialisation; `FederatorClient` returns `Vec::new()`/`None` on every error path.
- Parser fixtures (`xustive-federation` tests): live SearXNG shapes for web, images and videos;
  `null` fields and one malformed result do not drop the rest; a transport error never renders the
  query (BUG-033 regression, against port 9).
- Convergence: a federated URL, once crawled+indexed, is answered locally on the next identical query and its federation tag disappears.
- Privacy: no code path attaches an identifier to a federate request or a crawl hint; no query text reaches a log/metric/span.

## 12. Decisions

- [[ADR-0017 - Query-Time Federation with External Metasearch]] — why this exists, and the invariants it must keep (serving-plane no-egress preserved as one allowlisted hop; fail-open; self-hosted SearXNG; default off; converge to standalone).

## Related

[[ADR-0017 - Query-Time Federation with External Metasearch]] · [[Query Pipeline]] · [[Crawler Orchestrator]] · [[Ranking and Relevance]] · [[Tool Data Plane]] · the `weak_coverage` discovery funnel ([[Crawler Orchestrator]]) · [[Security and Privacy]] · [[Milestone 7 - Federated Retrieval and External Tools]] · [[Milestone 9 - Images and Videos]] · [[Summarizer]]
