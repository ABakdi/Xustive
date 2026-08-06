#!/usr/bin/env bash
#
# Catch instructional documentation that tells people to run commands which do not exist.
#
# This exists because `make setup` was documented, was never implemented, and the first person to
# follow the instructions hit "No rule to make target". A warning banner on the note was not
# enough: a command in a code block reads as an instruction no matter what the prose around it
# says.
#
# Scope is deliberately narrow, on two axes.
#
# **Which files.** Only docs someone would follow along with. Planning notes and component specs
# describe commands that will exist later — that is what a plan is for, and linting them would
# either produce noise or discourage writing the plan down.
#
# **Which text.** Only `make x` inside backticks or at the start of a line in a code block.
# Prose like "make it clear" is not an instruction.
#
# Escape hatch: mark the line ❌ (or "not built") and it is accepted as an honest statement of
# future state.

set -uo pipefail
cd "$(dirname "$0")/.."

# Files a reader would type commands out of.
INSTRUCTIONAL=(
  "README.md"
  "docs/Engineering/Running the System.md"
  "docs/Engineering/Local Development.md"
)

fail=0
existing=$(grep -oE '^[a-z][a-z-]*:' Makefile | tr -d ':' | sort -u)

# A line is acceptable if it marks the command as unbuilt, or explicitly says it does not
# exist. Telling a reader "there is no `make run-web`" is the most useful thing a doc can say
# about a command people keep reaching for, so it must not be flagged.
is_marked() {
  case "$1" in
    *❌*|*"NOT BUILT"*|*"not built"*|*"arrive"*) return 0 ;;
    *"do not exist"*|*"does not exist"*|*"there is no"*|*"There is no"*|*"no \`make"*) return 0 ;;
    *) return 1 ;;
  esac
}

unknown=""
for file in "${INSTRUCTIONAL[@]}"; do
  [ -f "$file" ] || continue
  lineno=0
  while IFS= read -r line; do
    lineno=$((lineno + 1))

    # `make x` in backticks, or a command line in a code block.
    targets=$(printf '%s' "$line" \
      | grep -oE '`make [a-z][a-z-]*`|^[[:space:]]*make [a-z][a-z-]*' \
      | sed -E 's/`//g; s/^[[:space:]]*//; s/^make //' || true)
    [ -z "$targets" ] && continue

    is_marked "$line" && continue

    while IFS= read -r target; do
      [ -z "$target" ] && continue
      printf '%s\n' "$existing" | grep -qx "$target" && continue
      unknown="${unknown}    ${file}:${lineno}  make ${target}\n"
    done <<< "$targets"
  done < "$file"
done

if [ -n "$unknown" ]; then
  echo "✗ docs lint: instructions reference make targets that do not exist" >&2
  printf "%b" "$unknown" >&2
  echo "  Implement the target, or mark the line ❌ so readers know it is not built yet." >&2
  fail=1
fi

# Scripts referenced anywhere in the docs must exist — those are never aspirational.
while IFS= read -r hit; do
  [ -z "$hit" ] && continue
  file="${hit%%:*}"
  script=$(printf '%s' "$hit" | grep -oE 'scripts/[a-z_-]+\.(sh|py)' | head -1)
  [ -z "$script" ] && continue
  if [ ! -f "$script" ]; then
    echo "✗ docs lint: $file references missing $script" >&2
    fail=1
  fi
done <<< "$(grep -rnoE 'scripts/[a-z_-]+\.(sh|py)' README.md docs/ --include='*.md' 2>/dev/null || true)"

if [ "$fail" -eq 0 ]; then
  echo "✓ docs lint: every documented command exists or is marked as not built"
fi
exit "$fail"
