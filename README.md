# Xustive

A self-hosted search engine for the Algerian web, indexing public content in **Arabic, Darija,
French, and English**. Searches are not recorded, and nothing leaves the country.

> **Documentation lives in [`docs/`](docs/), an Obsidian vault.** Open it in Obsidian and start at
> [`Home.md`](docs/Home.md) — the graph view shows the system topology, and every component note
> links to its neighbours. Reading the docs in a plain editor works too, but you lose the backlinks.

| Question | Note |
|:---|:---|
| How does it all fit together? | [`docs/Architecture/System Architecture.md`](docs/Architecture/System%20Architecture.md) |
| What are the components? | [`docs/Architecture/Component Map.md`](docs/Architecture/Component%20Map.md) |
| What does the API return? | [`docs/Architecture/API Contract.md`](docs/Architecture/API%20Contract.md) |
| Why was X chosen? | [`docs/Decisions/Decision Log.md`](docs/Decisions/Decision%20Log.md) |
| What are we building next? | [`docs/Planning/TODO.md`](docs/Planning/TODO.md) |
| What does this word mean? | [`docs/Glossary.md`](docs/Glossary.md) |

---

## Status

**Milestone 0 — Foundations.** Search works end to end over a sample corpus. No crawler yet; the
index is populated from fixtures. See [`docs/Planning/TODO.md`](docs/Planning/TODO.md) for what
each milestone delivers.

## Quick start

Needs Rust 1.85+, Docker, and Python 3 (for the corpus generator).

```sh
make dev-up      # meilisearch, qdrant, redis, prometheus, grafana
make corpus      # generate ~10k sample documents
make seed        # create indexes, apply settings, index the corpus
make run-api     # http://localhost:8080
```

Then open <http://localhost:8080> and search for `سونلغاز`, `wach rak`, or `facture`.

| Command | Does |
|:---|:---|
| `make check` | everything CI runs: fmt, clippy, both lints, all tests |
| `make text Q='الجَزَائِر'` | show what the normaliser does to a string |
| `make search Q='وهران'` | search from the command line |
| `./scripts/smoke.sh` | end-to-end checks against a running API |
| `make help` | all targets |

Ports are overridable if they clash with something else you run: `XUSTIVE_REDIS_PORT=6395 make dev-up`.

## Layout

```
crates/
  xustive-text      ★ shared normalisation — called at BOTH query time and index time
  xustive-core        canonical types, error taxonomy, config, SafeUrl, dedup hashing
  xustive-search      Meilisearch client, index settings, filter builder
  xustive-api         HTTP surface, search handler, server-rendered results
  xustive-cli         migrate, seed, stats, text, search
web/public/         hand-authored UI (no build step yet)
deploy/             docker-compose: base is production-shaped, dev override adds host ports
scripts/            corpus generator, lints, smoke suite
docs/               ← the Obsidian vault
```

`xustive-text` is starred because it holds the system together. If query-time and index-time
normalisation ever diverge, Arabic search stops matching with no error anywhere — which is why
its symmetry and idempotency are property-tested rather than assumed.

## Things enforced rather than promised

- **No query logging.** `scripts/lint-telemetry.sh` fails the build if a query or credential field
  appears in a `tracing` call, and the metrics registry only accepts `&'static str` label names, so
  a query string cannot become a metric label. The smoke suite runs a canary query and greps the
  logs for it.
- **No exposed databases.** `scripts/lint-compose.sh` asserts the production compose file publishes
  no ports and keeps its networks `internal: true`.
- **Crawled markup is text, never markup.** Result rendering escapes everything and re-admits only
  the `<em>` the search engine inserts. Tested with live hostile documents.
- **Search works without JavaScript.** `/search` is server-rendered; the client script is an
  enhancement.

## Licence

MIT. All runtime components are MIT/Apache-2.0/BSD — see
[`docs/Engineering/Legal and Compliance.md`](docs/Engineering/Legal%20and%20Compliance.md) §7.
