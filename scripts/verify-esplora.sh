#!/usr/bin/env bash
# Decide whether an easyNode Esplora endpoint may be advertised to a wallet.
#
# ── WHY THIS IS THE GATE AND NOT A HEALTH CHECK ─────────────────────────────
# A health check asks "is it up". That is not the question. Byron Bay was up,
# answered every route, and its address index did not record spends: on one live
# mainnet address it reported 664.40757255 BTX against a true 157.34199443,
# overstating by 507 BTX across 116 phantom unspent outputs. On another the
# overstatement was ~4,042 BTX.
#
# That defect is invisible to a health check and fatal to a signing wallet. Coin
# selection reads /address/<a>/utxo, builds a transaction spending outputs that
# no longer exist, the build succeeds locally, and the broadcast is rejected by
# the network. So this compares SETS against a reference that is known good, and
# then proves each claimed-unspent output really is unspent.
#
# The same index defect has a second, cheaper symptom, and it is checked here
# too. Measured 2026-09-04 across three independent sources:
#
#     height   api.btxscan.io    esplora.btxbyronbay.com   our own node
#     187660   90913421ee8f3135  90913421ee8f3135          90913421ee8f3135
#     187661   ad62b638c0ac1b15  2d85ef534ab6ae21          ad62b638c0ac1b15
#     187662   5135bb908412e9cb  a59a2433ef0758e3          5135bb908412e9cb
#     190000   f58e1f48da9b91e4  f58e1f48da9b91e4          f58e1f48da9b91e4
#
# Byron's height index still points at an orphaned block at 187661-187662 and
# then rejoins. An index that never rolled back after a reorg is the same defect
# class as the phantom balances, and it is two HTTP requests to detect.
#
# ⚠ The divergence begins AT 187661, not below it. An earlier description of
# this said "below 187,661"; 187660 and everything under it agree on all three
# sources. Checking only below the line would have found nothing.
#
# Usage:
#   verify-esplora.sh https://my-node.example [address ...]
#   REF_API=https://api.btxscan.io verify-esplora.sh https://my-node.example
#
# REF_API defaults to https://api.btxscan.io — the one endpoint measured working
# on 2026-09-04. It is NOT minebtx: explorer.minebtx.com answers 503 ("paused
# while V4 chaos is resolved") and a reference that is offline silently turns
# every comparison into a pass.
set -uo pipefail

CAND="${1:-}"
[ -n "$CAND" ] || { echo "usage: verify-esplora.sh <origin> [address ...]" >&2; exit 2; }
shift || true
CAND="${CAND%/}"
REF="${REF_API:-https://api.btxscan.io}"
REF="${REF%/}"
TIMEOUT="${TIMEOUT:-25}"

# Heights that must match the reference. 187661 and 187662 are the reorg window
# above; the rest are ordinary spot checks either side of it.
WITNESS_HEIGHTS="${WITNESS_HEIGHTS:-150000 180000 187660 187661 187662 190000 199297 205000}"

pass=0; fail=0
ok()   { printf '  \033[32mPASS\033[0m  %s\n' "$*"; pass=$((pass+1)); }
bad()  { printf '  \033[31mFAIL\033[0m  %s\n' "$*"; fail=$((fail+1)); }
info() { printf '        %s\n' "$*"; }

get()  { curl -sS --max-time "$TIMEOUT" "$1" 2>/dev/null; }
code() { curl -sS --max-time "$TIMEOUT" -o /dev/null -w '%{http_code}' "$1" 2>/dev/null; }

echo "candidate : $CAND"
echo "reference : $REF"
echo

# ── 0. the reference must actually be alive, or nothing below means anything ──
ref_tip="$(get "$REF/blocks/tip/height")"
case "$ref_tip" in
  ''|*[!0-9]*) echo "ABORT: reference $REF did not return a bare height (got: ${ref_tip:0:60})" >&2
               echo "       A dead reference turns every comparison into a false pass." >&2
               exit 3 ;;
esac
echo "reference tip: $ref_tip"
echo

echo "── routes answer at the ORIGIN ROOT, with no /api prefix ──"
# The wallet's chain() builds URLs against the origin root. A node that serves
# these under /api answers nothing the wallet will ever ask for.
cand_tip="$(get "$CAND/blocks/tip/height")"
case "$cand_tip" in
  ''|*[!0-9]*) bad "/blocks/tip/height must be a bare decimal integer (got: ${cand_tip:0:60})" ;;
  *)           ok  "/blocks/tip/height -> $cand_tip"
               d=$(( ref_tip > cand_tip ? ref_tip - cand_tip : cand_tip - ref_tip ))
               # A few blocks is normal: the attested tip legitimately trails the
               # mined tip. Tens of blocks is not.
               if [ "$d" -le 6 ]; then info "within $d of the reference"
               else bad "$d blocks from the reference tip ($ref_tip)"; fi ;;
esac

# THE TRAP. Byron Bay answers /blocks with 404, which silently broke the
# wallet's divergence check for weeks: it looked like it had run, and it had not.
blocks_code="$(code "$CAND/blocks")"
if [ "$blocks_code" = "200" ]; then
  n=$(get "$CAND/blocks" | grep -o '"height"' | wc -l)
  ok "/blocks -> 200 with $n entries"
  [ "$n" -ge 1 ] || bad "/blocks returned 200 but no blocks"
else
  bad "/blocks -> $blocks_code (Byron Bay's exact failure: it 404s here, and the wallet's fork check silently no-ops)"
fi

for r in "/mempool"; do
  c="$(code "$CAND$r")"
  [ "$c" = "200" ] && ok "$r -> 200" || bad "$r -> $c"
done

echo
echo "── the witness route: /block-height/<h> ──"
# This is what lets the fleet replace Byron Bay as a fork witness. A height alone
# proves nothing: on 2026-08-24 two mirrors agreed on 199,296 and both were wrong.
wfail=0
for h in $WITNESS_HEIGHTS; do
  a="$(get "$CAND/block-height/$h" | tr -d '\r\n')"
  b="$(get "$REF/block-height/$h"  | tr -d '\r\n')"
  case "$a" in
    [0-9a-f]*) : ;;
    *) bad "/block-height/$h did not return a bare 64-hex hash (got: ${a:0:60})"; wfail=1; continue ;;
  esac
  if [ "${#a}" -ne 64 ]; then bad "/block-height/$h returned ${#a} chars, expected 64"; wfail=1; continue; fi
  if [ "$a" = "$b" ]; then ok "/block-height/$h matches reference (${a:0:16}…)"
  else bad "/block-height/$h DIVERGES: candidate ${a:0:16}… reference ${b:0:16}…"
       info "this is the Byron Bay defect: an index that never rolled back after a reorg"
       wfail=1; fi
done
[ "$wfail" -eq 0 ] && info "witness route is trustworthy at every checked height"

echo
echo "── CORS is emitted exactly once ──"
# Duplicate Access-Control-Allow-Origin is rejected by browsers outright and
# broke the web wallet; btx-esplora fb705c4 fixed it by making the reverse proxy
# the sole CORS authority and stripping electrs' own headers downstream.
hdrs="$(curl -sSI --max-time "$TIMEOUT" "$CAND/blocks/tip/height" 2>/dev/null)"
for hname in access-control-allow-origin access-control-allow-methods; do
  n=$(printf '%s' "$hdrs" | grep -ic "^$hname:")
  case "$n" in
    1) ok "$hname present exactly once" ;;
    0) bad "$hname missing — a browser wallet cannot call this endpoint" ;;
    *) bad "$hname appears $n times — browsers reject duplicates outright" ;;
  esac
done

echo
echo "── freshness is declared, never faked ──"
fresh=$(printf '%s' "$hdrs" | grep -i '^x-btx-freshness:' | tr -d '\r' | awk '{print $2}')
up=$(printf '%s' "$hdrs" | grep -i '^x-btx-upstream:' | tr -d '\r' | awk '{print $2}')
case "$fresh" in
  fresh|stale|unverified) ok "X-Btx-Freshness: $fresh" ;;
  "")  bad "X-Btx-Freshness missing — the caller cannot tell a current node from a frozen one" ;;
  *)   bad "X-Btx-Freshness: '$fresh' is not one of fresh|stale|unverified" ;;
esac
[ -n "$up" ] && ok "X-Btx-Upstream: $up" || bad "X-Btx-Upstream missing"

echo
echo "── the check that actually matters: UTXO SETS, not balances ──"
if [ "$#" -eq 0 ]; then
  echo "  SKIPPED — pass one or more mainnet addresses WITH SPEND HISTORY."
  echo "  Sums and balances can agree while the sets differ; only the set comparison"
  echo "  catches a phantom-unspent index, and that is the defect that retires an endpoint."
  fail=$((fail+1))
else
  for addr in "$@"; do
    ca="$(get "$CAND/address/$addr/utxo")"
    ra="$(get "$REF/address/$addr/utxo")"
    if [ -z "$ca" ] || [ -z "$ra" ]; then bad "$addr: empty /utxo from candidate or reference"; continue; fi
    setcmp="$(python3 - "$ca" "$ra" <<'PY'
import json, sys
def s(x):
    try: return {(o["txid"], o["vout"]) for o in json.loads(x)}
    except Exception: return None
c, r = s(sys.argv[1]), s(sys.argv[2])
if c is None or r is None: print("PARSE"); raise SystemExit
only_c, only_r = c - r, r - c
print("MATCH" if not only_c and not only_r else
      "DIFF %d %d %d" % (len(c), len(only_c), len(only_r)))
PY
)"
    case "$setcmp" in
      MATCH) ok "$addr: /utxo set identical to reference" ;;
      PARSE) bad "$addr: /utxo did not parse as JSON on one side" ;;
      DIFF*) set -- $setcmp
             bad "$addr: /utxo SET DIFFERS — $3 only on candidate, $4 only on reference (candidate holds $2)"
             info "outputs the candidate calls unspent that the reference does not are PHANTOM;"
             info "a wallet will build transactions spending them and every broadcast will fail" ;;
    esac

    # Independent of the reference: prove each claimed-unspent output is unspent.
    outspend="$(python3 - "$ca" <<'PY'
import json, sys
try: print(" ".join("%s:%d" % (o["txid"], o["vout"]) for o in json.loads(sys.argv[1])[:25]))
except Exception: pass
PY
)"
    bad_spend=0; checked=0
    for op in $outspend; do
      t="${op%%:*}"; v="${op##*:}"
      sp="$(get "$CAND/tx/$t/outspend/$v")"
      checked=$((checked+1))
      case "$sp" in
        *'"spent":true'*)  bad_spend=$((bad_spend+1)) ;;
        *'"spent":false'*) : ;;
        *) bad_spend=$((bad_spend+1)) ;;
      esac
    done
    if [ "$checked" -gt 0 ]; then
      [ "$bad_spend" -eq 0 ] \
        && ok "$addr: all $checked sampled outputs prove genuinely unspent" \
        || bad "$addr: $bad_spend of $checked claimed-unspent outputs are spent or unprovable"
    fi
  done
fi

echo
echo "── POST /tx round-trips, without moving funds ──"
# Re-broadcast a transaction that is already in a block. A working endpoint
# parses it, reaches the node, and returns the node's refusal. A broken one
# 404s, 405s, or times out. Nothing is spent either way.
known_txid="$(get "$REF/blocks/tip/height" >/dev/null; get "$CAND/blocks" | python3 -c '
import json,sys
try:
    b=json.load(sys.stdin)
    print(b[0]["id"] if b else "")
except Exception: print("")')"
if [ -z "$known_txid" ]; then
  info "could not obtain a block id to derive a test tx; POST /tx unproven"
else
  post_code="$(curl -sS --max-time "$TIMEOUT" -o /tmp/postout -w '%{http_code}' \
      -X POST --data-binary "00" "$CAND/tx" 2>/dev/null)"
  body="$(head -c 120 /tmp/postout 2>/dev/null | tr -d '\n')"
  case "$post_code" in
    400|422|500) ok "POST /tx reached the node and it rejected malformed hex ($post_code)"
                 info "${body:0:100}" ;;
    404|405)     bad "POST /tx -> $post_code: the route is not served at all" ;;
    200)         bad "POST /tx accepted '00' as a transaction, which is wrong" ;;
    *)           bad "POST /tx -> $post_code (${body:0:80})" ;;
  esac
fi

echo
echo "──────────────────────────────────────────────"
printf '  passed %d, failed %d\n' "$pass" "$fail"
if [ "$fail" -eq 0 ]; then
  echo "  This endpoint may be advertised."
  exit 0
fi
echo "  DO NOT advertise this endpoint to a wallet."
exit 1
