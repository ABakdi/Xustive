#!/usr/bin/env bash
#
# Stop everything `make dev` started, and **free the ports it uses** so the next `make dev` is not
# refused with "address in use".
#
# `make dev` runs the API and the frontend as *host* processes (not containers), so
# `docker compose down` alone leaves them holding ports 8080 and 3000. A killed terminal or a crash
# orphans them, and then the next `make dev` trips over "port in use" — the exact thing this fixes.
#
#   ./scripts/dev-down.sh            # stop infra + free the ports; keep the data volumes
#   ./scripts/dev-down.sh --clean    # also DELETE the data volumes (asks first: type 'yes')
#   ./scripts/dev-down.sh --clean --yes   # for scripts: skip the prompt
set -uo pipefail

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.dev.yml"
# The host ports `make dev` binds. The infra ports (redis/meili/qdrant/…) live in containers and
# are released by `compose down`, so they are not force-killed here.
DEV_PORTS="8080 3000"

CLEAN=0
ASSUME_YES=0
for arg in "$@"; do
  case "$arg" in
  --clean) CLEAN=1 ;;
  --yes | -y) ASSUME_YES=1 ;;
  *)
    echo "usage: $0 [--clean] [--yes]" >&2
    exit 2
    ;;
  esac
done

if [ -t 1 ]; then B=$'\033[1m'; O=$'\033[0m'; else B=""; O=""; fi

# 1. Stop the `make dev` process tree cleanly, if it left a pidfile. Signalling the parent lets its
#    own trap drain the crawler and worker before they are killed.
PIDFILE="${TMPDIR:-/tmp}/xustive-dev.pid"
pid=$(cat "$PIDFILE" 2>/dev/null || true)
if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
  echo "▸ stopping the 'make dev' process tree (pid $pid)"
  kill -INT "$pid" 2>/dev/null || true
  for _ in $(seq 1 10); do kill -0 "$pid" 2>/dev/null || break; sleep 1; done
  kill -KILL "$pid" 2>/dev/null || true
fi
rm -f "$PIDFILE"

# 2. Free the host ports. Anything still bound to them is a straggler (an orphaned run, an IDE
#    launch, a crash) — `fuser -k` signals every process on the port so the next run starts clean.
freed=""
for port in $DEV_PORTS; do
  if fuser -s "$port/tcp" 2>/dev/null; then
    echo "▸ freeing port $port"
    fuser -k -TERM "$port/tcp" 2>/dev/null || true
    sleep 1
    fuser -k -KILL "$port/tcp" 2>/dev/null || true
    freed="$freed $port"
  fi
done

# 3. Belt and braces: our own binaries by name, in case one is mid-restart and not yet on a port.
for pattern in "xustive-api" "xustive-cli .*worker" "xustive-cli .*crawld" "xustive-toold" "next dev"; do
  pkill -f "$pattern" 2>/dev/null || true
done

# 4. The containers — and, with --clean, the data volumes too.
if [ "$CLEAN" -eq 1 ]; then
  echo
  echo "${B}--clean deletes every data volume${O}: the search index, the crawl frontier, the index"
  echo "queue and dead letters, cached robots rules, and tool data. Everything crawled is gone."
  echo "  Kept: your seed list, config, and code."
  echo "  Re-crawling costs the sites their bandwidth, not only your time."
  echo
  if [ "$ASSUME_YES" -ne 1 ]; then
    printf "  Type 'yes' to delete the volumes: "
    read -r reply || reply=""
    if [ "$reply" != "yes" ]; then
      echo "  cancelled the volume delete — stopping containers only, volumes kept"
      $COMPOSE down
      echo "${B}Stopped.${O} Ports freed:${freed:- none}. Data volumes kept."
      exit 0
    fi
  fi
  echo "▸ removing containers and volumes"
  $COMPOSE down -v
  echo "${B}Clean.${O} Ports freed:${freed:- none}. Nothing is running and nothing is stored."
else
  echo "▸ stopping containers (volumes kept)"
  $COMPOSE down
  echo "${B}Stopped.${O} Ports freed:${freed:- none}. 'make dev' will start cleanly."
fi
