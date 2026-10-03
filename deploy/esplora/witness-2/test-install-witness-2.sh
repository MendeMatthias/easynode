#!/usr/bin/env bash
# ok and bad always succeed, so `check && ok || bad` is an if/else here.
# shellcheck disable=SC2015
#
# Prove what install-witness-2.sh does before it is ever run on the box.
#
# It runs here, as an ordinary user, against a copy of the box's Caddyfile
# SHAPE (test-fixtures/Caddyfile.box-shape, no credentials, no live file). It
# sources the install script's functions and never calls its main, so nothing
# is downloaded, installed, started or reloaded.
#
#   deploy/esplora/witness-2/test-install-witness-2.sh
#
# The routing check needs a caddy carrying the rate-limit plugin
# (deploy/esplora/build-caddy.sh) plus python3 and curl. Without one it says
# SKIP for that part and still runs everything else.
set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
FIXTURE="$HERE/test-fixtures/Caddyfile.box-shape"
export PATH="$HOME/.local/bin:$HOME/go/bin:$PATH"

# shellcheck source-path=SCRIPTDIR source=install-witness-2.sh
. "$HERE/install-witness-2.sh"

WORK="$(mktemp -d)"
pids=()
cleanup() {
  for p in "${pids[@]:-}"; do [ -n "$p" ] && kill "$p" 2>/dev/null; done
  wait 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT

pass=0; fail=0
ok()  { printf '  PASS  %s\n' "$*"; pass=$((pass+1)); }
bad() { printf '  FAIL  %s\n' "$*"; fail=$((fail+1)); }

echo "── the unit the script writes is the unit in the repo ──"
if diff <(unit_text) "$HERE/btx-witness-2.service" >"$WORK/unit.diff"; then
  ok "unit_text matches btx-witness-2.service byte for byte"
else
  bad "unit_text differs from btx-witness-2.service:"; cat "$WORK/unit.diff"
fi

echo "── arguments ──"
GOOD_SHA="$(printf '%064d' 0 | tr 0 a)"
valid_args "https://example.com/btx-witness" "$GOOD_SHA" && ok "https URL and 64 hex accepted" || bad "https URL and 64 hex refused"
valid_args "file:///tmp/btx-witness" "$GOOD_SHA" && ok "file URL accepted" || bad "file URL refused"
valid_args "https://example.com/x" "$(printf '%s' "$GOOD_SHA" | tr a A)" && ok "upper-case sha accepted" || bad "upper-case sha refused"
valid_args "http://example.com/x" "$GOOD_SHA" 2>/dev/null && bad "plain http accepted" || ok "plain http refused"
valid_args "https://example.com/x" "abc" 2>/dev/null && bad "short sha accepted" || ok "short sha refused"
valid_args "https://example.com/x" "" 2>/dev/null && bad "empty sha accepted" || ok "empty sha refused"
valid_args "" "$GOOD_SHA" 2>/dev/null && bad "empty URL accepted" || ok "empty URL refused"

echo "── the checksum ──"
printf 'witness bytes' > "$WORK/bin"
real="$(python3 -c 'import hashlib,sys; print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$WORK/bin")"
sha_matches "$WORK/bin" "$real" && ok "the right sha256 matches" || bad "the right sha256 did not match"
sha_matches "$WORK/bin" "$(printf '%s' "$real" | tr 'a-f' 'A-F')" && ok "upper case matches too" || bad "upper case did not match"
sha_matches "$WORK/bin" "$GOOD_SHA" && bad "a wrong sha256 matched" || ok "a wrong sha256 is refused"

echo "── the Caddyfile edit ──"
cp "$FIXTURE" "$WORK/Caddyfile"
caddy_has_witness "$WORK/Caddyfile" && bad "the fixture already counts as edited" || ok "the fixture has no witness route"
if caddy_insert_witness "$WORK/Caddyfile" "$WORK/Caddyfile.new" 2>"$WORK/err"; then
  ok "the edit succeeds on the box shape"
else
  bad "the edit failed on the box shape: $(cat "$WORK/err")"
fi
caddy_has_witness "$WORK/Caddyfile.new" && ok "the edited file counts as edited" || bad "the edited file does not count as edited"
n="$(grep -c 'handle /witness/\*' "$WORK/Caddyfile.new")"
[ "$n" = "1" ] && ok "exactly one witness handle" || bad "$n witness handles"
# Only lines were added: every original line is still there, in order.
if diff "$WORK/Caddyfile" "$WORK/Caddyfile.new" | grep -q '^[<]'; then
  bad "the edit removed or changed an existing line"
else
  ok "the edit only adds lines"
fi
# Inside the api.btxscan.io block, ahead of `import esplora_api`.
site_line="$(grep -n '^api\.btxscan\.io {' "$WORK/Caddyfile.new" | cut -d: -f1)"
handle_line="$(grep -n 'handle /witness/\*' "$WORK/Caddyfile.new" | cut -d: -f1)"
import_line="$(awk -v s="$site_line" 'NR > s && /import esplora_api/ { print NR; exit }' "$WORK/Caddyfile.new")"
if [ -n "$site_line" ] && [ -n "$handle_line" ] && [ -n "$import_line" ] \
   && [ "$handle_line" -gt "$site_line" ] && [ "$handle_line" -lt "$import_line" ]; then
  ok "the handle sits inside the api.btxscan.io site, before the esplora import"
else
  bad "handle at line ${handle_line:-?}, site at ${site_line:-?}, import at ${import_line:-?}"
fi
grep -q 'uri strip_prefix /witness' "$WORK/Caddyfile.new" && ok "the prefix is stripped" || bad "no strip_prefix"
awk '/handle \/witness\/\*/,/^\t}/' "$WORK/Caddyfile.new" | grep -q 'import perip_limit' \
  && ok "the per-IP limit applies to the witness" || bad "no perip_limit inside the witness handle"
awk '/handle \/witness\/\*/,/^\t}/' "$WORK/Caddyfile.new" | grep -q 'reverse_proxy 127.0.0.1:3081' \
  && ok "it proxies to 127.0.0.1:3081" || bad "wrong upstream"

echo "── the edit refuses shapes it does not know ──"
cp "$WORK/Caddyfile.new" "$WORK/twice"
caddy_insert_witness "$WORK/twice" "$WORK/twice.new" 2>/dev/null \
  && bad "a second insert was allowed" || ok "a file that already has the route is not edited again"
grep -v '^api\.btxscan\.io {' "$FIXTURE" > "$WORK/nosite"
caddy_insert_witness "$WORK/nosite" "$WORK/nosite.new" 2>/dev/null \
  && bad "edited a file with no api.btxscan.io site" || ok "no api.btxscan.io site: refused"
{ cat "$FIXTURE"; printf '\napi.btxscan.io {\n\timport esplora_api\n}\n'; } > "$WORK/twosites"
caddy_insert_witness "$WORK/twosites" "$WORK/twosites.new" 2>/dev/null \
  && bad "edited a file with two api.btxscan.io sites" || ok "two api.btxscan.io sites: refused"
grep -v '^(perip_limit) {' "$FIXTURE" > "$WORK/nolimit"
caddy_insert_witness "$WORK/nolimit" "$WORK/nolimit.new" 2>/dev/null \
  && bad "edited a file with no (perip_limit) snippet" || ok "no (perip_limit) snippet: refused"

echo "── Caddy itself: the witness route wins over every freshness handle ──"
CADDY="${CADDY:-$(command -v caddy || true)}"
if [ -z "$CADDY" ] || ! "$CADDY" list-modules 2>/dev/null | grep -q '^http.handlers.rate_limit$'; then
  echo "  SKIP  no caddy with the rate_limit module (deploy/esplora/build-caddy.sh builds one)"
elif ! command -v python3 >/dev/null || ! command -v curl >/dev/null; then
  echo "  SKIP  needs python3 and curl"
else
  FRONT_PORT="${FRONT_PORT:-13180}"; ELECTRS_PORT="${ELECTRS_PORT:-13100}"; WITNESS_PORT="${WITNESS_PORT:-13181}"
  export CADDY_ADMIN="${CADDY_ADMIN:-127.0.0.1:12119}"
  mkdir -p "$WORK/run"
  # The fresh marker present: if the witness handle sorted after the
  # freshness handles, @fresh would take /witness/* to electrs.
  : > "$WORK/run/btx-fresh"
  # The edited box shape, pointed at stubs on free ports and served over
  # plain HTTP on localhost. Only addresses change, never the structure.
  sed -e "s#^api\\.btxscan\\.io {#http://127.0.0.1:$FRONT_PORT {#" \
      -e "s#127\\.0\\.0\\.1:3081#127.0.0.1:$WITNESS_PORT#" \
      -e "s#127\\.0\\.0\\.1:3000#127.0.0.1:$ELECTRS_PORT#" \
      -e "s#root /run#root $WORK/run#" \
      -e "s#/var/log/caddy/access.log#$WORK/access.log#" \
      "$WORK/Caddyfile.new" > "$WORK/Caddyfile.run"
  stub() { # port body
    python3 - "$1" "$2" <<'PY' &
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer
port, body = int(sys.argv[1]), sys.argv[2]
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        b = (body + " " + self.path).encode()
        self.send_response(200); self.send_header("Content-Length", str(len(b))); self.end_headers()
        self.wfile.write(b)
    def log_message(self, *a): pass
HTTPServer(("127.0.0.1", port), H).serve_forever()
PY
    pids+=($!)
  }
  stub "$ELECTRS_PORT" electrs
  stub "$WITNESS_PORT" witness
  if "$CADDY" validate --config "$WORK/Caddyfile.run" --adapter caddyfile >"$WORK/validate.log" 2>&1; then
    ok "caddy validate accepts the edited file"
  else
    bad "caddy validate refused it: $(tail -3 "$WORK/validate.log")"
  fi
  "$CADDY" run --config "$WORK/Caddyfile.run" --adapter caddyfile >"$WORK/caddy.log" 2>&1 & pids+=($!)
  for _ in $(seq 1 50); do
    curl -fsS "http://127.0.0.1:$FRONT_PORT/blocks/tip/height" >/dev/null 2>&1 && break
    sleep 0.2
  done
  got="$(curl -sS "http://127.0.0.1:$FRONT_PORT/witness/blocks/tip/height" 2>&1)"
  [ "$got" = "witness /blocks/tip/height" ] && ok "/witness/blocks/tip/height reaches the witness, prefix stripped" \
    || bad "/witness/blocks/tip/height answered: $got"
  got="$(curl -sS "http://127.0.0.1:$FRONT_PORT/witness/signers/recent" 2>&1)"
  [ "$got" = "witness /signers/recent" ] && ok "/witness/signers/recent reaches the witness" \
    || bad "/witness/signers/recent answered: $got"
  got="$(curl -sS "http://127.0.0.1:$FRONT_PORT/blocks/tip/height" 2>&1)"
  [ "$got" = "electrs /blocks/tip/height" ] && ok "everything else still reaches electrs" \
    || bad "/blocks/tip/height answered: $got"
  hdr="$(curl -sSI "http://127.0.0.1:$FRONT_PORT/blocks/tip/height" 2>&1 | tr -d '\r' | grep -i '^x-btx-freshness:')"
  [ "$hdr" = "X-Btx-Freshness: fresh" ] && ok "the freshness handles still answer for Esplora" \
    || bad "Esplora freshness header: ${hdr:-none}"
fi

echo
echo "──────────────────────────────────────────────"
printf '  passed %d, failed %d\n' "$pass" "$fail"
[ "$fail" -eq 0 ] || exit 1
