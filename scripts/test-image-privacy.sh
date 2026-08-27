#!/usr/bin/env bash
# A reverse image search leaves no trace (M10-T05.3).
#
# Posts a generated picture to the API, then looks for it — its bytes' hash, and the words it was
# described with — in every log file given and in both Redis keyspaces. The words are the one
# thing that legitimately leaves the machine (to the metasearch engine, ADR-0028); they must still
# not be *kept* anywhere here, and the picture must not appear at all.
#
#   ./scripts/test-image-privacy.sh [logfile...]
set -uo pipefail
API="${API:-http://127.0.0.1:8080}"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
pass=0; fail=0
ok()  { printf '  \033[32m✓\033[0m %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  \033[31m✗\033[0m %s\n' "$1"; fail=$((fail + 1)); }

# A picture nobody has: noise, so its hash and its words are this run's alone.
ffmpeg -loglevel error -y -f lavfi -i "nullsrc=s=96x96:d=1,geq=random(1)*255:128:128" -frames:v 1 "$tmp/q.png" \
  || { echo "ffmpeg is needed"; exit 2; }
hash=$(sha256sum "$tmp/q.png" | cut -c1-16)
reply=$(curl -sS -X POST -H 'Content-Type: image/png' --data-binary "@$tmp/q.png" "$API/api/v1/search/image")
words=$(printf '%s' "$reply" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d["query"].get("web_query") or "")' 2>/dev/null || true)
[ -n "$reply" ] && ok "the search answered" || bad "the search did not answer (is [vector] on?)"

needles=("$hash")
[ -n "$words" ] && needles+=("$words")
for f in "$@"; do
  [ -r "$f" ] || continue
  for n in "${needles[@]}"; do
    grep -qF -- "$n" "$f" && bad "$(basename "$f") contains '$n'" || ok "$(basename "$f") does not contain '$n'"
  done
done
for c in xustive-redis xustive-redis-signals; do
  if docker ps --format '{{.Names}}' 2>/dev/null | grep -qx "$c"; then
    keys=$(docker exec "$c" redis-cli --scan 2>/dev/null || true)
    for n in "${needles[@]}"; do
      printf '%s' "$keys" | grep -qF -- "$n" && bad "$c has a key naming '$n'" || ok "$c has no key naming '$n'"
    done
  fi
done
echo; echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
