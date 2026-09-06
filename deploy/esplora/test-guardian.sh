#!/usr/bin/env bash
# Prove the freshness guardian decides what its header says it decides.
#
# WHY THIS EXISTS. btx-staleness-check.sh is the thing that decides whether a
# served endpoint is labelled fresh, stale or unverified, and until this script
# it had never been run. Its rules are documented as identical to
# crates/btx-core/src/esplora_freshness.rs, which has 35 tests; the shell had
# none, and "change one and change the other" is not a mechanism.
#
# Everything is local: a python stub stands in for the served Esplora endpoint
# and a second one serves a canned census, so no node, no chain and no network
# are involved. The markers go to a temporary directory, never /run.
#
#   deploy/esplora/test-guardian.sh
#
# Needs python3 and curl.
set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
GUARD="$HERE/btx-staleness-check.sh"
LOCAL_PORT="${LOCAL_PORT:-13100}"
CENSUS_PORT="${CENSUS_PORT:-13101}"

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

for port in "$LOCAL_PORT" "$CENSUS_PORT"; do
  if (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
    exec 3<&- 2>/dev/null
    echo "SKIP: 127.0.0.1:$port is in use; set LOCAL_PORT/CENSUS_PORT to free ones."
    exit 0
  fi
done

# ── the stubs ────────────────────────────────────────────────────────────────
# Both read their answers from files, so a case is set up by writing a file
# rather than by restarting a server.
cat > "$WORK/stub.py" <<'PY'
import os, sys
from http.server import BaseHTTPRequestHandler, HTTPServer
WORK = sys.argv[1]
PORT = int(sys.argv[2])
KIND = sys.argv[3]
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        if KIND == "census":
            body, code = open(os.path.join(WORK, "census.json"), "rb").read(), 200
        elif self.path == "/blocks/tip/height":
            body, code = open(os.path.join(WORK, "tip"), "rb").read().strip(), 200
        elif self.path.startswith("/block-height/"):
            h = self.path.rsplit("/", 1)[-1]
            f = os.path.join(WORK, "h" + h)
            if os.path.exists(f):
                body, code = open(f, "rb").read().strip(), 200
            else:
                body, code = b"not found", 404
        else:
            body, code = b"not found", 404
        self.send_response(code)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *a): pass
HTTPServer(("127.0.0.1", PORT), H).serve_forever()
PY

# A 64-hex hash starting with the given prefix.
full() { printf '%s' "$1"; printf '0%.0s' $(seq 1 $((64 - ${#1}))); }

# The 2026-09-06 00:00Z shape, with settled pairs: chain A is heaviest with tip
# 211404, which the project's validator held as a ONE-BLOCK ORPHAN while its
# active chain ran to 211416. Chain C is the 2026-09-05 minority branch.
write_census() {  # $1 = checkedAt
  cat > "$WORK/census.json" <<JSON
{"schema":2,"checkedAt":$1,"chains":{"split":true,"tipHeight":211404,"chains":[
 {"id":"A","tipHeight":211404,"tipHash":"d5cdc194a5bbc8a7","forkHeight":null,"nodes":6,
  "competing":true,"heaviest":true,"partial":false,
  "settled":[{"height":211396,"hash":"1111111111111111"},{"height":211398,"hash":"3333333333333333"}]},
 {"id":"C","tipHeight":210885,"tipHash":"457516cceb7b076a","forkHeight":210496,"nodes":1,
  "competing":true,"heaviest":false,"partial":false,
  "settled":[{"height":210879,"hash":"dddddddddddddddd"}]}]}}
JSON
}

now() { date -u +%s; }
run_guard() {
  BTX_ESPLORA_RUN="$RUN" \
  BTX_LOCAL_ESPLORA="http://127.0.0.1:$LOCAL_PORT" \
  BTX_CENSUS_URL="http://127.0.0.1:$CENSUS_PORT/api/nodes" \
    bash "$GUARD" 2>/dev/null
}
marker() {
  local found=""
  for m in btx-fresh btx-stale btx-unverified; do
    [ -e "$RUN/$m" ] && found="$found $m"
  done
  printf '%s' "${found# }"
}
# <label> <expected state> <expected marker> [substring the line must contain]
expect() {
  local label="$1" want_state="$2" want_marker="$3" needle="${4:-}"
  local line state
  line="$(run_guard)"
  state="${line%% *}"; state="${state#state=}"
  local m; m="$(marker)"
  if [ "$state" != "$want_state" ]; then bad "$label: state=$state, expected $want_state  [$line]"; return; fi
  if [ "$m" != "$want_marker" ]; then bad "$label: marker '$m', expected '$want_marker'  [$line]"; return; fi
  if [ -n "$needle" ] && ! printf '%s' "$line" | grep -q -- "$needle"; then
    bad "$label: the line does not say '$needle'  [$line]"; return
  fi
  ok "$label -> $line"
}

python3 "$WORK/stub.py" "$WORK" "$LOCAL_PORT" local & pids+=($!)
python3 "$WORK/stub.py" "$WORK" "$CENSUS_PORT" census & pids+=($!)
printf '211416' > "$WORK/tip"
write_census "$(now)"
for _ in $(seq 1 40); do
  curl -sS -o /dev/null -m 2 "http://127.0.0.1:$LOCAL_PORT/blocks/tip/height" 2>/dev/null && break
  sleep 0.25
done

echo "── a settled block below the racing window decides it ──"
# THE CASE THIS EXISTS FOR. The endpoint is twelve blocks past the census tip
# and does NOT hold that tip, because it was a one-block orphan. The old rules
# called that endpoint unverified. A settled block proves it is on the chain.
full 3333333333333333 > "$WORK/h211398"
full 1111111111111111 > "$WORK/h211396"
printf 'a433ed21d83356c1f13e49e6969e27e33cf4de78a71f809a268c13483b020676\n' > "$WORK/h211404"
expect "past an orphaned census tip, on the chain" fresh btx-fresh "why=settled-block-matches"

echo
echo "── a settled MISMATCH is a divergence, and the chain is named ──"
full ffffffffffffffff > "$WORK/h211398"
full dddddddddddddddd > "$WORK/h210879"
expect "serving another chain at a settled height" unverified btx-unverified "serves_chain=C"

echo
echo "── on the right chain but behind is stale, not fresh ──"
full 3333333333333333 > "$WORK/h211398"
rm -f "$WORK/h210879"
printf '211398' > "$WORK/tip"
expect "six blocks behind the heaviest tip" stale btx-stale "why=behind-on-heaviest-chain"
printf '211402' > "$WORK/tip"
expect "two blocks behind is within tolerance" fresh btx-fresh "why=settled-block-matches"

echo
echo "── no witness never clears anything ──"
printf '211416' > "$WORK/tip"
write_census "$(( $(now) - 4000 ))"
expect "a census older than 30 minutes" unverified btx-unverified "why=census-old"
printf 'not json at all' > "$WORK/census.json"
expect "a census that does not parse" unverified btx-unverified "why=no-census"
write_census "$(now)"

echo
echo "── the endpoint itself being down is said plainly ──"
printf 'not a height' > "$WORK/tip"
expect "the served tip is unreadable" unverified btx-unverified "why=local-down"
printf '211416' > "$WORK/tip"

echo
echo "── a feed with no settled pairs still works (the rules before #468) ──"
cat > "$WORK/census.json" <<JSON
{"schema":2,"checkedAt":$(now),"chains":{"split":false,"tipHeight":211404,"chains":[
 {"id":"A","tipHeight":211404,"tipHash":"d5cdc194a5bbc8a7","forkHeight":null,"nodes":6,
  "competing":true,"heaviest":true,"partial":false}]}}
JSON
printf 'd5cdc194a5bbc8a7000000000000000000000000000000000000000000000000\n' > "$WORK/h211404"
expect "holding the census tip on an old feed" fresh btx-fresh
printf 'a433ed21d83356c1f13e49e6969e27e33cf4de78a71f809a268c13483b020676\n' > "$WORK/h211404"
expect "not holding it, with no deep branch known" unverified btx-unverified "why=race-at-census-tip"

echo
echo "── exactly one marker exists, always ──"
n="$(ls "$RUN" | wc -l)"
[ "$n" = "1" ] && ok "one marker file in the run directory" || bad "$n marker files: $(ls "$RUN" | tr '\n' ' ')"

echo
echo "──────────────────────────────────────────────"
printf '  passed %d, failed %d\n' "$pass" "$fail"
[ "$fail" -eq 0 ] || exit 1
echo "  The guardian decides what its header says it decides."
