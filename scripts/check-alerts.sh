#!/usr/bin/env bash
#
# Run the Prometheus alert-rule unit tests.
#
# A rule file that parses is not a rule file that fires. `promtool test rules` replays synthetic
# series through the real rules and asserts which alerts appear — so a threshold typo, a `for:`
# that never elapses, or a label that does not propagate is caught here rather than during the
# incident the alert was written for.
#
# promtool ships inside the Prometheus image, so no host install is needed.
set -euo pipefail

cd "$(dirname "$0")/.."
RULES_DIR="deploy/prometheus"
IMAGE="prom/prometheus:latest"

if command -v promtool >/dev/null 2>&1; then
  run() { promtool "$@"; }
elif command -v docker >/dev/null 2>&1; then
  # Read-only mount: a linter has no business writing to the tree it is checking.
  run() {
    docker run --rm -v "$PWD/$RULES_DIR:/rules:ro" --entrypoint promtool "$IMAGE" \
      "${@/#$RULES_DIR\//\/rules\/}"
  }
else
  echo "neither promtool nor docker is available; skipping alert checks" >&2
  exit 0
fi

run check rules "$RULES_DIR/alerts.yml" >/dev/null
run test rules "$RULES_DIR/alerts_test.yml" >/dev/null

echo "✓ alert lint: rules parse and every alert fires when it should"
