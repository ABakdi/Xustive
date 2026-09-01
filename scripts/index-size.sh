#!/usr/bin/env bash
# PROB-004, as a command. Meilisearch memory-maps its index and a cgroup limit counts the page
# cache, so an index larger than the container's limit is read back from disk on every batch and
# indexing collapses (measured: 260 → 8 documents a minute). This is the number to watch.
set -euo pipefail
cd "$(dirname "$0")/.."

url="${MEILI_URL:-http://127.0.0.1:7700}"
key="${MEILI_MASTER_KEY:-${MEILI_KEY:-}}"
auth=()
[ -n "$key" ] && auth=(-H "Authorization: Bearer $key")

used=$(curl -fsS "${auth[@]}" "$url/stats" | python3 -c 'import sys,json; print(json.load(sys.stdin)["usedDatabaseSize"])' 2>/dev/null || echo 0)
limit=$(docker inspect xustive-meilisearch --format '{{.HostConfig.Memory}}' 2>/dev/null || echo 0)

human() { python3 -c "print(f'{$1/1e9:.1f} GB')"; }
echo "index in use : $(human "$used")"
if [ "$limit" = 0 ]; then
  echo "memory limit : (container not running — check deploy/docker-compose.yml)"
  exit 0
fi
echo "memory limit : $(human "$limit")"
python3 - "$used" "$limit" <<'PY'
import sys
used, limit = int(sys.argv[1]), int(sys.argv[2])
ratio = used / limit if limit else 0
if ratio > 0.8:
    print(f"\n  RAISE IT. The index is at {ratio:.0%} of the limit; past it indexing collapses.")
    print("  deploy/docker-compose.yml → meilisearch.mem_limit, then: docker compose … up -d meilisearch")
    sys.exit(1)
print(f"\n  Headroom: {(1-ratio):.0%} of the limit free.")
PY
