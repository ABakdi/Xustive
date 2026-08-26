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

# The settings page and the per-tool opt-out must work without JavaScript too.
#
# Both are Server Actions posting from real forms, which is the only reason they can. A click
# handler would have been easier to write and would have made the one control for switching a
# tool off the single part of the page that needed script to use.
settings=$(curl -fsS --max-time 5 "$BASE/$LANG_SEG/settings" || true)
if [ -z "$settings" ]; then
  echo "  settings page did not render" >&2
  fail=1
else
  # A real form with a submit button per tool, and an action to post to.
  if ! printf '%s' "$settings" | grep -q '<form'; then
    echo "  settings has no <form> — the toggles cannot work without JavaScript" >&2
    fail=1
  fi
  if ! printf '%s' "$settings" | grep -q 'type="submit"'; then
    echo "  settings has no submit button — the toggles are script-only" >&2
    fail=1
  fi
  if [ "$fail" -eq 0 ]; then
    toggles=$(printf '%s' "$settings" | grep -o 'type="submit"' | wc -l)
    echo "  ✓ settings toggles are real form submits ($toggles)"
  fi
fi

# The Images tab is a server-rendered grid (M9-T05.1): tiles must be in the first response, and
# every one of them must go through the signed proxy — a raw crawled-host <img src> would hand
# the reader's address to that host on page load.
images=$(page --data-urlencode "v=images")
tiles=$(printf '%s' "$images" | grep -o '/api/thumb?u=' | wc -l | tr -d ' ')
raw=$(printf '%s' "$images" | grep -oE '<img[^>]*src="https?://[^"]*' | grep -vc '/api/thumb' || true)
if [ "$tiles" -gt 0 ] && [ "${raw:-0}" -eq 0 ]; then
  echo "  ✓ the images tab renders $tiles server-side tiles, all through the signed proxy"
elif [ "$tiles" -eq 0 ]; then
  bad "the images tab rendered no tiles without script"
else
  bad "the images tab hotlinks $raw image(s) from crawled hosts"
fi

# And the dismiss control on a tool card.
card=$(curl -fsS --max-time 5 --get "$BASE/$LANG_SEG/search" --data-urlencode "q=roman 2026" || true)
if printf '%s' "$card" | grep -q 'MMXXVI'; then
  if printf '%s' "$card" | grep -q '<form'; then
    echo "  ✓ the tool card renders and its dismiss control is a real form"
  else
    echo "  tool card has no <form> — dismissing a tool needs JavaScript" >&2
    fail=1
  fi
else
  # Not a silent pass. If the card stops rendering entirely this check would otherwise report
  # nothing at all, which reads identically to success.
  echo "  ! tool card did not render — dismiss control not exercised" >&2
  fail=1
fi

# The shared UI primitives must stay server components.
#
# This is the property that makes them worth having rather than installing shadcn/ui: its
# primitives are Radix components, Radix components are client components, and adopting them would
# push `'use client'` into the result page. A results list that ships as markup is the single most
# valuable thing about this frontend, and it is one careless import away from not being true.
# Anchored to the start of a line, because the directive is only a directive there. A loose match
# flagged Button.tsx, whose doc comment explains why Radix's client components are not used —
# prose about 'use client' is not 'use client'.
if grep -rlE "^['\"]use client['\"]" web/components/ui 2>/dev/null | grep -q .; then
  echo "  a UI primitive declares 'use client' — that pulls a runtime onto every page using it" >&2
  fail=1
elif grep -rlE "@radix-ui|from 'cva'|class-variance-authority" web/components/ui 2>/dev/null | grep -q .; then
  echo "  a UI primitive imports Radix or cva — see components/ui/Button.tsx for why not" >&2
  fail=1
else
  primitives=$(find web/components/ui -name '*.tsx' 2>/dev/null | wc -l)
  # Not a silent pass. Finding no primitives at all would otherwise report success, which reads
  # identically to having checked them.
  if [ "$primitives" -eq 0 ]; then
    echo "  ! no UI primitives found — is the path right?" >&2
    fail=1
  else
    echo "  ✓ all $primitives UI primitives are server components"
  fi
fi

if [ "$fail" -ne 0 ]; then
  echo >&2
  echo "  The no-JavaScript path is a commitment in UI - Frontend Architecture §3, and it is" >&2
  echo "  the path that breaks silently: everything works in a browser with script." >&2
fi
exit "$fail"
