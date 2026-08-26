#!/usr/bin/env bash
#
# Fetch the IP-to-city database the weather card uses to answer "weather" with no place in it.
#
# DB-IP City Lite, CC BY 4.0, updated monthly. It is fetched rather than committed because it is
# tens of megabytes of binary that changes every month — the wrong shape for git, and the same
# reason `fetch-models.sh` exists.
#
# Nothing depends on this file. Without it the serving plane simply never guesses a location, and
# a reader who names a place is unaffected ([[ADR-0020]]).
#
# The licence requires attribution, which the weather card carries.
set -euo pipefail

cd "$(dirname "$0")/.."
dest="data/geoip"
target="$dest/dbip-city-lite.mmdb"

# The publisher versions the file by month, so the current one is derived rather than pinned.
month="${1:-$(date -u +%Y-%m)}"
url="https://download.db-ip.com/free/dbip-city-lite-${month}.mmdb.gz"

mkdir -p "$dest"
echo "→ ${url}"
if ! curl -fsSL "$url" -o "${target}.gz"; then
  # The current month's file is published a few days in, so early in a month the previous one is
  # the newest that exists. Falling back is more useful than failing on the 1st.
  previous=$(date -u -d "${month}-01 -1 month" +%Y-%m 2>/dev/null || echo "")
  if [ -z "$previous" ]; then
    echo "✗ could not download ${url}" >&2
    exit 1
  fi
  echo "  not published yet; trying ${previous}"
  curl -fsSL "https://download.db-ip.com/free/dbip-city-lite-${previous}.mmdb.gz" -o "${target}.gz"
fi

gunzip -f "${target}.gz"
echo "✓ ${target} ($(du -h "$target" | cut -f1))"
echo "  DB-IP City Lite is CC BY 4.0 — the attribution is rendered on the weather card."
