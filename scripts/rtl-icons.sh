#!/usr/bin/env bash
# Directional icons must be mirrored under RTL (M1-T14.5).
#
# A glyph that points — arrow, chevron, reply hook — means "forward", and in Arabic forward is
# leftward. Logical CSS moves boxes but cannot flip the inside of an SVG, so a directional icon has
# to carry `rtl-flip` (see the rule in globals.css). This check fails if one is imported from
# lucide without that class appearing in the same file.
#
# Why a check rather than trust: the codebase deliberately has *no* directional icons today —
# pagination uses the words "previous"/"next", not chevrons. That is easy to erode. The first
# "next →" someone adds works perfectly in a French browser and points the wrong way in an Arabic
# one, and nothing on screen reveals it to a developer who does not read Arabic. This makes the
# regression a red build instead of a shipped bug.
#
#   ./scripts/rtl-icons.sh
set -euo pipefail

cd "$(dirname "$0")/.."
root="web/components web/app"

# lucide icons whose glyph points in a direction. Not exhaustive of lucide, but covers the ones a
# search UI reaches for. Add to it rather than removing the check.
directional='ArrowLeft|ArrowRight|ArrowUpLeft|ArrowUpRight|ArrowDownLeft|ArrowDownRight|ArrowBigLeft|ArrowBigRight|ChevronLeft|ChevronRight|ChevronsLeft|ChevronsRight|ChevronFirst|ChevronLast|CornerDownLeft|CornerDownRight|CornerUpLeft|CornerUpRight|Reply|Forward|Undo|Redo|SkipBack|SkipForward|MoveLeft|MoveRight|PanelLeft|PanelRight'

status=0
# Every .tsx that imports a directional icon from lucide-react.
while IFS= read -r file; do
  [ -z "$file" ] && continue
  # Which directional icons does this file import?
  icons=$(grep -oE "\b($directional)\b" "$file" | sort -u | tr '\n' ' ' || true)
  if [ -n "$icons" ] && ! grep -q 'rtl-flip' "$file"; then
    echo "✗ $file uses directional icon(s) [$icons] without the rtl-flip class"
    echo "    Add className=\"rtl-flip\" so it mirrors in Arabic. See globals.css."
    status=1
  fi
done < <(grep -rlE "from 'lucide-react'" $root 2>/dev/null || true)

if [ "$status" -eq 0 ]; then
  echo "✓ rtl icons: no directional icon ships without mirroring"
fi
exit "$status"
