#!/usr/bin/env bash
#
# Back up every stateful store to an off-host directory (M4-T04.1/.2/.3/.4).
#
# The three data stores each have their own snapshot mechanism, and this script drives all of them
# to one timestamped directory so a restore has a single coherent point in time to work from:
#
#   - Meilisearch : POST /snapshots  →  a .snapshot in the data volume, copied out (T04.1)
#   - Qdrant      : POST /collections/{c}/snapshots, then downloaded over HTTP (T04.2)
#   - Redis       : BGSAVE, then dump.rdb copied out; the AOF is already on (T04.3)
#   - Registry    : the git-versioned source registry, copied for completeness (T04.4)
#
# Nothing here is destructive. Restore is scripts/restore.sh, deliberately separate.
#
# Usage:
#   scripts/backup.sh [BACKUP_ROOT]        # default ./backups
#   MEILI_URL=... QDRANT_URL=... REDIS_CONTAINER=... MEILI_CONTAINER=... scripts/backup.sh
#
# Off-host: point BACKUP_ROOT at a mounted volume / object-store gateway. Where that physically
# lives, given data sovereignty, is the open question in Deployment Topology §7 (M4-T04.7).

set -euo pipefail

MEILI_URL="${MEILI_URL:-http://127.0.0.1:7700}"
# How long to wait for the snapshot. It queues behind indexing and then writes the whole index.
MEILI_SNAPSHOT_TIMEOUT="${MEILI_SNAPSHOT_TIMEOUT:-1800}"
QDRANT_URL="${QDRANT_URL:-http://127.0.0.1:6333}"
MEILI_CONTAINER="${MEILI_CONTAINER:-xustive-meilisearch}"
REDIS_CONTAINER="${REDIS_CONTAINER:-xustive-redis}"
QDRANT_COLLECTIONS="${QDRANT_COLLECTIONS:-image_clip}"
REGISTRY="${REGISTRY:-data/sources/registry.jsonl}"

cd "$(dirname "$0")/.."
STAMP="$(date -u +%Y-%m-%dT%H-%M-%SZ)"
DEST="${1:-backups}/$STAMP"
mkdir -p "$DEST"

echo "→ backing up to $DEST"
warned=0
failed=0
warn() { echo "  ⚠ $*" >&2; warned=1; }
# Meilisearch holds the corpus: everything else can be rebuilt from it or from the repository, and
# it cannot be rebuilt from them. A run that does not capture it has not backed anything up worth
# having, so it fails rather than reporting a warning nobody reads.
fatal() { echo "  ✗ $*" >&2; failed=1; }

# --- Meilisearch ---------------------------------------------------------------------------------
# On-demand snapshot: enqueue, poll the task, then copy the file out of the container volume.
meili_backup() {
  local task uid status
  task="$(curl -fsS -X POST "$MEILI_URL/snapshots")" || { warn "meili: snapshot request failed"; return; }
  uid="$(printf '%s' "$task" | sed -n 's/.*"taskUid":\([0-9]*\).*/\1/p')"
  [ -n "$uid" ] || { warn "meili: no taskUid in response"; return; }
  # Two minutes was not enough and the difference is invisible: the poll gave up, the file was
  # not there yet, and the run reported "completed with warnings" while shipping a backup with no
  # corpus in it. A snapshot queues behind whatever the indexer is doing and then writes the whole
  # index — on a 24 GB index, tens of minutes. Wait properly, say what it is waiting for, and let
  # the operator raise the ceiling.
  local waited=0 every=5
  echo "  meili: snapshot task $uid enqueued; waiting (up to ${MEILI_SNAPSHOT_TIMEOUT}s)…"
  while :; do
    status="$(curl -fsS "$MEILI_URL/tasks/$uid" | sed -n 's/.*"status":"\([a-z]*\)".*/\1/p')"
    case "$status" in
      succeeded) break ;;
      failed|canceled)
        fatal "meili: snapshot $status — the corpus is NOT backed up"
        return ;;
    esac
    if [ "$waited" -ge "$MEILI_SNAPSHOT_TIMEOUT" ]; then
      fatal "meili: snapshot still '$status' after ${waited}s — the corpus is NOT backed up. \
Raise MEILI_SNAPSHOT_TIMEOUT, or pause the crawler (the snapshot queues behind indexing)."
      return
    fi
    # A long wait with no output looks like a hang; say so once a minute.
    if [ $((waited % 60)) -eq 0 ] && [ "$waited" -gt 0 ]; then
      echo "  meili: still $status after ${waited}s…"
    fi
    sleep "$every"
    waited=$((waited + every))
  done
  # The snapshot lands in Meili's snapshot directory; copy it out if docker is available. The path
  # depends on MEILI_SNAPSHOT_DIR / the deployment — try the common locations and warn if none held
  # a file, rather than silently shipping a backup with no Meili in it.
  if command -v docker >/dev/null 2>&1; then
    local f=""
    for dir in ${MEILI_SNAPSHOT_DIR:-} /meili_data/snapshots /meili_data/data.ms/snapshots /snapshots; do
      [ -n "$dir" ] || continue
      f="$(docker exec "$MEILI_CONTAINER" sh -c "ls -t $dir/*.snapshot 2>/dev/null | head -1" | tr -d '\r')"
      [ -n "$f" ] && break
    done
    # Last resort: ask the filesystem rather than guessing where this build writes.
    if [ -z "$f" ]; then
      f="$(docker exec "$MEILI_CONTAINER" sh -c "find / -name '*.snapshot' -type f 2>/dev/null | head -1" | tr -d '\r')"
    fi
    # Check there is room before starting: a `docker cp` that fills the disk leaves a truncated
    # file that looks like a backup, and takes the machine down with it. (Found the hard way on
    # 2026-09-02 — the copy died at "disk quota exceeded" halfway through 5 GB.)
    if [ -n "$f" ]; then
      local need avail
      need="$(docker exec "$MEILI_CONTAINER" stat -c %s "$f" 2>/dev/null || echo 0)"
      avail="$(df -PB1 "$DEST" 2>/dev/null | tail -1 | awk '{print $4}')"
      if [ "${need:-0}" -gt 0 ] && [ "${avail:-0}" -gt 0 ] &&
         [ "$avail" -lt $((need + need / 10)) ]; then
        fatal "meili: the snapshot is $((need / 1000000)) MB and only $((avail / 1000000)) MB is free at $DEST — the corpus is NOT backed up"
        return
      fi
    fi
    if [ -n "$f" ] && docker cp "$MEILI_CONTAINER:$f" "$DEST/meili.snapshot" 2>/dev/null; then
      echo "  meili: → $DEST/meili.snapshot"
    else
      fatal "meili: snapshot task succeeded but no .snapshot file was found — the corpus is NOT backed up. Set MEILI_SNAPSHOT_DIR to this deployment's snapshot directory"
    fi
  else
    fatal "meili: snapshot created in the volume, but docker is unavailable to copy it out — the corpus is NOT backed up"
  fi
}

# --- Qdrant --------------------------------------------------------------------------------------
# Snapshot per collection, then download over HTTP (no container access needed).
qdrant_backup() {
  local c name
  for c in $QDRANT_COLLECTIONS; do
    resp="$(curl -fsS -X POST "$QDRANT_URL/collections/$c/snapshots")" || { warn "qdrant: '$c' snapshot failed (missing collection?)"; continue; }
    name="$(printf '%s' "$resp" | sed -n 's/.*"name":"\([^"]*\)".*/\1/p')"
    [ -n "$name" ] || { warn "qdrant: '$c' no snapshot name"; continue; }
    if curl -fsS "$QDRANT_URL/collections/$c/snapshots/$name" -o "$DEST/qdrant-$c.snapshot"; then
      echo "  qdrant: → $DEST/qdrant-$c.snapshot"
    else
      warn "qdrant: '$c' snapshot download failed"
    fi
  done
}

# --- Redis ---------------------------------------------------------------------------------------
# BGSAVE writes dump.rdb in the container; copy it out. The AOF (appendonly yes) is the finer-grained
# record and is captured by a volume backup; the RDB is the point-in-time copy this ships.
#
# DELIBERATELY only the queue Redis. The signals instance (xustive-redis-signals) holds the
# interaction counters and weak-coverage terms and must NEVER be backed up (BUG-034, ADR-0018):
# a backup of windowed identifier-free counters is a durable query log with none of those
# properties. Do not add it here.
redis_backup() {
  command -v docker >/dev/null 2>&1 || { warn "redis: docker unavailable; cannot BGSAVE/copy"; return; }
  docker exec "$REDIS_CONTAINER" redis-cli BGSAVE >/dev/null 2>&1 || { warn "redis: BGSAVE failed"; return; }
  # BGSAVE is asynchronous; wait for rdb_bgsave_in_progress to clear.
  for _ in $(seq 1 60); do
    if docker exec "$REDIS_CONTAINER" redis-cli INFO persistence 2>/dev/null | grep -q 'rdb_bgsave_in_progress:0'; then
      break
    fi
    sleep 1
  done
  if docker cp "$REDIS_CONTAINER:/data/dump.rdb" "$DEST/redis-dump.rdb" 2>/dev/null; then
    echo "  redis: → $DEST/redis-dump.rdb"
  else
    warn "redis: could not copy dump.rdb"
  fi
}

# --- Registry ------------------------------------------------------------------------------------
registry_backup() {
  if [ -f "$REGISTRY" ]; then
    cp "$REGISTRY" "$DEST/registry.jsonl"
    echo "  registry: → $DEST/registry.jsonl (also git-versioned)"
  else
    warn "registry: $REGISTRY not found"
  fi
}

meili_backup
qdrant_backup
redis_backup
registry_backup

# A manifest so a restore knows what a directory contains and when it was taken.
{
  echo "backup_utc: $STAMP"
  echo "meili_url: $MEILI_URL"
  echo "qdrant_url: $QDRANT_URL"
  echo "qdrant_collections: $QDRANT_COLLECTIONS"
  echo "artifacts:"
  ( cd "$DEST" && ls -1 | grep -v '^manifest.txt$' | sed 's/^/  - /' )
} >"$DEST/manifest.txt"

echo "→ manifest:"
sed 's/^/  /' "$DEST/manifest.txt"
if [ "${failed:-0}" -eq 1 ]; then
  echo "→ FAILED: the Meilisearch corpus was not captured. This backup is not a backup." >&2
  exit 1
fi
if [ "$warned" -eq 1 ]; then
  echo "→ completed with warnings (see above). This is expected where a store or docker is absent." >&2
else
  echo "→ backup complete."
fi
