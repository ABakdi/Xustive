#!/usr/bin/env bash
#
# The egress test.
#
# `xustive-api` and `xustive-ml` must have no route to the public internet. That is what makes
# "user queries never leave the country" a property of the network rather than a promise about
# the code: even a bug, a malicious dependency, or a misconfigured client cannot exfiltrate a
# query from a container that cannot reach anything.
#
# **This test passes only if the connection FAILS.** A success is a failure.
#
# Only crawler containers are permitted egress, and they never see user queries.

set -uo pipefail

cd "$(dirname "$0")/.."

IMAGE="${EGRESS_TEST_IMAGE:-alpine:3.20}"
fail=0

ok()  { printf '  \033[32m✓\033[0m %s\n' "$1"; }
bad() { printf '  \033[31m✗\033[0m %s\n' "$1"; fail=1; }

# The `core` network is where the API and the backends live.
NET_NAME="${COMPOSE_PROJECT_NAME:-xustive}_core"

if ! docker network inspect "$NET_NAME" >/dev/null 2>&1; then
  echo "  ~ skipping: network $NET_NAME not present (run 'make dev-up' first)" >&2
  exit 0
fi

echo "egress test against network $NET_NAME"

# 1. The network must be marked internal.
internal=$(docker network inspect "$NET_NAME" --format '{{.Internal}}' 2>/dev/null)
if [ "$internal" = "true" ]; then
  ok "network is marked internal"
else
  bad "network is NOT internal — containers can reach the internet"
fi

# 2. Prove it: a container on this network must not complete an outbound TCP connection.
#    A 5s timeout is generous; a working route resolves in well under a second.
docker run --rm --network "$NET_NAME" "$IMAGE" \
  timeout 5 wget -q -O /dev/null http://example.com >/dev/null 2>&1
rc=$?
if [ "$rc" -ne 0 ]; then
  ok "outbound HTTP from the network failed as required (exit $rc)"
else
  bad "OUTBOUND HTTP SUCCEEDED — the egress guarantee is broken"
fi

# 3. DNS resolution of a public name must also fail.
docker run --rm --network "$NET_NAME" "$IMAGE" \
  timeout 5 nslookup example.com >/dev/null 2>&1
rc=$?
if [ "$rc" -ne 0 ]; then
  ok "public DNS resolution failed as required"
else
  bad "PUBLIC DNS RESOLVED — the network has a route out"
fi

# 4. Sanity check the test itself: the same probe on a normal bridge network must succeed.
#    Without this, a broken image or a typo'd command would make the test pass vacuously.
if docker run --rm "$IMAGE" timeout 10 wget -q -O /dev/null http://example.com 2>/dev/null; then
  ok "control: the same probe succeeds on a non-internal network"
else
  bad "control probe failed — this test cannot distinguish 'blocked' from 'broken'"
fi

# 5. The Federation Gateway bridge must not become a back door (M7-T04.6, ADR-0017).
#    The federator is dual-homed: `core` (serving side) and `ingest` (egress side). That is safe
#    only because Docker does not route between a container's interfaces — the API can speak HTTP to
#    the gateway's `/federate`, but gets no IP path to the internet, or to SearXNG, through it.
#    Checks 2 and 3 above already prove the internet stays unreachable from `core` even with the
#    gateway attached. This proves the other half: from `core`, SearXNG itself is unreachable, so the
#    serving plane cannot bypass the gateway and query it directly. Only the federator, on both
#    networks, may reach it. Skipped unless the federation profile is running.
if docker ps --format '{{.Names}}' | grep -q '^xustive-searxng$'; then
  docker run --rm --network "$NET_NAME" "$IMAGE" \
    timeout 5 wget -q -O /dev/null http://xustive-searxng:8080 >/dev/null 2>&1
  rc=$?
  if [ "$rc" -ne 0 ]; then
    ok "SearXNG is unreachable from core — only the dual-homed gateway can reach it"
  else
    bad "SEARXNG REACHABLE FROM CORE — the serving plane can bypass the gateway"
  fi
else
  echo "  ~ federation profile not running; skipping the gateway-bridge check" >&2
fi

# 6. Probe from inside a REAL serving-plane container, not only a fresh one (BUG-009).
#    Checks 2-3 attach a throwaway container to `core` alone — but the dev overlay multi-homes the
#    real containers onto the `devhost` ingress bridge, and a NAT-ing bridge would grant them
#    egress the fresh-container probe can never see. Redis is the probe host: it is always running,
#    alpine-based (busybox wget), and joined to `devhost` in dev.
if docker ps --format '{{.Names}}' | grep -q '^xustive-redis$'; then
  docker exec xustive-redis timeout 5 wget -q -O /dev/null http://example.com >/dev/null 2>&1
  rc=$?
  if [ "$rc" -ne 0 ]; then
    ok "real container (redis, all its networks) cannot reach the internet"
  else
    bad "A REAL CONTAINER REACHED THE INTERNET — a joined network (devhost?) is NAT-ing egress"
  fi
else
  echo "  ~ xustive-redis not running; skipping the real-container probe" >&2
fi

if [ "$fail" -eq 0 ]; then
  echo "✓ egress test: the serving plane cannot reach the internet"
fi
exit "$fail"
