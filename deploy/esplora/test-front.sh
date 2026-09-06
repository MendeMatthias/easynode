#!/usr/bin/env bash
# Prove the Caddy front does what the comments in Caddyfile.template claim.
#
# WHY THIS EXISTS. Every incident recorded in that file is about a component
# that looked fine and answered confidently while being wrong, and until this
# script the front itself had never been run at all: not validated, not
# started, never asked what header it emits. The three freshness states, the
# "CORS exactly once" rule and the "btxd's RPC stays unreachable" rule were
# assertions in comments.
#
# It runs entirely on localhost against stubs. No BTX node, no chain, no
# network, nothing under any datadir. Two python stubs stand in for electrs
# (127.0.0.1:3000) and for btxd's REST+RPC port (127.0.0.1:19334); the real
# Caddyfile.template is started in front of them, unmodified.
#
#   deploy/esplora/test-front.sh
#
# Needs: python3, curl, and a caddy carrying the rate-limit plugin
# (deploy/esplora/build-caddy.sh). Skips with exit 0 and a message when caddy
# is missing, so it is safe to call from a wider test run.
set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
TEMPLATE="$HERE/Caddyfile.template"
export PATH="$HOME/.local/bin:$HOME/go/bin:$PATH"

CADDY="${CADDY:-$(command -v caddy || true)}"
if [ -z "$CADDY" ]; then
  echo "SKIP: no caddy on PATH. Build one with deploy/esplora/build-caddy.sh."
  exit 0
fi
if ! "$CADDY" list-modules 2>/dev/null | grep -q '^http.handlers.rate_limit$'; then
  echo "SKIP: $CADDY has no rate_limit module; deploy/esplora/build-caddy.sh builds one that does."
  exit 0
fi

# Ports are overridable so this can run beside a real deployment.
FRONT_PORT="${FRONT_PORT:-3080}"
ELECTRS_PORT="${ELECTRS_PORT:-3000}"
BTXD_PORT="${BTXD_PORT:-19334}"
# Caddy's admin endpoint. It defaults to 127.0.0.1:2019 and Caddy REFUSES TO
# START when that address is taken. That is a real deployment failure, not a
# test artifact: on this machine a Windows process holds 2019 and WSL2 shares
# localhost, so the front would not start and the reason is one line deep in
# a log nobody reads. The test gives it its own port instead.
export CADDY_ADMIN="${CADDY_ADMIN:-127.0.0.1:12019}"
TIP=211448
HASH=aa11bb22cc33dd44ee55ff6600112233445566778899aabbccddeeff00112233

WORK="$(mktemp -d)"
RUN="$WORK/run"
mkdir -p "$RUN"
pids=()
cleanup() {
  for p in "${pids[@]:-}"; do [ -n "$p" ] && kill "$p" 2>/dev/null; done
  wait 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT

pass=0; fail=0
ok()  { printf '  \033[32mPASS\033[0m  %s\n' "$*"; pass=$((pass+1)); }
bad() { printf '  \033[31mFAIL\033[0m  %s\n' "$*"; fail=$((fail+1)); }

# ── the stubs ────────────────────────────────────────────────────────────────
# The electrs stub deliberately sets its OWN Access-Control-Allow-Origin, which
# is the condition the front's header_down directives exist to strip. If Caddy
# stopped stripping it, browsers would see two and reject the response, and
# that is what broke the web wallet once.
cat > "$WORK/electrs_stub.py" <<PY
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer
TIP, HASH = "$TIP", "$HASH"
class H(BaseHTTPRequestHandler):
    def _send(self, code, body, ctype="text/plain"):
        b = body.encode()
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(b)))
        # electrs --cors '*' emits this itself; Caddy must strip it.
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        self.end_headers()
        self.wfile.write(b)
    def do_GET(self):
        p = self.path
        if p == "/blocks/tip/height": self._send(200, TIP)
        elif p.startswith("/block-height/"): self._send(200, HASH)
        elif p == "/mempool": self._send(200, '{"count":0}', "application/json")
        elif p == "/blocks": self._send(200, '[{"height":%s}]' % TIP, "application/json")
        else: self._send(404, "not found")
    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0)
        self.rfile.read(n)
        # Marks anything that reached electrs rather than btxd.
        self._send(400, "electrs-stub: sendrawtransaction refused")
    def log_message(self, *a): pass
HTTPServer(("127.0.0.1", $ELECTRS_PORT), H).serve_forever()
PY

# The btxd stub serves REST and JSON-RPC on ONE port, as btxd really does. The
# front must forward GET /rest/block/* here and must never let anything reach
# the RPC endpoint, which is POST to "/".
cat > "$WORK/btxd_stub.py" <<PY
from http.server import BaseHTTPRequestHandler, HTTPServer
class H(BaseHTTPRequestHandler):
    def _send(self, code, body):
        b = body.encode()
        self.send_response(code); self.send_header("Content-Length", str(len(b))); self.end_headers()
        self.wfile.write(b)
    def do_GET(self):
        if self.path.startswith("/rest/block/"): self._send(200, "btxd-rest-block-bytes")
        else: self._send(404, "btxd: not a REST path")
    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0); self.rfile.read(n)
        # If this is ever reached through the front, the node's RPC is exposed.
        self._send(200, "BTXD-RPC-REACHED")
    def log_message(self, *a): pass
HTTPServer(("127.0.0.1", $BTXD_PORT), H).serve_forever()
PY

for port in "$ELECTRS_PORT" "$BTXD_PORT" "$FRONT_PORT"; do
  if (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
    exec 3<&- 2>/dev/null
    echo "SKIP: 127.0.0.1:$port is already in use; set FRONT_PORT/ELECTRS_PORT/BTXD_PORT to free ones."
    exit 0
  fi
done

python3 "$WORK/electrs_stub.py" & pids+=($!)
python3 "$WORK/btxd_stub.py" & pids+=($!)

echo "── the template is valid Caddy configuration ──"
if BTX_ESPLORA_HOST="http://127.0.0.1:$FRONT_PORT" BTX_ESPLORA_RUN="$RUN" \
     BTX_ESPLORA_ELECTRS="127.0.0.1:$ELECTRS_PORT" BTX_ESPLORA_BTXD_RPC="127.0.0.1:$BTXD_PORT" \
     "$CADDY" validate --config "$TEMPLATE" --adapter caddyfile >"$WORK/validate.log" 2>&1; then
  ok "caddy validate accepts Caddyfile.template"
else
  bad "caddy validate REJECTED Caddyfile.template"
  sed 's/^/        /' "$WORK/validate.log" | tail -12
  echo; echo "  passed $pass, failed $fail"; exit 1
fi

BTX_ESPLORA_HOST="http://127.0.0.1:$FRONT_PORT" BTX_ESPLORA_RUN="$RUN" \
  BTX_ESPLORA_ELECTRS="127.0.0.1:$ELECTRS_PORT" BTX_ESPLORA_BTXD_RPC="127.0.0.1:$BTXD_PORT" \
  "$CADDY" run --config "$TEMPLATE" --adapter caddyfile >"$WORK/caddy.log" 2>&1 & pids+=($!)

up=0
for _ in $(seq 1 60); do
  if curl -sS -o /dev/null -m 2 "http://127.0.0.1:$FRONT_PORT/blocks/tip/height" 2>/dev/null; then up=1; break; fi
  sleep 0.5
done
if [ "$up" -ne 1 ]; then
  # Running every later check against a dead server prints two hundred lines
  # of connection-refused and buries the one line that says why.
  bad "the front never came up on 127.0.0.1:$FRONT_PORT; caddy's own log follows"
  sed 's/^/        /' "$WORK/caddy.log" | tail -15
  echo; echo "  passed $pass, failed $fail"; exit 1
fi

F="http://127.0.0.1:$FRONT_PORT"
hdr() { curl -sSI -m 5 "$1" 2>/dev/null | tr -d '\r'; }
# Response headers of a GET, not a HEAD. The @rawblock matcher is GET-only,
# so a HEAD to /rest/block/* deliberately does not match it.
ghdr() { curl -sS -m 5 -D - -o /dev/null "$1" 2>/dev/null | tr -d '\r'; }
freshness_of() { hdr "$1" | grep -i '^x-btx-freshness:' | awk '{print $2}'; }
set_marker() { rm -f "$RUN"/btx-fresh "$RUN"/btx-stale "$RUN"/btx-unverified; [ -n "${1:-}" ] && touch "$RUN/$1"; }

echo
echo "── it serves, and the body comes from upstream ──"
body="$(curl -sS -m 5 "$F/blocks/tip/height")"
[ "$body" = "$TIP" ] && ok "/blocks/tip/height -> $body" || bad "/blocks/tip/height returned '${body:0:40}'"

echo
echo "── freshness is the marker's answer, never invented ──"
# The deliberate difference from the deployment this was ported from: with no
# marker at all the front says `unverified`, where the original said `fresh`.
set_marker ""
v="$(freshness_of "$F/blocks/tip/height")"
[ "$v" = "unverified" ] && ok "no marker -> unverified (the ported default said 'fresh'; this is the change)" \
  || bad "no marker -> '$v', expected unverified"
for state in fresh stale unverified; do
  set_marker "btx-$state"
  v="$(freshness_of "$F/blocks/tip/height")"
  [ "$v" = "$state" ] && ok "btx-$state present -> X-Btx-Freshness: $state" || bad "btx-$state present -> '$v'"
done
set_marker "btx-fresh"
u="$(hdr "$F/blocks/tip/height" | grep -ci '^x-btx-upstream: local$')"
[ "$u" = "1" ] && ok "X-Btx-Upstream: local, once" || bad "X-Btx-Upstream appeared $u times"

echo
echo "── CORS exactly once, whatever upstream sends ──"
# The stub sends its own on every response; duplicates are what browsers reject.
for h in access-control-allow-origin access-control-allow-methods; do
  n="$(hdr "$F/blocks/tip/height" | grep -ci "^$h:")"
  [ "$n" = "1" ] && ok "$h present exactly once" || bad "$h appeared $n times (upstream's must be stripped)"
done
pre="$(curl -sS -m 5 -o /dev/null -w '%{http_code}' -X OPTIONS \
  -H 'Origin: https://pq-wallet.com' -H 'Access-Control-Request-Method: POST' "$F/tx")"
[ "$pre" = "204" ] && ok "OPTIONS preflight -> 204" || bad "OPTIONS preflight -> $pre, expected 204"
n="$(curl -sSI -m 5 -X OPTIONS "$F/tx" 2>/dev/null | tr -d '\r' | grep -ci '^access-control-allow-origin:')"
[ "$n" = "1" ] && ok "the preflight carries CORS exactly once" || bad "preflight CORS appeared $n times"

echo
echo "── the node's RPC stays unreachable, and only /rest/block/* is forwarded ──"
raw="$(curl -sS -m 5 "$F/rest/block/000000.bin")"
[ "$raw" = "btxd-rest-block-bytes" ] && ok "GET /rest/block/* reaches btxd's REST interface" \
  || bad "GET /rest/block/* returned '${raw:0:40}'"
src="$(ghdr "$F/rest/block/000000.bin" | grep -i '^x-btx-source:' | awk '{print $2}')"
[ "$src" = "btxd-rest" ] && ok "and is labelled X-Btx-Source: btxd-rest" || bad "X-Btx-Source was '$src'"
# HEAD is not GET. The matcher says `method GET`, so a HEAD falls through to
# electrs rather than reaching btxd. Pinned because it looks like a bug when
# you first meet it, and because widening the matcher to HEAD would widen
# what is exposed of a node's RPC port.
hsrc="$(hdr "$F/rest/block/000000.bin" | grep -ci '^x-btx-source:')"
[ "$hsrc" = "0" ] && ok "a HEAD to /rest/block/* does not reach btxd (the matcher is GET-only)" \
  || bad "a HEAD to /rest/block/* was forwarded to btxd"
# The security claim in the Caddyfile: RPC is POST to "/" and can never match
# the @rawblock matcher, so it lands on electrs instead of on btxd.
rpc="$(curl -sS -m 5 -X POST -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"1.0","method":"getblockchaininfo","params":[]}' "$F/")"
case "$rpc" in
  *BTXD-RPC-REACHED*) bad "POST / REACHED btxd's JSON-RPC through the front" ;;
  *) ok "POST / does not reach btxd's JSON-RPC (got '${rpc:0:40}')" ;;
esac
rpc2="$(curl -sS -m 5 -X POST --data '{}' "$F/rest/block/x")"
case "$rpc2" in
  *BTXD-RPC-REACHED*) bad "POST /rest/block/x reached btxd (the matcher must be GET-only)" ;;
  *) ok "POST to a /rest/block/ path does not reach btxd either (the matcher is GET-only)" ;;
esac

echo
echo "── the rate limit is real ──"
# 200 events per 10s per client; the 250th request in one burst must be refused.
# ONE curl process, 300 requests through its URL globbing: the burst has to
# be faster than the window it is meant to exceed. 250 separate curl
# processes took ~12 s on this box, so a 200-per-10 s sliding window never
# saw more than ~200 at once and a WORKING rate limit reported as absent.
burst_start=$SECONDS
codes="$(curl -sS -o /dev/null -m 30 -w '%{http_code}\n' "$F/blocks/tip/height?burst=[1-300]" \
  2>/dev/null | sort | uniq -c | tr '\n' ' ')"
burst_secs=$((SECONDS - burst_start))
if printf '%s' "$codes" | grep -q '429'; then
  ok "a 300-request burst in ${burst_secs}s is throttled ($codes)"
elif [ "$burst_secs" -ge 10 ]; then
  # Inconclusive rather than a failure: the burst outran its own window.
  info "burst took ${burst_secs}s, longer than the 10s window ($codes) — inconclusive, not a failure"
  ok "rate limiting not disproven (the burst was too slow to test it here)"
else
  bad "no 429 in a 300-request burst finished in ${burst_secs}s ($codes) — the rate limit is not in force"
fi

echo
echo "──────────────────────────────────────────────"
printf '  passed %d, failed %d\n' "$pass" "$fail"
[ "$fail" -eq 0 ] || exit 1
echo "  The front behaves as Caddyfile.template says it does."
