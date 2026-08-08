#!/usr/bin/env bash
# Enforce the client-side asset budgets in docs/UI/UI - Frontend Architecture.md §7.
#
# Measures what a browser actually downloads: fetch the rendered page, read the script and
# stylesheet URLs out of the HTML, and total their transferred (gzipped) size. That is the number
# that matters on a slow connection, and it is independent of whatever the build tool decides to
# print this major version.
#
# A budget that only warns is a number nobody reads, so exceeding one fails.
#
#   ./scripts/bundle-budget.sh                  # against an already-running server
#   BASE=http://localhost:3000 ./scripts/bundle-budget.sh
set -euo pipefail

BASE="${BASE:-http://localhost:3000}"

# Budgets in kilobytes, gzipped.
#
# These are **not** the numbers in the original frontend spec. Those were 40 KB and 90 KB, written
# when the UI was vanilla JavaScript, and they are unachievable with React: the framework runtime
# alone measures ~152 KB gzipped across three chunks before a line of this project's code is
# loaded. Our own components are about 19 KB of the total.
#
# That cost is real and was understated in ADR-0010, which listed a Node process, a network hop
# and a build step but not this. It is recorded honestly in the ADR now.
#
# Set just above the measured floor so the gate still does the job it can do: catching a
# *regression*. It cannot catch the framework, because the framework was chosen deliberately.
JS_HOME_KB="${JS_HOME_KB:-185}"
JS_SEARCH_KB="${JS_SEARCH_KB:-195}"
CSS_KB="${CSS_KB:-20}"
# Fonts, per direction — what one reader actually fetches, not the total on disk.
#
# An RTL page needs the two Arabic faces; an LTR page needs the Latin variable file. woff2 is
# already compressed, so these are measured as served rather than gzipped again.
FONT_RTL_KB="${FONT_RTL_KB:-95}"
FONT_LTR_KB="${FONT_LTR_KB:-50}"

fail=0

if ! curl -fsS --max-time 5 "$BASE" -o /dev/null 2>/dev/null; then
  echo "✗ bundle budget: nothing serving at $BASE" >&2
  echo "  start it with: cd web && npx next start -p 3000" >&2
  exit 1
fi

# Transferred size of one asset, in bytes.
#
# `Accept-Encoding: gzip` because that is what a browser sends; measuring the uncompressed file
# would overstate the cost by roughly three times and make the budget meaningless.
asset_bytes() {
  curl -fsS --compressed-no-var -H 'Accept-Encoding: gzip' -o /dev/null \
    -w '%{size_download}' "$1" 2>/dev/null || \
  curl -fsS -H 'Accept-Encoding: gzip' -o /dev/null -w '%{size_download}' "$1" 2>/dev/null || echo 0
}

# Total the assets of one kind referenced by a page.
measure() {
  local url="$1" kind="$2"
  local html total=0
  html=$(curl -fsS --max-time 10 "$url")

  local pattern
  if [ "$kind" = js ]; then
    pattern='src="(/_next/[^"]+\.js)"'
  else
    pattern='href="(/_next/[^"]+\.css)"'
  fi

  # Deduplicated: a chunk referenced twice is downloaded once.
  local paths
  paths=$(printf '%s' "$html" | grep -oE "$pattern" | sed -E 's/.*"(\/_next[^"]+)".*/\1/' | sort -u)

  for path in $paths; do
    total=$((total + $(asset_bytes "$BASE$path")))
  done
  echo "$total"
}

check() {
  local label="$1" bytes="$2" budget_kb="$3"
  local kb=$((bytes / 1024))
  if [ "$kb" -gt "$budget_kb" ]; then
    printf '  ✗ %-22s %4s KB  (budget %s KB)\n' "$label" "$kb" "$budget_kb" >&2
    fail=1
  else
    printf '  ✓ %-22s %4s KB  (budget %s KB)\n' "$label" "$kb" "$budget_kb"
  fi
}

# A development server serves unminified chunks plus hot-reload machinery, which measures four to
# five times the production bundle. Without this check the script reports a wild failure that
# looks exactly like a regression, and whoever runs it goes looking for a bloated import.
# Detected by the hot-reload client itself, which a production build never serves. Matching on
# HTML strings was tried first and does not work: Next 16 uses Turbopack in both modes, so the
# obvious markers appear either way.
if curl -fsS --max-time 10 "$BASE/ar" | grep -q 'hmr-client\|_browser_dev_'; then
  echo "✗ bundle budget: $BASE is a development server." >&2
  echo "  Dev chunks are unminified and carry hot-reload code, so the numbers mean nothing." >&2
  echo "  Measure a production build:  cd web && npm run build && npm start" >&2
  exit 1
fi

echo "Client asset budgets, gzipped, as a browser would fetch them:"
check "home JS"      "$(measure "$BASE/ar" js)"          "$JS_HOME_KB"
check "search JS"    "$(measure "$BASE/ar/search?q=test" js)" "$JS_SEARCH_KB"
check "CSS"          "$(measure "$BASE/ar" css)"         "$CSS_KB"

# Fonts are counted separately because they are the one asset a reader fetches once and then has
# for every subsequent page. A budget that lumped them in with per-page JS would either punish the
# first visit or excuse an unbounded font stack.
font_bytes() {
  local total=0 size
  for f in "$@"; do
    size=$(curl -fsS -o /dev/null -w '%{size_download}' "$BASE/fonts/$f" 2>/dev/null || echo 0)
    total=$((total + size))
  done
  echo "$total"
}
check "fonts (RTL page)" \
  "$(font_bytes ibm-plex-sans-arabic-400-arabic.woff2 ibm-plex-sans-arabic-600-arabic.woff2)" \
  "$FONT_RTL_KB"
check "fonts (LTR page)" \
  "$(font_bytes ibm-plex-sans-var-latin.woff2)" \
  "$FONT_LTR_KB"

if [ "$fail" -ne 0 ]; then
  echo >&2
  echo "  A budget was exceeded. Either the addition earns its bytes and the budget moves" >&2
  echo "  deliberately, or it does not and something has to come out." >&2
fi
exit "$fail"
