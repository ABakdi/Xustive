#!/usr/bin/env bash
#
# Wipe everything and start clean.
#
# Deletes the search index, the crawl frontier, the index queue, the dead letters, the cached
# robots rules and the tool data. Everything crawled is gone.
#
# # Why this exists as its own command
#
# `dev-reset` removed the volumes and nothing else, which left the API, crawler and worker running
# against infrastructure that had just been deleted — they reconnect to an empty Redis and a
# missing index and behave in ways that look like new bugs.
#
# It also had no confirmation. Re-crawling costs the *sites* bandwidth, not just time, so a wipe is
# not something to trip over.
#
# # The failure this was written for
#
# Meilisearch reached a state where its scheduler could not find its own update files:
#
#     ERROR file_store: Can't access update file …: No such file or directory
#     ERROR index_scheduler: No such file or directory (os error 2)
#
# It then sat idle at 0.2% CPU with hundreds of tasks it could never process, so every submission
# queued behind a dead engine and timed out. Nothing in our code could recover from that; the task
# database and the update files had diverged on disk.
#
#   ./scripts/reset.sh          # asks first
#   ./scripts/reset.sh --yes    # for scripts
set -uo pipefail

cd "$(dirname "$0")/.."

ASSUME_YES=0
for arg in "$@"; do
  case "$arg" in
  --yes | -y) ASSUME_YES=1 ;;
  *)
    echo "usage: $0 [--yes]" >&2
    exit 2
    ;;
  esac
done

if [ -t 1 ]; then B=$'\033[1m'; O=$'\033[0m'; else B=""; O=""; fi

# Report what is about to be lost, in the terms that matter. "Delete all volumes" does not tell
# anyone how much work they are discarding.
docs="unknown"
if curl -fsS --max-time 3 http://localhost:8080/healthz >/dev/null 2>&1; then
  docs=$(curl -fsS --max-time 5 "http://localhost:8080/admin/crawler/status" 2>/dev/null |
    python3 -c 'import sys,json; print(json.load(sys.stdin).get("indexed","unknown"))' 2>/dev/null || echo unknown)
fi

echo
echo "${B}This deletes everything the engine has collected.${O}"
echo
echo "  · the search index          (documents crawled: $docs)"
echo "  · the crawl frontier        (every discovered URL)"
echo "  · the index queue and dead letters"
echo "  · cached robots.txt rules and tool data"
echo
echo "  Kept: your seed list, config, and code."
echo
echo "  Re-crawling costs the sites their bandwidth, not only your time."
echo

if [ "$ASSUME_YES" -ne 1 ]; then
  printf "  Type 'delete' to confirm: "
  read -r reply
  if [ "$reply" != "delete" ]; then
    echo "  cancelled"
    exit 1
  fi
fi

# 1. Stop the application processes first.
#
# Before the volumes, not after: a crawler still running while Redis is recreated writes a frontier
# into the fresh instance, so the "clean" state is dirty before you have looked at it.
echo
echo "▸ stopping application processes"
make dev-stop >/dev/null 2>&1 || true
for pattern in "xustive-api" "xustive-cli" "xustive-toold" "next dev" "next start"; do
  pkill -f "target/[a-z]*/$pattern" 2>/dev/null || true
  pkill -f "$pattern --config" 2>/dev/null || true
done
sleep 2

# 2. Then the data.
echo "▸ removing containers and volumes"
make dev-down >/dev/null 2>&1 || true
docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.dev.yml down -v >/dev/null 2>&1 || true

echo
echo "${B}Clean.${O} Nothing is running and nothing is stored."
echo
echo "  Start again with:"
echo "    make up      # infrastructure, index settings, sample corpus"
echo "    make dev     # everything, including the crawler"
echo
