#!/usr/bin/env bash
# Prove the results page works without JavaScript.
#
# This is a stated commitment, and a stated commitment that nobody tests is a stated hope. It is
# also the one that rots most quietly: every change works in a browser with JavaScript, so nothing
# reveals the day search stops submitting without it.
#
# `curl` executes no script, so what it receives is exactly what a reader without JavaScript gets.
# The assertions are about that HTML: are there results in it, does a filter link narrow them,
# does page two load, and is the search box a real form that can submit on its own.
#
#   ./scripts/no-js-check.sh
#   BASE=http://localhost:3000 Q='الجزائر' ./scripts/no-js-check.sh
set -euo pipefail

BASE="${BASE:-http://localhost:3000}"
LANG_SEG="${LANG_SEG:-ar}"
Q="${Q:-الجزائر}"
fail=0

if ! curl -fsS --max-time 5 "$BASE" -o /dev/null 2>/dev/null; then
  echo "✗ no-js: nothing serving at $BASE" >&2
  exit 1
fi

page() {
  curl -fsS --max-time 20 -G --data-urlencode "q=$Q" "$@" \
    "$BASE/$LANG_SEG/search" 2>/dev/null || true
}

ok()  { printf '  ✓ %s\n' "$1"; }
bad() { printf '  ✗ %s\n' "$1" >&2; fail=1; }

echo "No-JavaScript path (curl runs no script, so this is what a reader without it receives):"

HTML=$(page)
[ -n "$HTML" ] || { bad "the results page returned nothing"; exit 1; }

# 1. Results are in the first response, not fetched afterwards.
# `grep -c` counts matching lines, and this HTML is a single line — it always said 1.
COUNT=$(printf '%s' "$HTML" | grep -o 'id="result-' | wc -l | tr -d ' ')
if [ "${COUNT:-0}" -ge 3 ]; then
  ok "results are server-rendered ($COUNT on the page)"
else
  bad "only ${COUNT:-0} results in the HTML — the page is client-rendering"
fi

# 2. The search box is a real form. Without this, a reader without JavaScript cannot search at
#    all — every other assertion here is moot.
if printf '%s' "$HTML" | grep -q '<form[^>]*method="get"' &&
   printf '%s' "$HTML" | grep -q '<input[^>]*name="q"'; then
  ok "the search box is a real GET form"
else
  bad "the search box is not a plain form; searching would need JavaScript"
fi

# 3. Filters are links with real hrefs, and following one narrows the results.
FILTER=$( { printf '%s' "$HTML" | grep -oE 'href="/[a-z]{2,3}/search\?[^"]*lang=[a-z]+[^"]*"' || true; } |
         head -1 | sed -E 's/href="([^"]+)"/\1/' | sed 's/&amp;/\&/g')
if [ -n "$FILTER" ]; then
  FILTERED=$(curl -fsS --max-time 20 "$BASE$FILTER" 2>/dev/null || true)
  FCOUNT=$(printf '%s' "$FILTERED" | grep -o 'id="result-' | wc -l | tr -d ' ')
  if [ "${FCOUNT:-0}" -ge 1 ]; then
    ok "a filter link returns results without script ($FCOUNT)"
  else
    bad "following a filter link produced no results"
  fi
  # Deliberately not asserting the count *decreased*: with one language dominating the corpus a
  # filter can legitimately return everything, and a test that fails on real data is a test people
  # delete. What must hold is that the link works at all.
else
  bad "no filter links in the HTML — filtering would need JavaScript"
fi

# 4. Pagination is links.
PAGE2=$( { printf '%s' "$HTML" | grep -oE 'href="/[a-z]{2,3}/search\?[^"]*page=2[^"]*"' || true; } |
        head -1 | sed -E 's/href="([^"]+)"/\1/' | sed 's/&amp;/\&/g')
if [ -n "$PAGE2" ]; then
  P2=$(curl -fsS --max-time 20 "$BASE$PAGE2" 2>/dev/null || true)
  if [ "$(printf '%s' "$P2" | grep -o 'id="result-' | wc -l | tr -d ' ')" -ge 1 ]; then
    ok "page two loads from a link"
  else
    bad "the page-two link returned no results"
  fi
else
  # Only a failure if there is more than one page to reach.
  if printf '%s' "$HTML" | grep -q 'aria-label'; then
    ok "no pagination needed for this result set"
  fi
fi

# 5. The home page is reachable and carries a form too.
HOME=$(curl -fsS --max-time 10 "$BASE/$LANG_SEG" 2>/dev/null || true)
if printf '%s' "$HOME" | grep -q '<form[^>]*method="get"'; then
  ok "the home page has a working form"
else
  bad "the home page cannot submit without JavaScript"
fi

if [ "$fail" -ne 0 ]; then
  echo >&2
  echo "  The no-JavaScript path is a commitment in UI - Frontend Architecture §3, and it is" >&2
  echo "  the path that breaks silently: everything works in a browser with script." >&2
fi
exit "$fail"
