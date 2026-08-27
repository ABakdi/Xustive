#!/usr/bin/env bash
# The thumbnail proxy's refusals, against a running web on :3000 (M9-T02.5, landed with M10-T05.5).
#
# The proxy is the one place the reader's browser and a crawled host could meet, so what it
# refuses matters more than what it serves: a forged signature, a private host, an unsigned
# reverse-image fetch, and a description the words leg did not produce. The positive path — a
# real thumbnail through a valid signature — is covered by the no-JS check rendering the Images
# tab; here every request must be turned away.
set -uo pipefail
WEB="${WEB:-http://127.0.0.1:3000}"
pass=0; fail=0
ok()  { printf '  \033[32m✓\033[0m %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  \033[31m✗\033[0m %s\n' "$1"; fail=$((fail + 1)); }
code() { curl -sS -o /dev/null -w '%{http_code}' "$@"; }
expect() { local want="$1" label="$2"; shift 2; local got; got=$(code "$@"); [ "$got" = "$want" ] && ok "$label ($got)" || bad "$label (got $got, want $want)"; }

echo "thumbnail proxy on $WEB"
expect 403 "no signature is refused"            "$WEB/api/thumb?u=https%3A%2F%2Fexample.com%2Fa.jpg"
expect 403 "a forged signature is refused"      "$WEB/api/thumb?u=https%3A%2F%2Fexample.com%2Fa.jpg&s=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
expect 403 "a private host is refused even signed-looking" "$WEB/api/thumb?u=http%3A%2F%2F127.0.0.1%3A8080%2Fhealthz&s=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
expect 403 "an https private name is refused"   "$WEB/api/thumb?u=https%3A%2F%2Fmeilisearch.internal%2F&s=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
# Public image hosts are served unsigned — but only images, and only those hosts.
expect 403 "an unsigned non-public host is refused" "$WEB/api/thumb?u=https%3A%2F%2Fexample.com%2Fa.jpg&s="
echo "reverse image search on $WEB"
expect 403 "the URL leg needs a valid signature"    "$WEB/api/image-search?u=https%3A%2F%2Fexample.com%2Fa.jpg&s=forged"
expect 400 "an empty upload is refused"             -X POST "$WEB/api/image-search"
expect 422 "the words leg refuses what it did not say" "$WEB/api/image-search?web=Casbah%3B%20drop"
echo
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
