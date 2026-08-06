#!/usr/bin/env bash
#
# The privacy lint.
#
# Fails the build if a `tracing::` call site names a field that would carry user query content or
# a collection credential. This is the mechanism that turns "we don't log queries" from a policy
# into something CI enforces.
#
# It is deliberately dumb and slightly over-eager: a false positive costs one rename, a false
# negative costs the product's central claim.

set -euo pipefail

cd "$(dirname "$0")/.."

# Field names that must never be attached to a span or event.
FORBIDDEN='(^|[^a-z_])(q|query|raw_query|normalized_query|transcript|ocr_text|user_query|search_term|password|passwd|cookie|cookies|credentials|totp|secret|api_key|token)[[:space:]]*='

fail=0

# Look only at tracing macro invocations, including the multi-line form.
matches=$(
  grep -rnE --include='*.rs' \
    'tracing::(trace|debug|info|warn|error)!|^[[:space:]]*(trace|debug|info|warn|error)!\(' \
    crates/ 2>/dev/null \
  | cut -d: -f1,2 \
  | while IFS=: read -r file line; do
      # Inspect the macro call and the four following lines, which covers the wrapped style.
      snippet=$(sed -n "${line},$((line + 4))p" "$file")
      if echo "$snippet" | grep -qE "$FORBIDDEN"; then
        echo "$file:$line"
      fi
    done
)

if [ -n "$matches" ]; then
  echo "✗ telemetry lint: query or credential fields in tracing call sites" >&2
  echo "$matches" | sed 's/^/    /' >&2
  echo >&2
  echo "  User queries must never reach a log, metric label, or span attribute." >&2
  echo "  Record a length bucket or a language instead." >&2
  fail=1
fi

# The metrics registry only accepts &'static str label *names*, so a dynamic query string cannot
# become a label key. Guard against that being loosened to &str.
#
# Checked structurally rather than by inspecting each signature, because the signatures wrap.
METRICS=crates/xustive-api/src/metrics.rs
expected_sig="labels: &[(&'static str, &str)]"
recording_fns=$(grep -cE '^\s*pub fn (incr|incr_by|observe)\(' "$METRICS" || true)
typed_labels=$(grep -cF "$expected_sig" "$METRICS" || true)

if [ "$recording_fns" -eq 0 ]; then
  echo "✗ telemetry lint: cannot find the metric recording functions in $METRICS" >&2
  fail=1
elif [ "$typed_labels" -lt "$recording_fns" ]; then
  echo "✗ telemetry lint: metric label names must stay &'static str" >&2
  echo "    found $recording_fns recording fn(s) but only $typed_labels typed label param(s)" >&2
  echo "    a &str label key would let a query string become a metric label" >&2
  fail=1
fi

if [ "$fail" -eq 0 ]; then
  echo "✓ telemetry lint: no query or credential fields in telemetry"
fi

exit "$fail"
