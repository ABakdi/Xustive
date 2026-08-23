#!/usr/bin/env bash
#
# Everything, in one terminal.
#
# Infrastructure, the API, the frontend, the crawler, the index worker and the tool-data fetcher —
# started in dependency order, logs interleaved with a prefix per service, and all of it stopped
# together on Ctrl-C.
#
# The alternative is six terminals and remembering the order, which is how you end up with an
# indexer running against a Meilisearch that has not finished starting and a crawler filling a
# queue nobody is draining.
#
#   ./scripts/dev.sh                 # everything, on the GPU if a CUDA toolkit is present
#   ./scripts/dev.sh --cpu           # force CPU, even where a GPU is available
#   ./scripts/dev.sh --no-crawler    # without the crawler, for UI work
#   ./scripts/dev.sh --fast          # skip the summariser, so the build is seconds not minutes
set -uo pipefail

cd "$(dirname "$0")/.."
CONFIG="${CONFIG:-config/dev.toml}"

WITH_CRAWLER=1
API_FEATURES=""
FORCE_CPU=0
for arg in "$@"; do
  case "$arg" in
  --no-crawler) WITH_CRAWLER=0 ;;
  --fast) API_FEATURES="--no-default-features" ;;
  --cpu) FORCE_CPU=1 ;;
  *)
    echo "unknown option: $arg" >&2
    echo "usage: $0 [--cpu] [--no-crawler] [--fast]" >&2
    exit 2
    ;;
  esac
done

# Colours, one per service, so interleaved output is readable at a glance. Disabled when stdout is
# not a terminal, because escape codes in a piped log file help nobody.
if [ -t 1 ]; then
  C_API=$'\033[36m' C_WEB=$'\033[35m' C_CRAWL=$'\033[33m' C_WORK=$'\033[32m'
  C_TOOL=$'\033[34m' C_SYS=$'\033[1m' C_OFF=$'\033[0m'
else
  C_API="" C_WEB="" C_CRAWL="" C_WORK="" C_TOOL="" C_SYS="" C_OFF=""
fi

say() { printf '%s▸ %s%s\n' "$C_SYS" "$1" "$C_OFF"; }

PIDS=()

# Our own pid, so `make dev-stop` can shut this down from another terminal — and so the shutdown
# path is testable without a terminal, which is the only way to be sure Ctrl-C leaves nothing
# behind rather than merely believing it.
PIDFILE="${TMPDIR:-/tmp}/xustive-dev.pid"
echo "$$" >"$PIDFILE"

# Kill a process and everything below it, depth first.
#
# `cargo run` execs the real binary as a *child*, so signalling the cargo pid alone leaves the
# server holding port 8080. The symptom is not an error — it is the next run appearing to ignore
# your code changes, which is a genuinely horrible afternoon.
kill_tree() {
  local pid="$1" signal="$2"
  [ -z "$pid" ] && return
  for child in $(pgrep -P "$pid" 2>/dev/null); do
    kill_tree "$child" "$signal"
  done
  kill "-$signal" "$pid" 2>/dev/null
}

# Stop everything, once, however we got here.
cleanup() {
  trap - INT TERM EXIT
  echo
  say "stopping"
  for pid in "${PIDS[@]:-}"; do
    kill_tree "$pid" TERM
  done
  # A moment to drain: the crawler finishes its in-flight fetch, the worker acks what it has.
  sleep 3
  for pid in "${PIDS[@]:-}"; do
    kill_tree "$pid" KILL
  done
  wait 2>/dev/null
  rm -f "$PIDFILE"
  say "stopped. Infrastructure is still up — 'make dev-down' to stop it too."
}
trap cleanup INT TERM EXIT

# Run a command in the background, prefixing every line it writes.
#
# Process substitution rather than a pipe, so `$!` is the *service's* pid. With `cmd | sed &`,
# `$!` is sed's — which is what the first version of this captured, so cleanup signalled the log
# formatter and left every service running.
start() {
  local name="$1" colour="$2"
  shift 2
  "$@" > >(sed -u "s/^/${colour}${name}${C_OFF} │ /") 2>&1 &
  PIDS+=("$!")
}

# --- 0. is something already running? -------------------------------------------------------
#
# Refuse rather than start alongside. A second API cannot bind 8080 and a second Next cannot bind
# 3000, so what you get is a stack that half-starts and a log full of "address in use" — and,
# worse, the *old* processes keep serving, so your code changes appear to do nothing.
busy=""
for port in 8080 3000; do
  if curl -fsS --max-time 1 "http://localhost:$port" >/dev/null 2>&1 ||
    curl -fsS --max-time 1 "http://localhost:$port/healthz" >/dev/null 2>&1; then
    busy="$busy $port"
  fi
done
if [ -n "$busy" ]; then
  say "something is already listening on:$busy"
  echo "  Free the ports (stops a previous run and anything holding them):  make dev-down"
  echo "  Then run 'make dev' again."
  exit 1
fi

# --- 1. infrastructure ----------------------------------------------------------------------
say "starting infrastructure"
make dev-up || {
  say "infrastructure failed to start"
  exit 1
}

# --- 2. build ------------------------------------------------------------------------------
# Everything, before anything starts. A compile error after three services are up leaves a
# half-running stack and a confusing error, and the first build links llama.cpp from source —
# minutes, during which nothing would answer and it would look broken.
say "building (the first build compiles llama.cpp — several minutes; --fast skips it)"
# shellcheck disable=SC2086
# GPU is the default and a build-time decision: the cuda feature needs nvcc, so it is chosen here by
# detecting the toolkit rather than by config, and the device preference is set to `gpu` so the API
# prefers the card (falling back to CPU only if the model does not fit). `--cpu` forces CPU; `--fast`
# also stays on CPU because it skips the summariser (and llama.cpp) entirely, which is the whole
# point of it — cuda would drag it straight back in.
if [ "$FORCE_CPU" -eq 1 ]; then
  export XUSTIVE_DEVICE=cpu
  echo "  --cpu: running on the CPU (the GPU, if any, is not used)"
elif [ -z "$API_FEATURES" ] && [ -x /opt/cuda/bin/nvcc ]; then
  API_FEATURES="--features cuda"
  export PATH=/opt/cuda/bin:$PATH CUDA_PATH=/opt/cuda CUDACXX=/opt/cuda/bin/nvcc
  export XUSTIVE_DEVICE=gpu
  echo "  CUDA toolkit found — building the API with GPU support (use --cpu to force CPU)"
elif [ -z "$API_FEATURES" ]; then
  echo "  no CUDA toolkit (/opt/cuda/bin/nvcc) — building CPU-only"
fi

# The crawler and worker are the throughput-critical, CPU-bound paths (HTML parse, dedup,
# batch submission), so build them optimised — a debug crawler/indexer runs many times slower
# for no dev benefit. The API stays debug for fast iteration (and its CUDA build is heavy).
cargo build -p xustive-api $API_FEATURES && cargo build --release -p xustive-cli || {
  say "build failed — nothing started"
  exit 1
}

if [ ! -d web/node_modules ]; then
  say "installing frontend dependencies (first run only)"
  (cd web && npm install) || exit 1
fi

# --- 3. the processes ----------------------------------------------------------------------
say "starting services"

# shellcheck disable=SC2086
start "api    " "$C_API" cargo run -q -p xustive-api $API_FEATURES -- --config "$CONFIG"

# The API owns the index settings, so the worker and crawler wait for it rather than racing it.
until curl -fsS --max-time 2 http://localhost:8080/healthz >/dev/null 2>&1; do
  sleep 1
done
say "api ready"

start "web    " "$C_WEB" npm --prefix web run dev
start "worker " "$C_WORK" cargo run -q --release -p xustive-cli -- --config "$CONFIG" worker
start "toold  " "$C_TOOL" cargo run -q -p xustive-toold -- --once

if [ "$WITH_CRAWLER" -eq 1 ]; then
  start "crawler" "$C_CRAWL" cargo run -q --release -p xustive-cli -- --config "$CONFIG" crawld
fi

cat <<EOF

  ${C_SYS}Xustive is running.${C_OFF}

    Search      http://localhost:3000
    Admin       http://localhost:8080/admin
    Crawler doc http://localhost:8080/bot
    Grafana     http://localhost:3001

  Ctrl-C stops everything. Infrastructure stays up.

EOF

# Wait on the children. `wait` returns when any exits, so the loop keeps going until they are all
# gone or the trap fires — a service crashing should not silently leave the rest running.
wait
