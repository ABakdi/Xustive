#!/usr/bin/env bash
#
# End-to-end smoke test against a running xustive-api.
#
# Covers the M0 exit gate: a known query returns known documents, filters narrow correctly,
# errors match the contract, and the privacy headers are present.

set -uo pipefail

BASE="${BASE:-http://localhost:8080}"
pass=0
fail=0

ok()   { printf '  \033[32m✓\033[0m %s\n' "$1"; pass=$((pass + 1)); }
bad()  { printf '  \033[31m✗\033[0m %s\n' "$1"; fail=$((fail + 1)); }

# jq is not assumed; python3 is already a dependency of the corpus generator.
jget() { python3 -c "import json,sys;d=json.load(sys.stdin);print(eval('d'+sys.argv[1]))" "$1" 2>/dev/null; }

search() { curl -sS -G "$BASE/api/v1/search" "$@"; }

section() { printf '\n\033[1m%s\033[0m\n' "$1"; }

# ---------------------------------------------------------------------------------------
section "health"

[ "$(curl -sS "$BASE/healthz")" = "ok" ] && ok "healthz" || bad "healthz"
[ "$(curl -sS -o /dev/null -w '%{http_code}' "$BASE/readyz")" = "200" ] \
  && ok "readyz reports ready" || bad "readyz"
curl -sS "$BASE/metrics" | grep -q '^# TYPE xustive_http_requests_total counter' \
  && ok "metrics exposition" || bad "metrics exposition"

# ---------------------------------------------------------------------------------------
section "search across languages"

for probe in "سونلغاز:arabic entity" "وهران:arabic wilaya" \
             "electricite:french" "wach rak:arabizi darija" \
             "facture:french common"; do
  q="${probe%%:*}"; label="${probe##*:}"
  n=$(search --data-urlencode "q=$q" | jget "['pagination']['total_hits']")
  if [ "${n:-0}" -gt 0 ]; then ok "$label ('$q') → $n hits"; else bad "$label ('$q') → no hits"; fi
done

# ---------------------------------------------------------------------------------------
section "normalisation reaches the index"

# Tatweel, harakat and Arabic-Indic digits must not change what matches. This is the
# index-time/query-time symmetry guarantee, observed end to end.
plain=$(search --data-urlencode "q=سونلغاز" | jget "['pagination']['total_hits']")
tatweel=$(search --data-urlencode "q=سونلـــغاز" | jget "['pagination']['total_hits']")
harakat=$(search --data-urlencode "q=سُونلغَاز" | jget "['pagination']['total_hits']")
[ "$plain" = "$tatweel" ] && ok "tatweel ignored ($plain = $tatweel)" \
  || bad "tatweel changed results ($plain vs $tatweel)"
[ "$plain" = "$harakat" ] && ok "harakat ignored ($plain = $harakat)" \
  || bad "harakat changed results ($plain vs $harakat)"

# ---------------------------------------------------------------------------------------
section "filters"

all=$(search --data-urlencode "q=الجزائر" | jget "['pagination']['total_hits']")
fb=$(search --data-urlencode "q=الجزائر" -d "source=facebook" | jget "['pagination']['total_hits']")
web=$(search --data-urlencode "q=الجزائر" -d "source=web" | jget "['pagination']['total_hits']")
[ "${fb:-0}" -lt "${all:-0}" ] && ok "source filter narrows ($all → $fb facebook)" \
  || bad "source filter did not narrow"
[ "${web:-0}" -gt 0 ] && ok "web filter returns results ($web)" || bad "web filter empty"

only_fb=$(search --data-urlencode "q=الجزائر" -d "source=facebook" \
  | python3 -c "import json,sys;d=json.load(sys.stdin);print(all(r['source_type']=='facebook' for r in d['results']) if d['results'] else False)")
[ "$only_fb" = "True" ] && ok "source filter is honoured in results" \
  || bad "source filter leaked other source types"

neg=$(search --data-urlencode "q=الجزائر" -d "sentiment=negative" | jget "['pagination']['total_hits']")
[ "${neg:-0}" -lt "${all:-0}" ] && ok "sentiment filter narrows ($all → $neg negative)" \
  || bad "sentiment filter did not narrow"

# ---------------------------------------------------------------------------------------
section "pagination"

p1=$(search --data-urlencode "q=الجزائر" -d "page=1" -d "hits_per_page=5" | jget "['results'][0]['id']")
p2=$(search --data-urlencode "q=الجزائر" -d "page=2" -d "hits_per_page=5" | jget "['results'][0]['id']")
[ -n "$p1" ] && [ "$p1" != "$p2" ] && ok "page 2 differs from page 1" || bad "pagination not advancing"

cnt=$(search --data-urlencode "q=الجزائر" -d "hits_per_page=5" \
  | python3 -c "import json,sys;print(len(json.load(sys.stdin)['results']))")
[ "$cnt" = "5" ] && ok "hits_per_page honoured" || bad "hits_per_page ignored (got $cnt)"

# Over-large page size is clamped, not rejected.
clamped=$(search --data-urlencode "q=الجزائر" -d "hits_per_page=9999" | jget "['pagination']['hits_per_page']")
[ "$clamped" = "50" ] && ok "hits_per_page clamped to max ($clamped)" \
  || bad "hits_per_page not clamped (got $clamped)"

# ---------------------------------------------------------------------------------------
section "highlighting and shaping"

search --data-urlencode "q=سونلغاز" | grep -q '<em>' \
  && ok "search terms are highlighted" || bad "no highlighting"

# `body` is searchable but must never be served — the copyright posture is enforced by the
# index's displayedAttributes, not by the handler.
search --data-urlencode "q=سونلغاز" | grep -q '"body"' \
  && bad "full body leaked into the response" || ok "full body is not served"

# ---------------------------------------------------------------------------------------
section "error contract"

code_for() { curl -sS -o /dev/null -w '%{http_code}' -G "$BASE/api/v1/search" "$@"; }
body_code() { curl -sS -G "$BASE/api/v1/search" "$@" | jget "['error']['code']"; }

[ "$(code_for)" = "400" ] && ok "missing q → 400" || bad "missing q"
[ "$(body_code)" = "invalid_query" ] && ok "missing q → invalid_query" || bad "missing q code"

long=$(python3 -c "print('a'*600)")
[ "$(code_for --data-urlencode "q=$long")" = "400" ] && ok "long query → 400" || bad "long query"
[ "$(body_code --data-urlencode "q=$long")" = "query_too_long" ] \
  && ok "long query → query_too_long" || bad "long query code"

[ "$(code_for --data-urlencode "q=x" -d "source=myspace")" = "400" ] \
  && ok "unknown source → 400" || bad "unknown source"
[ "$(body_code --data-urlencode "q=x" -d "source=myspace")" = "invalid_filter" ] \
  && ok "unknown source → invalid_filter" || bad "unknown source code"

[ "$(code_for --data-urlencode "q=x" -d "sort=banana")" = "400" ] \
  && ok "bad sort → 400" || bad "bad sort"

# ---------------------------------------------------------------------------------------
section "filter injection"

# Closing the string and appending a clause must not widen the result set.
base_n=$(search --data-urlencode "q=الجزائر" -d "source=web" | jget "['pagination']['total_hits']")
inj=$(code_for --data-urlencode "q=الجزائر" --data-urlencode 'source=web" OR spam_score > 0 OR domain = "')
[ "$inj" = "400" ] && ok "filter injection rejected as unknown source value" \
  || bad "filter injection returned $inj"

# ---------------------------------------------------------------------------------------
section "server-rendered page (JavaScript disabled)"

ssr=$(curl -sS "$BASE/search?q=%D8%B3%D9%88%D9%86%D9%84%D8%BA%D8%A7%D8%B2")
cards=$(printf '%s' "$ssr" | grep -c 'class="result-card"')
[ "$cards" -ge 10 ] && ok "SSR renders $cards result cards without JS" \
  || bad "SSR rendered only $cards cards"
printf '%s' "$ssr" | grep -q '<em>' && ok "SSR highlighting" || bad "SSR highlighting missing"
printf '%s' "$ssr" | grep -q 'dir="auto"' && ok "SSR per-string dir=auto" || bad "dir=auto missing"
printf '%s' "$ssr" | grep -q 'class="pagination"' && ok "SSR pagination" || bad "SSR pagination missing"
printf '%s' "$ssr" | grep -q 'skip-link' && ok "SSR skip link" || bad "skip link missing"
[ "$(curl -sS -o /dev/null -w '%{http_code}' "$BASE/search")" = "303" ] \
  && ok "SSR /search with no query redirects home" || bad "SSR empty query handling"

# ---------------------------------------------------------------------------------------
section "XSS: crawled markup must render as text"

# Index a document whose title and excerpt are hostile, then confirm both the JSON API and the
# server-rendered page neutralise them. This is the boundary that matters: the corpus is the
# open web, so this content is not hypothetical.
XSS_ID="01JXSSPROBE0000000000000AA"
payload=$(python3 - <<'PY'
import json
print(json.dumps([{
  "id": "01JXSSPROBE0000000000000AA",
  "content_hash": "b3:xsstest",
  "url": "https://xss-probe.example.dz/a",
  "domain": "xss-probe.example.dz",
  "source_type": "web",
  "source_id": "xss-probe",
  "title": "<script>alert(1)</script> zzxssprobe",
  "excerpt": "<img src=x onerror=alert(2)> zzxssprobe payload",
  "body": "zzxssprobe",
  "language": "en", "script": "latin",
  "published_at": 1754438400, "crawled_at": 1754438400,
  "published_at_precision": "day",
  "quality_score": 0.5, "spam_score": 0.0,
  "schema_version": 1
}]))
PY
)
curl -sS -X POST "http://localhost:7700/indexes/documents/documents" \
  -H 'Content-Type: application/json' -d "$payload" > /dev/null
sleep 2

xss_json=$(search --data-urlencode "q=zzxssprobe")
xss_html=$(curl -sS "$BASE/search?q=zzxssprobe")

# The JSON API returns raw text; escaping is the renderer's job, so we assert the *renderer*.
#
# Assertions are on the exact injected forms. A bare `<script` would also match the page's own
# `<script src="/app.js">`, and `onerror=alert` matches the *escaped* text too — neither would
# tell us anything.
printf '%s' "$xss_html" | grep -qF '<script>alert(1)' \
  && bad "SCRIPT TAG SURVIVED INTO RENDERED HTML" || ok "injected script tag escaped"
printf '%s' "$xss_html" | grep -qF '<img src=x onerror' \
  && bad "IMG EVENT HANDLER SURVIVED INTO RENDERED HTML" || ok "injected img onerror escaped"
printf '%s' "$xss_html" | grep -qF '&lt;script&gt;alert(1)&lt;/script&gt;' \
  && ok "hostile title rendered as visible text" || bad "escaped title not found"
printf '%s' "$xss_html" | grep -qF '&lt;img src=x onerror=alert(2)&gt;' \
  && ok "hostile excerpt rendered as visible text" || bad "escaped excerpt not found"

# The only `<script` in the document should be our own, and it must have a src attribute.
stray=$(printf '%s' "$xss_html" | grep -oF '<script' | wc -l)
ours=$(printf '%s' "$xss_html" | grep -oF '<script src="/app.js"' | wc -l)
[ "$stray" -eq "$ours" ] && ok "no script tags beyond the page's own ($ours)" \
  || bad "found $stray script tags, expected $ours"

printf '%s' "$xss_json" | grep -q 'zzxssprobe' \
  && ok "hostile document is still indexed and findable" || bad "probe document not found"

curl -sS -X DELETE "http://localhost:7700/indexes/documents/documents/$XSS_ID" > /dev/null

# ---------------------------------------------------------------------------------------
section "security headers"

hdrs=$(curl -sS -D - -o /dev/null "$BASE/healthz")
echo "$hdrs" | grep -qi "content-security-policy: default-src 'self'" \
  && ok "CSP default-src 'self'" || bad "CSP missing"
echo "$hdrs" | grep -qi "referrer-policy: no-referrer" \
  && ok "Referrer-Policy: no-referrer" || bad "Referrer-Policy missing"
echo "$hdrs" | grep -qi "x-content-type-options: nosniff" \
  && ok "X-Content-Type-Options" || bad "X-Content-Type-Options missing"
echo "$hdrs" | grep -qi "permissions-policy:" \
  && ok "Permissions-Policy" || bad "Permissions-Policy missing"

# ---------------------------------------------------------------------------------------
section "no query text in logs"

if [ -f /tmp/xustive-api.log ]; then
  canary="zzqueryleakcanary$$"
  search --data-urlencode "q=$canary" > /dev/null
  sleep 0.4
  grep -q "$canary" /tmp/xustive-api.log \
    && bad "QUERY LEAKED INTO LOGS" || ok "query text absent from logs"
else
  printf '  \033[33m~\033[0m skipped log scan (no /tmp/xustive-api.log)\n'
fi

# ---------------------------------------------------------------------------------------
section "latency"

total=0
for _ in $(seq 1 20); do
  ms=$(curl -sS -o /dev/null -w '%{time_total}' -G "$BASE/api/v1/search" \
        --data-urlencode "q=سونلغاز فاتورة")
  total=$(python3 -c "print($total + $ms * 1000)")
done
avg=$(python3 -c "print(round($total / 20, 1))")
under=$(python3 -c "print(1 if $avg <= 200 else 0)")
[ "$under" = "1" ] && ok "mean latency ${avg}ms (budget 200ms)" \
  || bad "mean latency ${avg}ms exceeds the 200ms budget"

# ---------------------------------------------------------------------------------------
printf '\n\033[1m%d passed, %d failed\033[0m\n' "$pass" "$fail"
exit $(( fail > 0 ? 1 : 0 ))
