#!/usr/bin/env bash
#
# One-time developer setup.
#
# Checks prerequisites, installs the git hooks, and creates .env. It deliberately does NOT fetch
# models: xustive-ml does not exist yet, so there is nothing to fetch. When it does, model
# download belongs here.
#
# Safe to re-run.

set -uo pipefail
cd "$(dirname "$0")/.."

ok()   { printf '  \033[32m✓\033[0m %s\n' "$1"; }
warn() { printf '  \033[33m~\033[0m %s\n' "$1"; }
bad()  { printf '  \033[31m✗\033[0m %s\n' "$1"; }

missing=0

printf '\n\033[1mPrerequisites\033[0m\n'

if command -v cargo >/dev/null 2>&1; then
  ok "rust $(rustc --version | cut -d' ' -f2)"
else
  bad "rust not found — install from https://rustup.rs"
  missing=1
fi

if command -v docker >/dev/null 2>&1; then
  if docker compose version >/dev/null 2>&1; then
    ok "docker $(docker --version | cut -d' ' -f3 | tr -d ,) with compose v2"
  else
    bad "docker found but 'docker compose' is not available (compose v2 required)"
    missing=1
  fi
  docker info >/dev/null 2>&1 || warn "docker daemon does not appear to be running"
else
  bad "docker not found"
  missing=1
fi

if command -v python3 >/dev/null 2>&1; then
  ok "python $(python3 --version | cut -d' ' -f2) (corpus generator)"
else
  bad "python3 not found — needed to generate the sample corpus"
  missing=1
fi

# Optional: only needed for `make audit`.
if command -v cargo-deny >/dev/null 2>&1; then
  ok "cargo-deny (dependency audit)"
else
  warn "cargo-deny not installed — 'make audit' will not run. cargo install cargo-deny"
fi

printf '\n\033[1mConfiguration\033[0m\n'

if [ -f .env ]; then
  ok ".env already exists, left untouched"
else
  cp .env.example .env && ok "created .env from .env.example"
fi

# Point git at the tracked hooks directory. Idempotent.
if git rev-parse --git-dir >/dev/null 2>&1; then
  git config core.hooksPath .githooks
  ok "git hooks enabled (.githooks/pre-commit)"
else
  warn "not a git repository, skipped hooks"
fi

printf '\n\033[1mNot needed yet\033[0m\n'
printf '  Model files, Tesseract and libtorch become prerequisites when xustive-ml arrives\n'
printf '  in Milestone 2. There is nothing to download today.\n'

if [ "$missing" -ne 0 ]; then
  printf '\n\033[31mSome prerequisites are missing.\033[0m Install them, then re-run: make setup\n\n'
  exit 1
fi

printf '\n\033[1mReady.\033[0m Next:\n\n'
printf '  make up          # infrastructure, corpus, and a seeded index (~13s)\n'
printf '  make run-api     # then open http://localhost:8080\n\n'
printf 'Full runbook: docs/Engineering/Running the System.md\n\n'
