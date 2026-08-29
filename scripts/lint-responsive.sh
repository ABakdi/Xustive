#!/usr/bin/env bash
# The responsive gate ([[UI - Responsive]] §4). Static, because the project has no headless
# browser; the measuring is done by the harness at /responsive. Each rule here is a mistake that
# was actually found in the 2026-08-29 audit, not a style preference.
set -euo pipefail
cd "$(dirname "$0")/.."
fail=0

say() { printf '✗ responsive lint: %s\n' "$1"; fail=1; }

# 1. A fixed minimum width of 200px or more, with no breakpoint in front of it, overflows a phone.
if hits=$(grep -rnE "minWidth: *(2[0-9]{2}|[3-9][0-9]{2}|[0-9]{4,})" web/app web/components 2>/dev/null); then
  say "inline minWidth ≥ 200px (use w-full sm:w-auto sm:min-w-[…])"; echo "$hits" | head -5
fi
if hits=$(grep -rnP "(?<!sm:)(?<!md:)(?<!lg:)min-w-\[(2\d\d|[3-9]\d\d|\d{4,})px\]" web/app web/components 2>/dev/null); then
  say "min-w-[≥200px] without a breakpoint"; echo "$hits" | head -5
fi

# 2. `vh` does not survive a mobile URL bar.
if hits=$(grep -rnE "[^d]100vh|: *100vh" web/app web/components 2>/dev/null | grep -v dvh); then
  say "100vh (use 100dvh)"; echo "$hits" | head -5
fi

# 3. A table is wider than a phone by nature, so it must sit in a scroll container — the shared
#    `Table` component provides one; a hand-rolled table needs `<div className="scroll-x">`.
hits=$(awk '
  /<table/ { if (prev !~ /scroll-x|overflow-x-auto/) printf "%s:%d:%s\n", FILENAME, FNR, $0 }
  { if ($0 ~ /[^ \t]/) prev = $0 }
' $(grep -rl "<table" web/app web/components 2>/dev/null | grep -v "components/admin/ui.tsx") 2>/dev/null || true)
if [ -n "$hits" ]; then
  say "<table> not inside a scroll container (wrap it in <div className=\"scroll-x\">)"; echo "$hits" | head -5
fi

[ "$fail" = 0 ] && echo "✓ responsive lint: no fixed widths, no 100vh, tables wrapped"
exit "$fail"
