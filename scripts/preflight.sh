#!/usr/bin/env bash
# Everything that must be true before `make deploy` on a fresh VPS, checked in the order that
# fails fastest. Each check names the fix; none of them mutate anything.
set -uo pipefail
cd "$(dirname "$0")/.."
fail=0; warn=0
ok()   { printf '  \033[32m✓\033[0m %s\n' "$1"; }
bad()  { printf '  \033[31m✗\033[0m %s\n' "$1"; fail=1; }
soft() { printf '  \033[33m!\033[0m %s\n' "$1"; warn=1; }

echo "Preflight — deploying Xustive to this host"
echo
echo "Host"
cores=$(nproc 2>/dev/null || echo 0)
memgb=$(( $(grep MemTotal /proc/meminfo 2>/dev/null | awk '{print $2}' || echo 0) / 1024 / 1024 ))
diskgb=$(df -BG --output=avail . 2>/dev/null | tail -1 | tr -dc '0-9')
[ "$cores" -ge 4 ] && ok "$cores vCPU" || soft "$cores vCPU — 4 is the practical floor (crawl + index + serve)"
[ "$memgb" -ge 15 ] && ok "${memgb} GB RAM" || soft "${memgb} GB RAM — Meilisearch alone wants 16 GB once the index grows (PROB-004)"
[ "${diskgb:-0}" -ge 80 ] && ok "${diskgb} GB free" || soft "${diskgb:-?} GB free — the index outgrows a small disk quickly"
command -v docker >/dev/null && ok "docker present" || bad "docker missing — install Docker Engine + the compose plugin"
docker compose version >/dev/null 2>&1 && ok "docker compose present" || bad "the compose plugin is missing"
docker info >/dev/null 2>&1 && ok "docker daemon reachable" || bad "cannot talk to the docker daemon (permissions? add yourself to the docker group)"

echo
echo "Configuration"
if [ -f .env ]; then ok ".env exists"; set -a; . ./.env 2>/dev/null; set +a
else bad ".env missing — cp .env.example .env and fill it in"; fi
need() {
  local name="$1" hint="$2" value="${!1:-}"
  if [ -z "$value" ]; then bad "$name is empty — $hint"; else ok "$name is set"; fi
}
need XUSTIVE_DOMAIN "the hostname this box answers on"
need XUSTIVE_ACME_EMAIL "where the certificate authority sends expiry warnings"
need XUSTIVE_ADMIN_KEY "openssl rand -base64 32 — the API refuses to start on a public address without it"
need XUSTIVE_ADMIN_PASSWORD_HASH "make admin-password"
need MEILI_MASTER_KEY "openssl rand -base64 32 — without it Meilisearch has no authentication at all"
need XUSTIVE_GRAFANA_PASSWORD "the dashboards' own login, at /grafana"
if [ "${XUSTIVE_GRAFANA_PASSWORD:-}" = "admin" ]; then bad "XUSTIVE_GRAFANA_PASSWORD is still 'admin'"; fi
if [ -n "${XUSTIVE_ADMIN_KEY:-}" ] && [ "${#XUSTIVE_ADMIN_KEY}" -lt 16 ]; then
  bad "XUSTIVE_ADMIN_KEY is shorter than 16 characters; the API will refuse it"
fi
[ -f config/prod.toml ] && ok "config/prod.toml present" || bad "config/prod.toml missing"

echo
echo "Network"
if [ -n "${XUSTIVE_DOMAIN:-}" ]; then
  resolved=$(getent hosts "$XUSTIVE_DOMAIN" 2>/dev/null | awk '{print $1}' | head -1)
  public=$(curl -fsS --max-time 5 https://api.ipify.org 2>/dev/null || echo "")
  if [ -z "$resolved" ]; then bad "$XUSTIVE_DOMAIN does not resolve — the certificate request will fail"
  elif [ -n "$public" ] && [ "$resolved" != "$public" ]; then
    soft "$XUSTIVE_DOMAIN resolves to $resolved but this host looks like $public"
  else ok "$XUSTIVE_DOMAIN resolves to $resolved"; fi
fi
for p in 80 443; do
  if ss -lnt 2>/dev/null | awk '{print $4}' | grep -qE "[:.]$p\$"; then
    bad "port $p is already in use — Caddy cannot bind it"
  else ok "port $p is free"; fi
done

echo
if [ "$fail" != 0 ]; then echo "Not ready. Fix the ✗ lines above, then run make preflight again."; exit 1; fi
if [ "$warn" != 0 ]; then echo "Ready, with warnings (!). Small hosts work; they just fill up sooner."; else echo "Ready."; fi
