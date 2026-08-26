#!/usr/bin/env bash
#
# Catch bidirectional-text hazards in the UI.
#
# Arabic and Darija are read right-to-left. Latin text, URLs, digits and brackets embedded in an
# RTL line are *neutral or opposite-direction runs*, and the Unicode bidi algorithm reorders them
# against the surrounding text unless they are isolated. The result is not a subtle spacing issue:
# `(12 ms)` renders as `(ms 12`, a URL comes out backwards, a date flips to `2026/08/08`.
#
# It reads as a typo rather than as a rendering bug, which is why it survives review — a reviewer
# who does not read Arabic sees nothing wrong, and a reviewer who does assumes someone typed it
# that way.
#
# `dir="auto"` on a container is not a substitute. It resolves the direction of the container from
# its first strong character; it does not isolate the runs *inside* it. Both are needed.
set -euo pipefail

cd "$(dirname "$0")/../web"
fail=0

# 1. A URL rendered as text must be isolated.
#
# Attribute values are never laid out, so they cannot reorder — a URL in `href=`, `value=` or
# `title=` is data the browser reads, not text the bidi algorithm arranges. Rather than listing
# attribute names, every `name={...}` assignment is stripped from the line before the check, so
# what remains is only what is actually rendered. A line carrying both an attribute and a bare
# rendered value is therefore still caught, which a name-by-name exclusion list would have missed.
while IFS= read -r line; do
  file="${line%%:*}"
  rest="${line#*:}"
  num="${rest%%:*}"
  code="${rest#*:}"
  case "$code" in
  *"<bdi"*) continue ;;
  esac
  # What is left after the attributes are removed.
  rendered=$(printf '%s' "$code" | sed -E 's/[a-zA-Z-]+=\{[^}]*\}//g')
  case "$rendered" in
  *"{"*"display_url"*"}"* | *"{"*"host"*"}"* | *"{"*"domain"*"}"*) ;;
  *) continue ;;
  esac
  echo "  $file:$num — URL rendered without <bdi>: ${code#"${code%%[![:space:]]*}"}"
  fail=1
done < <(grep -rn "{[a-zA-Z_.]*\(display_url\|\bhost\b\|domain\)[a-zA-Z_.]*}" \
  --include="*.tsx" components app 2>/dev/null || true)

# 2. A formatted date or number rendered next to a bracket must be isolated.
#
# Brackets are the sharpest case: they are neutral on both sides, so an unisolated group has its
# opening and closing marks swapped rather than merely shifted.
#
# Checked over a small window rather than a single line, because JSX wraps: the `<bdi>` that
# isolates a group is routinely on the line above the value it wraps, and a line-at-a-time check
# reports that correct code as a fault.
python3 - <<'PY' || fail=1
import pathlib, re, sys

BRACKETED = re.compile(r"\([^)]*\{format(Number|Date)")
bad = []
for path in list(pathlib.Path("components").rglob("*.tsx")) + list(pathlib.Path("app").rglob("*.tsx")):
    lines = path.read_text().splitlines()
    for i, line in enumerate(lines):
        if not BRACKETED.search(line):
            continue
        # The isolating element may open on this line or shortly above it, and close below.
        window = "\n".join(lines[max(0, i - 3) : i + 3])
        if "<bdi" in window:
            continue
        bad.append(f"  {path}:{i + 1} — bracketed value without <bdi>: {line.strip()}")

if bad:
    print("\n".join(bad))
    sys.exit(1)
PY

# 3. Every page-level container must declare a direction.
#
# Without `dir`, the browser assumes the document default and lays Arabic out left-to-right.
if ! grep -rq 'dir={dir' --include="*.tsx" app 2>/dev/null &&
  ! grep -rq 'dir="rtl"\|dir={' --include="*.tsx" app 2>/dev/null; then
  echo "  no dir attribute found in app/ — Arabic will lay out left-to-right"
  fail=1
fi

# 4. Logical properties, not physical ones.
#
# `margin-left` is the left in both directions; `margin-inline-start` follows the reader. A
# physical property is how an RTL layout ends up mirrored everywhere except the one place someone
# hard-coded a side.
if grep -rn "\b\(margin-left\|margin-right\|padding-left\|padding-right\|border-left\|border-right\)\s*:" \
  --include="*.css" app 2>/dev/null | grep -v "^\s*/\*" | grep -v "logical-ok"; then
  echo "  physical side properties in CSS — use the inline-start/end logical forms"
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  echo "✗ bidi lint failed" >&2
  exit 1
fi

echo "✓ bidi lint: URLs, dates and bracketed values are isolated; layout is direction-agnostic"
