---
tags:
  - problems
  - privacy
  - audit
date: 2026-08-27
status: open
decision: ADR-0029
---
# Privacy Relaxation Audit — what the old rule held back, ranked

> Governed by [[ADR-0029 - Raw Queries May Leave, Identities Never; First-Party Data Comes Later]]. Sibling of [[Problems]] (capacity) and the bug register. Every entry below is a place
> where [[ADR-0008 - No Query Logging]] and the rules that followed it — no egress from the
> serving plane, no query-keyed stores, k-anonymous counts only, words-never-pictures — limited,
> degraded or disabled something that search or the AI systems need. Each carries the evidence
> (file and line, as of 2026-08-27), what it costs today, what ADR-0029 now permits, and a fix
> ranked by effort. **Nothing here is fixed by this document.** It is the list to fix from.

## What ADR-0029 does not relax — read first

Three rules stand, and several items below lean on them:

- **No identity leaves, ever.** IP, device, session, account, precise location, or any
  combination that adds up to a person. Third-party calls originate from our servers with our
  credentials. The salted-network rate limiter and the signed thumbnail proxy
  ([[ADR-0021 - Proxied Thumbnails with Signed URLs]]) stay.
- **No face recognition** (`scripts/lint-no-face.sh`, `docs/Components/Image Pipeline.md:167`).
  A picture may go to a reverse-image service; a *person* search is still refused.
- **No query text in logs or metrics** (`scripts/lint-telemetry.sh`). A query beside a peer
  address in a log line is exactly the combination rule 2 forbids. What changes is *stores*,
  once the first-party-data ADR says how they are kept.

## Order of work

The register is ranked by what it buys search and the AI, against effort. Two things come
before any feature: **PRIV-001** (the words the product says about itself must be true first)
and **PRIV-002** (one place to send things out, with identity stripped, instead of a gateway
that can hold exactly two clients). Everything marked *needs ADR-0030* waits for the first-party
data decision (what is collected, lawful basis under Law 18-07, consent, retention, deletion).

| # | Problem | Hurts | Unblocked by | Effort |
|---|---|---|---|---|
| PRIV-001 | The privacy policy, README and architecture notes promise what the build no longer enforces | trust, legal | ADR-0029 rule 3 | S |
| PRIV-002 | Only two outbound clients exist, both on the gateway; every new third party is a code change | everything below | rule 1 + 2 | M |
| PRIV-003 | Summaries come from a 3B local model, non-commercial, "meaningfully lower" quality in Arabic | AI answer quality, launch licence | rule 1 | M |
| PRIV-004 | Translation into Arabic is broken by the local model's own measurement | translate tool, Darija | rule 1 | S |
| PRIV-005 | No spelling correction / "did you mean" | search recall | rule 1 (hosted) or rule 4 (own logs) | M |
| PRIV-006 | Autocomplete cannot learn from what people type; 61 hand-written suggestions | search UX | rule 4 (needs ADR-0030) | M |
| PRIV-007 | Synonyms mined from what publishers write, never from what Algerians type; human-gated (B7) | recall, Darija | rule 4 (needs ADR-0030) | M |
| PRIV-008 | Evaluation is synthetic and circular — 201 machine-judged queries, no real traffic | every ranking decision | rule 4 (needs ADR-0030) | M |
| PRIV-009 | Reverse image search shows the web words, not the picture | image search on the web | rule 1 | S–M |
| PRIV-010 | Discovery cannot use real queries; SERP channel yields nothing | corpus coverage | rule 1 (APIs) + rule 4 | M |
| PRIV-011 | Voice: Darija accuracy degraded and unmeasured; model size capped by a shared 4 GB card | voice search | rule 1 | S–M |
| PRIV-012 | Knowledge panels: pre-harvested only; no live authority lookups; ratings link-only | answer layer | rule 1 | M |
| PRIV-013 | Interaction signals: k ≥ 20, off by default, no sessions, no query chains — no personalisation possible | ranking | rule 4 (needs ADR-0030) | L |
| PRIV-014 | Weather guesses a wilaya from a Lite database and is always the proxy's location behind Next | tools | rule 4 with consent (ADR-0030) | S |
| PRIV-015 | Abuse handling is rate-and-shape only; BUG-042 makes one bucket for the whole site | availability | rule 2 keeps this hard; BUG-042 needs `trusted_proxies` | S |
| PRIV-016 | The no-egress enforcement (egress test, compose lint, "gateway and nothing else") will fail or lie once anything goes out | CI honesty | rule 1 | S |
| PRIV-017 | Law 18-07 obligations move from hypothetical to live | legal | rules 3–4 | — |

Effort: S ≤ a day, M ≤ a week, L longer.

---

## PRIV-001 — The product's own words are now false

**Today.** The home page says *Searches are never linked to you* (`web/lib/i18n/messages.ts:571`);
the privacy page says what is kept is "counts, with no identifier attached", what is never kept
is "your IP address, your device, a session or account" (`:574-577`), and that the external
summariser and federation are "off by default" (`:580-583`) — in four locales. The README's
"Guarantees enforced by the build" (`README.md:166-179`) promises no query logging and no
egress; [[Security and Privacy]] §1 states "nothing leaves the country… structurally true";
[[System Architecture]] :50-53 says no component sends a query to a third-party AI service.

**Cost.** The moment any item below ships, every one of these is a lie a reader could be shown.
Rule 3 of ADR-0029 requires the opposite: name that queries, images and recordings may be
processed by third parties on our behalf, without identifying data, and that despite best
efforts some data may leak.

**Fix (S, first).** Rewrite `privacyLead/Stored/NotStored/ModeNote/ExternalNote/FederationNote`
in ar/ary/fr/en; add the leak sentence; add the missing disclosure that the connection address
is read for approximate location (ADR-0018 :130, ADR-0020 :118 already record that gap); keep
"never your IP, device or account — to anyone" as the promise that is still true. Rewrite
README §Guarantees to "what the build enforces" (no query in logs/metrics; no identity out; no
exposed databases) and drop "no egress". Update Security and Privacy §1 and P2/P7, System
Architecture :50-53, [[Federation Gateway]] :84-102, the integrations page copy
(`web/app/(operator)/admin/integrations/page.tsx:151,172`).

## PRIV-002 — Nowhere to put a third party

**Today.** The serving plane has no route out; the one gateway holds exactly two outbound
clients, `SEARXNG_URL` and `EXTERNAL_LLM_URL`, with no runtime allow-list
(`crates/xustive-core/src/config.rs:418-422`, `crates/xustive-federator/src/main.rs:50-113`,
[[Federation Gateway]] :84-102). The knowledge live path had to be moved into the Next tier to
get egress at all (`web/app/api/knowledge-live/route.ts:5-17`, ADR-0019 :114).

**Cost.** Every feature below needs a client to a named host with identity stripped, and today
each one is a gateway code change or a detour through the web tier.

**Fix (M).** An `[egress]` section: a named allow-list of hosts with a credential each
(`EXTERNAL_*_KEY_FILE`), a shared outbound client that strips identity by construction (our
address, our UA, no forwarded headers, no cookies), timeouts and breakers per host, and
`xustive_egress_total{host,outcome}` so the audit surface is one metric. Whether it lives in the
gateway (keeps the serving plane topologically closed) or in the API (simpler) is the design
call; the gateway is the conservative choice and ADR-0029 §Consequences leans that way. Extend
`scripts/test-egress.sh` to assert *only* the allow-listed hosts are reachable (PRIV-016).

## PRIV-003 — The summariser

**Today.** Qwen2.5-3B-Instruct Q4_K_M in-process via llama.cpp; ADR-0005 :52-55 says quality
is "meaningfully lower than a frontier model, particularly for Arabic synthesis", two concurrent
generations, ~4 GB resident on a card shared with STT (1.5 GB) and the API's own models. The
default model is licensed **non-commercial** ([[Legal and Compliance]] Qwen callout). The
external leg exists but is off by default, gets half the deadline and is fallback-only
(`crates/xustive-api/src/summary.rs:159-186`, `config.rs:796-819`), and can only be one provider
held by the gateway (`admin.rs:697-716`).

**Cost.** The AI answer is the product's headline feature and it is the weakest component in
Arabic; the licence blocks a commercial launch on the default model; the GPU budget forces
trade-offs against voice.

**Fix (M).** Make the hosted model the *primary* summariser when configured, the local one the
fallback (invert `summary.rs`), with the full deadline; support the operator's preferred
providers (memory: Chinese open models first — DeepSeek, Qwen hosted); send the query and the
excerpts only; keep the validator (`INSUFFICIENT`, injection rejection) in front of either;
measure faithfulness on the golden set ([[Testing Strategy]] row "Summary faithfulness", not
built). Same for `knowledge_assist` blurbs (`config.rs:798-807`).

## PRIV-004 — Translation into Arabic

**Today.** `crates/xustive-tools/src/translator.rs:16-34`: measured, `en → ar` gives
`أين closest الصيدلية؟`; "Arabic as a target is not [fine]… The fix is a better model for this
task, not more prompt work." Eight languages only (`translate.rs:22-25`); TRANSLATE limited to
10/min by local slot scarcity (`ratelimit.rs:65-68`).

**Fix (S, after PRIV-002).** A hosted translation call for the pairs the local model fails
(target Arabic, any pair not in the eight), local for the rest; widen the language list; raise
the limit to what the provider allows. Darija stays "approximate" honestly until PRIV-007/008
give it data.

## PRIV-005 — Spelling and "did you mean"

**Today.** `corrected` is always `None`; no spell corrector ([[Query Pipeline]] :49, :174,
:240). Meilisearch's typo tolerance is the only repair.

**Cost.** Every misspelt query — common in Arabizi and in French-keyboard Arabic — returns less
or nothing, silently.

**Fix (M).** Two routes, either now legal: a hosted model asked for a correction of the raw
query (cheap, needs PRIV-002), or — better and later — a corrector trained on our own query
logs once PRIV-008/ADR-0030 exist. Offer it as "did you mean", never auto-apply.

## PRIV-006 — Autocomplete

**Today.** Four sources, none from history: curated (61 lines), prefix index built once at
startup, title search, transliteration (`crates/xustive-api/src/suggest.rs:7-12,44-47`,
[[Autocomplete Service]] :53,:71-98). "A popularity counter is a query log with a different
name" (`suggest.rs:20-25`) — deliberately not built; the k-anonymous history store is
firewalled from it.

**Fix (M, needs ADR-0030).** A popularity source from first-party query logs (with the
threshold the ADR sets), trending, and learned ranking of suggestions; rebuild the prefix index
on a cadence rather than at startup regardless.

## PRIV-007 — Synonyms from what people type

**Today.** `crates/xustive-cli/src/mine.rs`: PMI over document titles and a captured SearXNG
reference, cross-script pairs only, capped (200 candidates); output is a review file loaded by
nothing — "mining proposes; it never promotes" — pending native-speaker review (blocker B7).

**Cost.** Darija and Arabizi recall depends on a lexicon that grows at the speed of one
reviewer, from what publishers write rather than what readers search.

**Fix (M, needs ADR-0030).** Mine query reformulations (a query followed by a rephrased one in
the same session — impossible today by ADR-0018 :96-97) and click co-occurrence for candidate
pairs; keep human promotion but feed it real evidence, ranked by frequency, so B7 review is
minutes, not days. A hosted model can pre-screen candidates now (rule 1).

## PRIV-008 — Evaluation on real queries

**Today.** `eval/build_golden.py:4-27`: 200 machine-judged queries synthesised from the index
by transliteration rules; "This detects regressions. It does not measure quality… partly agrees
with the retrieval engine by construction". Testing Strategy :133-153: one of nine quality rows
exists. The eval replay must synthesise a cohort of 20 to clear the k floor
(`crates/xustive-cli/src/eval.rs:78`). ADR-0008 :62-70 named this cost itself: no production
relevance feedback, no live A/B.

**Fix (M, needs ADR-0030).** A real-query sample (with the retention and threshold ADR-0030
sets) as the eval set, click-based relevance labels, A/B on live traffic with ranking profiles
(the `news_heavy`/`social_heavy` profiles that [[Ranking and Relevance]] :170-174 says do not
exist), and the missing rows: WER (PRIV-011), summary faithfulness (PRIV-003).

## PRIV-009 — Reverse image search on the web

**Today.** The web leg takes words only — lowercase ASCII, 1–80 bytes
(`crates/xustive-api/src/image_search.rs:293-338`), pinned by a source-reading test (:572-589);
ADR-0028 :46-48 admits "a picture with no recognisable subject and no text gets a weak web
query". The filter also refuses Arabic and French words, so the description is English-only.

**Fix (S–M, after PRIV-002).** A reverse-image provider called with the picture itself
(identity stripped): SearXNG has none, so a direct API (the operator chooses; several take an
image upload or an image URL). Keep the words leg as the fallback and for providers that only
take text; let the words be in any script. Rewrite the source-reading test to pin "no identity
in the request", not "no body". Face rule unchanged.

## PRIV-010 — Discovery from real queries

**Today.** The SERP channel is discovery-only by ADR-0012/0013, confined to the ingestion plane,
and yields nothing: a datacentre IP is bot-challenged within a few requests
(`crates/xustive-ingest/src/serp/mod.rs:18-24`, `config/dev.toml:70-73`); T16.10/12/14 open.
Weak-coverage demand is k-thresholded and windowed, so long-tail gaps decay before they are
chased (`weak_coverage.rs:1-19,49-58`), and cannot resolve a term to URLs (:17-19).

**Fix (M).** Query-driven discovery through a search API that permits it (rule 1: the raw query
may go; the operator picks a provider with terms that allow indexing the results' URLs), fed by
real queries under ADR-0030 and by weak-coverage terms below any k once they are first-party
data; that closes T16 without residential proxies.

## PRIV-011 — Voice

**Today.** Whisper `small` + `base` on a shared 4 GB card; "Expect degraded accuracy on Darija…
Do not promise verbatim accuracy" ([[Speech to Text]] :124-127); WER unmeasured (:186-192);
CUDA OOM on the final pass falls back to the light model (:105-107); fine-tuning "needs a
licensed, consented speech corpus" (:195-196) — data the old rules forbade collecting.

**Fix (S–M).** A hosted STT for the final pass (the live partials stay local for latency),
chosen by measured Darija WER — which needs the fixture corpus first (100 recordings, targets
`ar ≤ 25 %, fr ≤ 20 %, ary ≤ 45 %`). Later, under ADR-0030 with explicit consent, retained
recordings become the fine-tuning corpus.

## PRIV-012 — Knowledge

**Today.** The serving plane cannot look an entity up (`knowledge.rs:3,227`); the live fallback
lives in the web tier and talks only to Wikidata/Wikipedia/Open Library
(`knowledge-list/route.ts:9-36`); ADR-0019 :33-38,113-115 rejected per-search fan-out to
authorities and any query-keyed cache; misses are recorded k-anonymously and off by default
(`knowledge.rs:193-205`); Wikimedia throttled the live path after a day of testing (M8 T11).
Goodreads ratings stay impossible (terms, not privacy).

**Fix (M, after PRIV-002).** Authority APIs with keys (film/TV metadata and ratings, books,
sports, weather already) called live for cold entities and cached **by entity** (still the
right key); the demand queue on by default; a hosted model for the blurb. Scraping stays out.

## PRIV-013 — Personalisation and sessions

**Today.** The interaction store has no field that could hold an identifier
(`interaction.rs:1-14`); k ≥ 20 refused below outside dev (`config.rs:527-537`); off by default;
no session grouping or fine timestamps "ever" (ADR-0018 :96-97); the only behavioural ranking
term is an anonymous Wilson-bounded CTR at weight 0.07; the reader's language is the closest
thing to a per-reader signal and it is capped by ADR-0026.

**Fix (L, needs ADR-0030 first).** Per-user history (signed-in or consented), session-level
reformulation chains, personal re-ranking, and a k lowered to what the lawful basis supports.
Also the open gap that does not wait: `queue.signals_url` is set only in dev, so outside dev the
k-anonymous counters already land in the persistent, backed-up queue Redis (ADR-0018 :129,
[[TODO]]).

## PRIV-014 — Location

**Today.** DB-IP City Lite, collapsed to one of 58 wilaya seats, request-scoped, never a cache
key (ADR-0020 rules 1–6); `X-Forwarded-For` never read, so behind the Next proxy the guess is
the proxy's own location (ADR-0020 :50-52 calls this "the correct failure"); attribution rule 8
unmet.

**Fix (S).** Under ADR-0029 rule 2 the *address* still never leaves — but the browser's own
geolocation, asked with consent, may (rule 4, ADR-0030); and `api.trusted_proxies` (BUG-042)
would make the wilaya guess real behind Next. Pay the CC BY attribution regardless.

## PRIV-015 — Abuse handling

**Today.** Buckets keyed on `HMAC(daily salt, ip/24)`, memory-only (`ratelimit.rs:3-22`) —
"we give up the ability to build a profile at all"; behind the Next rewrite proxy every reader
shares one bucket (BUG-042, high, open).

**Fix (S).** BUG-042 first (`api.trusted_proxies`, honour `X-Forwarded-For` only from them —
the rate limiter *may* see the address; rule 2 forbids storing or forwarding it). Content-aware
abuse detection (classifying query text) becomes possible under rule 1 but is a separate call.

## PRIV-016 — The enforcement that will lie

**Today.** `scripts/test-egress.sh` passes only if connections *fail* (:34-123);
`scripts/lint-compose.sh` asserts `internal: true`; ADR-0008 :105-107 already lists that CI runs
compose `--no-start` so the log scan is vacuous, and that there is no `api` service in compose.

**Fix (S, with PRIV-002).** Assert the allow-list: the serving plane reaches exactly the named
hosts and nothing else; keep the log scan for `[?&]q=`; keep lint-telemetry; delete the
"gateway and nothing else" wording from four docs (ADR-0008 :103).

## PRIV-017 — Law 18-07

**Today.** [[Legal and Compliance]] §5: "no user data at all" was the compliance strategy
(:162-164); cross-border transfer "restricted — two opt-in features… off by default" (:154);
ANPDP registration "⚖ VERIFY — likely required before beta" (:149).

**Now.** Sending queries abroad (rule 1) is a cross-border transfer of data that may be
personal (a query can name a person); collecting identifiable data (rule 4) makes the operator
a controller. Before ADR-0030 is written: the lawful basis, the ANPDP position, a retention
schedule, deletion on request, and processor terms with every third party in PRIV-002's
allow-list. This is the item a lawyer owns; the register only names it.

---

## What this register does not contain

Things the old rules did *not* cause, listed so nobody fixes them here: Goodreads ratings (terms
of service), the OCR sidecar's temp file (an implementation slip against ADR-0008, tracked in
[[TODO]]), the missing DB-IP attribution (a licence duty), BUG-042 (a proxy topology bug —
though PRIV-015 needs it), and the M3/M4 gates that were never measured.
