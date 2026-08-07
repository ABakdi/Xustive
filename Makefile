.DEFAULT_GOAL := help
# The base file is the production topology; the dev override adds a host-reachable network,
# because in development xustive-api runs on the host rather than inside `core`.
COMPOSE := docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.dev.yml
CONFIG  ?= config/dev.toml
# Port for the offline crawler fixture site (tests/fixtures/site).
FIXTURE_PORT ?= 8099

.PHONY: help
help: ## Show this help
	@grep -hE '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2}'

# --- infrastructure -----------------------------------------------------------------

.PHONY: setup
setup: ## Check prerequisites, install git hooks, create .env
	@./scripts/setup.sh

.PHONY: up
up: dev-up corpus seed ## Everything needed to serve search, then tells you what to run next
	@echo
	@echo "Ready. Start the API with:  make run-api"
	@echo "Then open:                  http://localhost:8080"

.PHONY: dev-up
dev-up: ## Start meilisearch, qdrant, redis, prometheus, grafana
	$(COMPOSE) up -d
	@echo "waiting for meilisearch..."
	@for i in $$(seq 1 60); do \
		curl -fsS http://localhost:7700/health >/dev/null 2>&1 && break; \
		sleep 1; \
	done
	@curl -fsS http://localhost:7700/health >/dev/null 2>&1 \
		&& echo "meilisearch ready" \
		|| { echo "meilisearch did not become ready"; exit 1; }

.PHONY: dev-down
dev-down: ## Stop infrastructure (keeps volumes)
	$(COMPOSE) down

.PHONY: dev-reset
dev-reset: ## Stop infrastructure and DELETE all data volumes
	$(COMPOSE) down -v

.PHONY: dev-logs
dev-logs: ## Tail infrastructure logs
	$(COMPOSE) logs -f

# --- application --------------------------------------------------------------------

.PHONY: migrate
migrate: ## Create indexes and apply settings (idempotent)
	cargo run -q -p xustive-cli -- --config $(CONFIG) migrate

.PHONY: migrate-check
migrate-check: ## Report drift between declared and live index settings
	cargo run -q -p xustive-cli -- --config $(CONFIG) migrate --check

.PHONY: crawl
crawl: ## Crawl real Algerian sites and index them (respects robots.txt)
	cargo run -q -p xustive-cli -- --config $(CONFIG) crawl $(ARGS)

.PHONY: corpus
corpus: ## Generate the sample corpus
	python3 scripts/gen_corpus.py --count 10000 \
		--out tests/fixtures/corpus/documents.jsonl

.PHONY: seed
seed: migrate ## Index the sample corpus
	cargo run -q -p xustive-cli -- --config $(CONFIG) seed

.PHONY: stats
stats: ## Show index document counts
	cargo run -q -p xustive-cli -- --config $(CONFIG) stats

.PHONY: run-api
run-api: ## Run the API server — this also serves the web UI at http://localhost:8080
	@# Build first, and say so. The summariser links llama.cpp, which is compiled from source
	@# and takes several minutes the first time. Without this notice `cargo run` looks like it
	@# has hung, and http://localhost:8080 refuses connections until it finishes — which is
	@# exactly what it looks like when the server is broken.
	@echo "  Building (the first build compiles llama.cpp and can take several minutes)…"
	@cargo build -p xustive-api
	@echo
	cargo run -p xustive-api -- --config $(CONFIG)

.PHONY: run-api-fast
run-api-fast: ## Run without the summariser — builds in seconds, no AI summaries
	cargo run -p xustive-api --no-default-features -- --config $(CONFIG)

.PHONY: scan-logs
scan-logs: ## Scan a log file for leaked query text: make scan-logs LOG=/tmp/api.log
	./scripts/scan-logs.sh $(LOG)

.PHONY: fixture-site
fixture-site: ## Serve the offline crawler fixture site on :8099
	@echo "  Fixture site on http://127.0.0.1:8099 — see tests/fixtures/site/README.md"
	./tests/fixtures/site/serve.py --port $(FIXTURE_PORT)

.PHONY: web
web: ## Open the UI in a browser (the API serves it; there is no separate web server)
	@if curl -fsS --max-time 2 http://localhost:8080/healthz >/dev/null 2>&1; then \
		echo "Opening http://localhost:8080"; \
		(xdg-open http://localhost:8080 >/dev/null 2>&1 \
			|| open http://localhost:8080 >/dev/null 2>&1 \
			|| echo "Could not open a browser. Go to http://localhost:8080"); \
	else \
		echo "The API is not running."; \
		echo; \
		echo "  The UI has no separate server — xustive-api serves it from web/public."; \
		echo "  Start it with:  make run-api"; \
		echo "  Then:           make web   (or just open http://localhost:8080)"; \
		exit 1; \
	fi

# The UI is hand-authored in web/public and served directly — there is no build step yet.
# The Tailwind/esbuild pipeline arrives with the component library in M1-T13.

# --- quality ------------------------------------------------------------------------

.PHONY: test
test: ## Run all tests
	cargo test --workspace

.PHONY: fmt
fmt: ## Format
	cargo fmt --all

.PHONY: lint
lint: ## Format check, clippy, and the privacy/topology/docs lints
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings
	./scripts/lint-telemetry.sh
	./scripts/lint-compose.sh
	./scripts/lint-docs.sh

.PHONY: audit
audit: ## Dependency advisories and licence check
	@command -v cargo-deny >/dev/null || { \
		echo "cargo-deny is not installed. Run: cargo install cargo-deny"; exit 1; }
	cargo deny check advisories licenses bans sources

.PHONY: egress-test
egress-test: ## Prove the serving plane cannot reach the internet
	./scripts/test-egress.sh

.PHONY: smoke
smoke: ## End-to-end checks against a running API
	./scripts/smoke.sh

.PHONY: check
check: lint test ## Everything CI runs

# --- convenience --------------------------------------------------------------------

.PHONY: text
text: ## Explain normalisation: make text Q='الجَزَائِر'
	@cargo run -q -p xustive-cli -- --config $(CONFIG) text "$(Q)"

.PHONY: search
search: ## Search from the CLI: make search Q='سونلغاز'
	@cargo run -q -p xustive-cli -- --config $(CONFIG) search "$(Q)"
