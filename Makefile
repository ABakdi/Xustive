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

.PHONY: dev
dev: ## ONE COMMAND — build and run everything, logs interleaved, Ctrl-C stops it all
	./scripts/dev.sh $(ARGS)

.PHONY: dev-stop
dev-stop: ## Stop everything `make dev` started (from another terminal)
	@pid=$$(cat $${TMPDIR:-/tmp}/xustive-dev.pid 2>/dev/null); \
	if [ -n "$$pid" ] && kill -0 "$$pid" 2>/dev/null; then \
		kill -INT "$$pid"; echo "  stopping (pid $$pid)"; \
	else \
		echo "  nothing running that 'make dev' started"; \
	fi

.PHONY: up
up: dev-up corpus seed ## Everything needed to serve search, then tells you what to run next
	@echo
	@echo "Ready. Xustive runs as TWO processes — start each in its own terminal:"
	@echo
	@echo "  1.  make run-api      the Rust API           http://localhost:8080"
	@echo "  2.  make run-web      the Next.js frontend   http://localhost:3000"
	@echo
	@echo "Then open http://localhost:3000 — not 8080. The API serves JSON, not pages."

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

.PHONY: reset
reset: ## Stop everything and DELETE all crawled data — index, frontier, queue
	@./scripts/reset.sh

.PHONY: dev-reset
dev-reset: reset ## Alias for `reset`

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
run-api: ## Run the Rust API on :8080 (JSON only — the UI is `make run-web`)
	@# Build first, and say so. The summariser links llama.cpp, which is compiled from source
	@# and takes several minutes the first time. Without this notice `cargo run` looks like it
	@# has hung, and http://localhost:8080 refuses connections until it finishes — which is
	@# exactly what it looks like when the server is broken.
	@echo "  Building (the first build compiles llama.cpp and can take several minutes)…"
	@# GPU support is decided here, not in config: the cuda feature needs nvcc at *build* time,
	@# so a config switch alone can never turn it on. When the toolkit is present the binary is
	@# built with it and the admin page's device switch does the rest; when absent, CPU-only,
	@# and the device layer says why.
	@if [ -x /opt/cuda/bin/nvcc ]; then \
		echo "  CUDA toolkit found — building with GPU support"; \
		PATH=/opt/cuda/bin:$$PATH CUDA_PATH=/opt/cuda CUDACXX=/opt/cuda/bin/nvcc \
			cargo build -p xustive-api --features cuda; \
	else \
		cargo build -p xustive-api; \
	fi
	@echo
	@if [ -x /opt/cuda/bin/nvcc ]; then \
		PATH=/opt/cuda/bin:$$PATH CUDA_PATH=/opt/cuda CUDACXX=/opt/cuda/bin/nvcc \
			cargo run -p xustive-api --features cuda -- --config $(CONFIG); \
	else \
		cargo run -p xustive-api -- --config $(CONFIG); \
	fi

.PHONY: run-api-fast
run-api-fast: ## Run without the summariser — builds in seconds, no AI summaries
	cargo run -p xustive-api --no-default-features -- --config $(CONFIG)

.PHONY: toold
toold: ## Fetch weather and other external tool data into the cache
	cargo run --release -q -p xustive-toold -- --once

.PHONY: crawld
crawld: ## Run the crawler continuously — resumes from the shared frontier, Ctrl-C to stop
	cargo run --release -q -p xustive-cli -- --config $(CONFIG) crawld $(ARGS)

.PHONY: worker
worker: ## Drain the index queue into Meilisearch
	cargo run --release -q -p xustive-cli -- --config $(CONFIG) worker

.PHONY: dlq
dlq: ## Dead letters: make dlq A=stats|peek|replay
	cargo run --release -q -p xustive-cli -- --config $(CONFIG) dlq $(or $(A),stats)

.PHONY: backup
backup: ## Snapshot Meili+Qdrant+Redis+registry off-host: make backup [DEST=dir]
	scripts/backup.sh $(or $(DEST),backups)

.PHONY: restore-drill
restore-drill: ## Restore from a backup (STAGING ONLY): make restore-drill SRC=backups/<ts> CONFIRM=yes
	CONFIRM=$(CONFIRM) scripts/restore.sh $(SRC)

.PHONY: load
load: ## Load-test the running API on :8080: make load S=search|suggest|summary|mixed [RPS=n DUR=s]
	cargo run --release -q -p xustive-loadgen -- \
		--scenario $(or $(S),search) $(if $(RPS),--rps $(RPS),) $(if $(DUR),--duration $(DUR),)

.PHONY: eval
eval: ## Score the golden set and write a dated report
	cargo run --release -q -p xustive-cli -- --config $(CONFIG) eval

.PHONY: eval-check
eval-check: ## Score the golden set and fail if nDCG@10 regressed
	cargo run --release -q -p xustive-cli -- --config $(CONFIG) eval \
		--baseline eval/reports/baseline.json --dry-run

.PHONY: golden
golden: ## Regenerate the machine-judged golden set from the live index
	./eval/build_golden.py --out eval/golden/v1.jsonl

.PHONY: ui-gates
ui-gates: ## Client asset budgets and the no-JavaScript path (needs web running on :3000)
	./scripts/bundle-budget.sh
	./scripts/no-js-check.sh
	./scripts/rtl-icons.sh
	node scripts/contrast-audit.mjs

.PHONY: scan-logs
scan-logs: ## Scan a log file for leaked query text: make scan-logs LOG=/tmp/api.log
	./scripts/scan-logs.sh $(LOG)

.PHONY: fixture-site
fixture-site: ## Serve the offline crawler fixture site on :8099
	@echo "  Fixture site on http://127.0.0.1:8099 — see tests/fixtures/site/README.md"
	./tests/fixtures/site/serve.py --port $(FIXTURE_PORT)

.PHONY: web
web: ## Open the UI in a browser
	@if curl -fsS --max-time 2 http://localhost:3000 >/dev/null 2>&1; then \
		echo "Opening http://localhost:3000"; \
		(xdg-open http://localhost:3000 >/dev/null 2>&1 \
			|| open http://localhost:3000 >/dev/null 2>&1 \
			|| echo "Could not open a browser. Go to http://localhost:3000"); \
	else \
		echo "The frontend is not running."; \
		echo; \
		echo "  Xustive is two processes. In separate terminals:"; \
		echo "    make run-api      the Rust API,       :8080"; \
		echo "    make run-web      the Next.js UI,     :3000"; \
		echo; \
		echo "  Open :3000. Port 8080 answers JSON, not pages — /search there is a 404."; \
		exit 1; \
	fi

.PHONY: run-web
run-web: ## Run the Next.js frontend on :3000 (needs the API on :8080)
	@command -v node >/dev/null || { echo "node is not installed (need 20+)"; exit 1; }
	@[ -d web/node_modules ] || { echo "  Installing frontend dependencies (first run only)…"; cd web && npm install; }
	@if ! curl -fsS --max-time 2 http://localhost:8080/healthz >/dev/null 2>&1; then \
		echo "  Note: the API is not up on :8080. The UI will start, but every search will"; \
		echo "        error until you run 'make run-api' in another terminal."; \
		echo; \
	fi
	cd web && npm run dev

.PHONY: web-build
web-build: ## Production build of the frontend, then serve it on :3000
	cd web && npm run build && npm start

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
	./scripts/check-alerts.sh
	./scripts/lint-runbooks.sh
	./scripts/lint-bidi.sh

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
