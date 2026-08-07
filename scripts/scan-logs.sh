#!/usr/bin/env bash
# Scan logs for query text that should never have been written.
#
# The lint in `lint-telemetry.sh` checks the *source* for identifiers passed to tracing macros.
# This checks the other end: whether anything that looks like a user's query actually reached a
# log line. The two catch different failures — the lint cannot see a query that arrives inside a
# struct someone made `Debug`, and this cannot see a leak on a code path that never ran.
#
# Intended to run nightly against the day's logs:
#     ./scripts/scan-logs.sh /var/log/xustive/*.log
# In development, against whatever the process wrote:
#     make run-api 2>&1 | tee /tmp/api.log && ./scripts/scan-logs.sh /tmp/api.log
set -euo pipefail

CORPUS="${CORPUS:-tests/fixtures/corpus/queries.txt}"
fail=0
files=("$@")

if [ ${#files[@]} -eq 0 ]; then
  echo "usage: $0 <logfile> [logfile...]" >&2
  exit 2
fi

# --- structural checks ---------------------------------------------------------------------
#
# These need no corpus. A log line carrying any of these field names is a leak regardless of what
# the value happens to be.
BANNED_FIELDS='(^|[ ,{"])(q|query|raw_query|normalized|transcript|ocr_text|summary_text)[=:]'

for f in "${files[@]}"; do
  [ -f "$f" ] || { echo "skipping missing $f" >&2; continue; }

  if hits=$(grep -nE "$BANNED_FIELDS" "$f" | head -20) && [ -n "$hits" ]; then
    echo "✗ $f: log lines carry a query-bearing field" >&2
    echo "$hits" | sed 's/^/    /' >&2
    fail=1
  fi

  # A URL with a query string in a log line is the same leak wearing a different hat.
  if hits=$(grep -nE '[?&](q|query)=[^ "]+' "$f" | head -20) && [ -n "$hits" ]; then
    echo "✗ $f: log lines contain a URL with a query parameter" >&2
    echo "$hits" | sed 's/^/    /' >&2
    fail=1
  fi
done

# --- corpus check --------------------------------------------------------------------------
#
# Replay known query strings against the logs. This is the check that catches a leak through a
# path the structural rules do not name — a Debug impl, a formatted error, a panic message.
if [ -f "$CORPUS" ]; then
  while IFS= read -r q; do
    # Short queries produce false positives against ordinary words; the leak we care about is a
    # recognisable phrase, not the word "the".
    [ ${#q} -ge 8 ] || continue
    for f in "${files[@]}"; do
      [ -f "$f" ] || continue
      if grep -qF -- "$q" "$f"; then
        echo "✗ $f: contains the query $(printf '%q' "$q")" >&2
        fail=1
      fi
    done
  done < "$CORPUS"
else
  echo "  note: no query corpus at $CORPUS; ran structural checks only"
fi

if [ "$fail" -eq 0 ]; then
  echo "✓ log scan: no query text found in ${#files[@]} file(s)"
fi
exit "$fail"
