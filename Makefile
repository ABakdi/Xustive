.DEFAULT_GOAL := help
# The base file is the production topology; the dev override adds a host-reachable network,
# because in development xustive-api runs on the host rather than inside `core`.
COMPOSE := docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.dev.yml
CONFIG  ?= config/dev.toml

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
run-api: ## Run the API server
	cargo run -p xustive-api -- --config $(CONFIG)

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
