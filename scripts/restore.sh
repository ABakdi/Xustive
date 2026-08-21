#!/usr/bin/env bash
#
# Restore drill: rebuild the stateful stores from a backup directory only (M4-T04.5).
#
# The point of the drill is to prove a backup is *restorable*, not merely that it was written — the
# failure everyone discovers too late is a backup that never round-tripped. Run it against a staging
# environment: wipe it, run this, and confirm search works.
#
#   scripts/restore.sh backups/2026-08-21T06-25-21Z
#
# It is DESTRUCTIVE to the target's Qdrant collections and Redis, so it refuses to run unless
# CONFIRM=yes is set — a guard against pointing it at production by muscle memory.
#
# Meilisearch restore is intentionally NOT automated here: Meili imports a snapshot at *startup*
# (`--import-snapshot <file> --import-dump`), so restoring it means stopping the node, placing the
# file, and restarting with that flag — an orchestration step, not a curl. The steps are printed.

set -euo pipefail

QDRANT_URL="${QDRANT_URL:-http://127.0.0.1:6333}"
REDIS_CONTAINER="${REDIS_CONTAINER:-xustive-redis}"
MEILI_CONTAINER="${MEILI_CONTAINER:-xustive-meilisearch}"

cd "$(dirname "$0")/.."
SRC="${1:-}"
[ -n "$SRC" ] && [ -d "$SRC" ] || { echo "usage: scripts/restore.sh <backup-dir>" >&2; exit 2; }

if [ "${CONFIRM:-}" != "yes" ]; then
  echo "This restores from $SRC and will OVERWRITE Qdrant collections and Redis on:" >&2
  echo "  QDRANT_URL=$QDRANT_URL  REDIS_CONTAINER=$REDIS_CONTAINER" >&2
  echo "Re-run with CONFIRM=yes once you are sure this is a staging target." >&2
  exit 3
fi

echo "→ restoring from $SRC"

# --- Qdrant ---------------------------------------------------------------------------------------
# Upload each snapshot and recover the collection from it. `?priority=snapshot` makes the snapshot
# authoritative over any existing data.
for snap in "$SRC"/qdrant-*.snapshot; do
  [ -e "$snap" ] || continue
  base="$(basename "$snap")"; c="${base#qdrant-}"; c="${c%.snapshot}"
  echo "  qdrant: recovering '$c' from $base"
  curl -fsS -X POST "$QDRANT_URL/collections/$c/snapshots/upload?priority=snapshot" \
    -H 'Content-Type:multipart/form-data' -F "snapshot=@$snap" >/dev/null \
    && echo "    ok" || echo "    ⚠ failed"
done

# --- Redis ----------------------------------------------------------------------------------------
# Replace dump.rdb and restart so Redis loads it. (A live SHUTDOWN NOSAVE + copy + start is the
# manual equivalent; here we copy in and restart the container.)
if [ -f "$SRC/redis-dump.rdb" ] && command -v docker >/dev/null 2>&1; then
  echo "  redis: loading dump.rdb (container restart)"
  docker cp "$SRC/redis-dump.rdb" "$REDIS_CONTAINER:/data/dump.rdb"
  docker restart "$REDIS_CONTAINER" >/dev/null && echo "    ok" || echo "    ⚠ restart failed"
else
  echo "  redis: no dump.rdb in backup or docker unavailable — skipped"
fi

# --- Meilisearch (manual) -------------------------------------------------------------------------
if [ -f "$SRC/meili.snapshot" ]; then
  cat <<EOF
  meili: import is a startup step, not scripted here. To restore:
    1. Stop the Meilisearch node.
    2. Place $SRC/meili.snapshot where the node can read it (e.g. docker cp into $MEILI_CONTAINER).
    3. Start it with:  meilisearch --import-snapshot <path/to/meili.snapshot> --ignore-snapshot-if-db-exists=false
    4. Verify:  curl \$MEILI_URL/indexes  and run a search.
EOF
else
  echo "  meili: no snapshot in backup — nothing to import"
fi

# --- Registry -------------------------------------------------------------------------------------
[ -f "$SRC/registry.jsonl" ] && echo "  registry: $SRC/registry.jsonl — restore into place with git or a copy as your process dictates"

cat <<EOF

→ restore steps issued. Now VERIFY (this is the drill's actual pass/fail):
    - xustive-cli stats        → document counts match the backup's era
    - a search returns results
    - xustive-cli reconcile-vectors --dry-run  → vector/document counts line up
Measure the wall-clock from wipe to a working search: that is your RTO (M4-T04.6).
EOF
