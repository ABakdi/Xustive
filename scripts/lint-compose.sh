#!/usr/bin/env bash
#
# Asserts the production topology stays production-shaped.
#
# The base compose file must never publish a port for a backing service. Meilisearch, Qdrant and
# Redis hold the entire index and all crawl state, and none of them has meaningful authentication
# in front of it by default — exposing one is the difference between an internal service and a
# public database.
#
# Development access lives in docker-compose.dev.yml, which is opt-in.

set -euo pipefail

cd "$(dirname "$0")/.."

BASE=deploy/docker-compose.yml
fail=0

if [ ! -f "$BASE" ]; then
  echo "✗ compose lint: $BASE not found" >&2
  exit 1
fi

# Any `ports:` key in the base file is a mistake, whichever service it is under.
if grep -nE '^\s*ports:' "$BASE" >/dev/null 2>&1; then
  echo "✗ compose lint: $BASE publishes ports" >&2
  grep -nE -A 2 '^\s*ports:' "$BASE" | sed 's/^/    /' >&2
  echo "  Move host port mappings to deploy/docker-compose.dev.yml." >&2
  fail=1
fi

# The internal networks must stay internal.
for net in core obs; do
  if ! awk -v n="$net" '
        $0 ~ "^  " n ":" { found = 1; next }
        found && /internal: true/ { ok = 1; exit }
        found && /^  [a-z]+:/ { exit }
        END { exit !ok }
      ' "$BASE"; then
    echo "✗ compose lint: network '$net' is not marked internal: true in $BASE" >&2
    fail=1
  fi
done

# Every service needs a resource limit. Without one, a runaway indexing job takes the whole
# machine with it, and the symptom is a hung search rather than anything that names the cause.
#
# Checked with mem_limit/cpus rather than deploy.resources: `docker compose up` ignores
# deploy.resources outside Swarm, so that spelling looks like a limit and enforces nothing.
services=$(awk '/^services:/ { in_s = 1; next }
                in_s && /^[a-z]/ { in_s = 0 }
                in_s && /^  [a-z][a-z0-9_-]*:/ { gsub(/[ :]/, ""); print }' "$BASE")
for svc in $services; do
  if ! awk -v s="$svc" '
        $0 ~ "^  " s ":$" { found = 1; next }
        found && /^    mem_limit:/ { ok = 1 }
        found && /^  [a-z]/ { exit !ok }
        END { exit !ok }
      ' "$BASE"; then
    echo "✗ compose lint: service '$svc' has no mem_limit in $BASE" >&2
    fail=1
  fi
done

if [ "$fail" -eq 0 ]; then
  echo "✓ compose lint: no published ports, internal networks intact, every service capped"
fi

exit "$fail"
