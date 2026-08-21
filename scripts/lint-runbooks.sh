#!/usr/bin/env bash
#
# Enforce the exit-gate rule: every configured alert has a runbook (M4-T09.5).
#
# An alert nobody can act on is noise that trains people to ignore the dashboard, so the rule is
# mechanical rather than aspirational: every `- alert: X` in deploy/prometheus/alerts.yml must have a
# matching `## X` section in docs/Operations/Runbooks.md. A new alert added without a runbook fails
# this check — the same posture as the compose and telemetry lints.
#
# The reverse is also checked: a runbook section for an alert that no longer exists is stale and
# should be removed, so the two files cannot drift apart in either direction.

set -uo pipefail
cd "$(dirname "$0")/.."

ALERTS="deploy/prometheus/alerts.yml"
RUNBOOKS="docs/Operations/Runbooks.md"
fail=0

for f in "$ALERTS" "$RUNBOOKS"; do
  [ -f "$f" ] || { echo "✗ runbook lint: $f not found" >&2; exit 1; }
done

# Alert names: the value after `- alert:`.
alerts="$(grep -hE '^\s*- alert:' "$ALERTS" | sed -E 's/.*- alert:[[:space:]]*//' | sort -u)"
# Runbook sections: alert sections are `## Name  · <severity>`. Keying on the `·` severity marker
# means non-alert headings (## Related, ## Scope) are ignored without a maintained exclusion list.
sections="$(grep -hE '^## .+·' "$RUNBOOKS" | sed -E 's/^## //; s/[[:space:]].*//' | sort -u)"

# Every alert must have a runbook section.
while IFS= read -r a; do
  [ -z "$a" ] && continue
  if ! printf '%s\n' "$sections" | grep -qx "$a"; then
    echo "✗ runbook lint: alert '$a' has no '## $a' section in $RUNBOOKS" >&2
    fail=1
  fi
done <<< "$alerts"

# Every runbook section must correspond to a real alert (no stale runbooks).
while IFS= read -r s; do
  [ -z "$s" ] && continue
  if ! printf '%s\n' "$alerts" | grep -qx "$s"; then
    echo "✗ runbook lint: runbook section '$s' has no matching alert in $ALERTS" >&2
    fail=1
  fi
done <<< "$sections"

if [ "$fail" -eq 0 ]; then
  n="$(printf '%s\n' "$alerts" | grep -c .)"
  echo "✓ runbook lint: all $n configured alerts have a runbook"
fi
exit "$fail"
