#!/usr/bin/env bash
# Prove that the node on this machine can be a fork witness and a swap index,
# whatever its prune posture.
#
# WHY THIS MATTERS. The wallet's fork check compares the block HASH at a height
# two sources both hold. Its only witness, esplora.btxbyronbay.com, has been
# frozen at 209,778 while the chain ran past 211,400, so the check has not run
# for days. Replacing it appeared to need a full 124 GiB archival node with a
# GPU, because that is what Esplora mode needs — and Esplora mode needs it for
# the ADDRESS index, which a witness is never asked about.
#
# Pruning discards block DATA, not the block INDEX. Every node knows every
# block hash. This runs the witness server against whatever node is here and
# checks its answers against that node's own getblockhash, including at heights
# far below the prune height, and checks that nothing else is served.
#
# Since 2026-11-28 the witness also serves the five routes an HTLC swap client
# checks across a quorum of operators (/tx/<txid>/status, /hex, /merkle-proof,
# /outspend/<vout>, /block/<hash>/header). The second half of this script
# checks each against the node's own RPC: the status against the block the
# node says holds the tx, the hex against getrawtransaction, the merkle proof
# by folding it into the header's merkle root, a real spend against the block
# that made it, and the header against getblockheader — plus the refusals.
#
#   deploy/esplora/test-witness.sh
#
# Needs btx-cli and the btx-witness binary (cargo build --bin btx-witness in
# crates/btx-core), and python3 for the swap checks. Skips with exit 0 when
# btx-cli or the binary is missing, or when no node answers. Read-only: it makes
# getblockchaininfo, getblockcount, getblockhash, getblock (verbosity 1 and 2),
# getblockheader, getchaintips, getrawtransaction and gettxout calls, and writes
# nothing. The swap index's first build reads
# every block in its window once; SWAP_WINDOW (default 2000 here, 100000 in
# production) keeps that short, and WITNESS_INDEX_WAIT (seconds, default 900)
# bounds the wait for it.
set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
export PATH="$HOME/.local/bin:$PATH"

find_bin() {
  local n="$1" p
  p="$(command -v "$n" 2>/dev/null)" && { printf '%s' "$p"; return; }
  for d in "${BTXD_BIN:-}" "$HOME"/.local/btx/*/*/bin; do
    [ -n "$d" ] && [ -x "$d/$n" ] && { printf '%s' "$d/$n"; return; }
  done
}
BTXCLI="$(find_bin btx-cli)"
WITNESS="${WITNESS_BIN:-}"
for c in "$REPO/crates/btx-core/target/release/btx-witness" \
         "$REPO/crates/btx-core/target/debug/btx-witness" \
         "$(command -v btx-witness 2>/dev/null || true)"; do
  [ -z "$WITNESS" ] && [ -x "$c" ] && WITNESS="$c"
done
[ -n "$BTXCLI" ] || { echo "SKIP: no btx-cli found. Set BTXD_BIN to the directory holding it."; exit 0; }
[ -n "$WITNESS" ] || { echo "SKIP: no btx-witness binary. Build it: (cd crates/btx-core && cargo build --bin btx-witness)"; exit 0; }

DATADIR="${BTX_DATADIR:-$HOME/.easybtx}"
CONF="${BTX_CONF:-$DATADIR/faststart/faststart.conf}"
RPC="${BTX_RPC:-127.0.0.1:19334}"
PORT="${WITNESS_PORT:-13400}"
SWAP_WINDOW="${SWAP_WINDOW:-2000}"
INDEX_WAIT="${WITNESS_INDEX_WAIT:-900}"

C() {
  local port_arg=()
  # The RPC port is passed explicitly when BTX_RPC names one, so a node whose
  # conf does not say it (the 3060 signer runs on 19334 from the command line)
  # is still reached.
  [ -n "${BTX_RPC:-}" ] && port_arg=(-rpcport="${RPC##*:}")
  if [ -f "$CONF" ]; then "$BTXCLI" -datadir="$DATADIR" -conf="$CONF" "${port_arg[@]}" "$@"
  else "$BTXCLI" -datadir="$DATADIR" "${port_arg[@]}" "$@"; fi
}
if ! C getblockchaininfo >/dev/null 2>&1; then
  echo "SKIP: no node answered at $DATADIR. Set BTX_DATADIR/BTX_CONF, or start one."
  exit 0
fi
if (exec 3<>"/dev/tcp/127.0.0.1/$PORT") 2>/dev/null; then
  exec 3<&- 2>/dev/null
  echo "SKIP: 127.0.0.1:$PORT is in use; set WITNESS_PORT."
  exit 0
fi

pids=(); cleanup() { for p in "${pids[@]:-}"; do [ -n "$p" ] && kill "$p" 2>/dev/null; done; wait 2>/dev/null; }
trap cleanup EXIT
pass=0; fail=0
ok()  { printf '  \033[32mPASS\033[0m  %s\n' "$*"; pass=$((pass+1)); }
bad() { printf '  \033[31mFAIL\033[0m  %s\n' "$*"; fail=$((fail+1)); }

info="$(C getblockchaininfo)"
PRUNED="$(printf '%s' "$info" | grep -o '"pruned": [a-z]*' | awk '{print $2}')"
PRUNEH="$(printf '%s' "$info" | grep -o '"pruneheight": [0-9]*' | grep -o '[0-9]*')"
PRUNEH="${PRUNEH:-0}"
HEIGHT="$(C getblockcount)"
echo "node: pruned=${PRUNED:-false} pruneheight=$PRUNEH height=$HEIGHT"
echo

"$WITNESS" --datadir "$DATADIR" --rpc "$RPC" --listen "127.0.0.1:$PORT" --swap-window "$SWAP_WINDOW" >/dev/null 2>&1 & pids+=($!)
up=0
for _ in $(seq 1 60); do
  curl -sS -o /dev/null -m 2 "http://127.0.0.1:$PORT/blocks/tip/height" 2>/dev/null && { up=1; break; }
  sleep 0.25
done
[ "$up" -eq 1 ] || { bad "the witness server never came up"; echo; echo "  passed $pass, failed $fail"; exit 1; }

echo "── the two routes a wallet needs ──"
tip="$(curl -sS -m 5 "http://127.0.0.1:$PORT/blocks/tip/height")"
now="$(C getblockcount)"
# The node may have moved on since HEIGHT was read; either is its own height.
if [ "$tip" = "$HEIGHT" ] || [ "$tip" = "$now" ]; then ok "/blocks/tip/height -> $tip, the node's own height"
else bad "/blocks/tip/height was '$tip', the node says '$now'"; fi

echo
echo "── every hash matches the node, including below the prune height ──"
heights="1 50000 $((HEIGHT / 2)) $((HEIGHT - 6))"
[ "$PRUNEH" -gt 1 ] && heights="$heights $((PRUNEH - 1)) $PRUNEH"
for h in $heights; do
  [ "$h" -ge 0 ] 2>/dev/null || continue
  served="$(curl -sS -m 10 "http://127.0.0.1:$PORT/block-height/$h" | tr -d '\r\n')"
  real="$(C getblockhash "$h" 2>/dev/null | tr -d '\r\n')"
  if [ -n "$served" ] && [ "$served" = "$real" ]; then
    note=""
    [ "$PRUNEH" -gt 0 ] && [ "$h" -lt "$PRUNEH" ] && note=" (below pruneheight $PRUNEH)"
    ok "$h -> ${served:0:16}…$note"
  else
    bad "$h: served '${served:0:20}', node says '${real:0:20}'"
  fi
done

echo
echo "── and it makes no claim about anything else ──"
# A node serving witness data has promised nothing about an address index. The
# defect that retired the last independent witness was an address index that
# answered every route confidently while not recording spends; a witness that
# also served balances would be that machine.
for p in "/address/btx1z7nkymajxh9s089hm8f6ztasptx2nwlmgqqeh9ruxpn6klh3qa55sxvmjs5/utxo" \
         "/address/btx1z7nkymajxh9s089hm8f6ztasptx2nwlmgqqeh9ruxpn6klh3qa55sxvmjs5" \
         "/tx/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" \
         "/mempool" "/blocks" "/blocks/tip/hash" "/"; do
  c="$(curl -sS -o /dev/null -m 5 -w '%{http_code}' "http://127.0.0.1:$PORT$p")"
  [ "$c" = "404" ] && ok "$(printf '%.48s' "$p") -> 404" || bad "$p -> $c"
done
c="$(curl -sS -o /dev/null -m 5 -w '%{http_code}' -X POST --data 'ff' "http://127.0.0.1:$PORT/tx")"
[ "$c" = "404" ] && ok "POST /tx -> 404: a witness cannot be asked to broadcast" || bad "POST /tx -> $c"

echo
echo "── a height it does not have is 'I do not have that', not a fault ──"
c="$(curl -sS -o /dev/null -m 5 -w '%{http_code}' "http://127.0.0.1:$PORT/block-height/$((HEIGHT + 1000))")"
[ "$c" = "404" ] && ok "above the tip -> 404, which is how a caller learns this witness is behind" \
  || bad "above the tip -> $c"
c="$(curl -sS -o /dev/null -m 5 -w '%{http_code}' "http://127.0.0.1:$PORT/block-height/0123")"
[ "$c" = "404" ] && ok "a padded height -> 404: one height has one spelling" || bad "padded height -> $c"

echo
echo "── the five swap routes, each checked against the node ──"
if ! command -v python3 >/dev/null 2>&1; then
  echo "  (no python3; swap route checks skipped)"
else
  # The index reads every block in its window once before it answers; until
  # then the routes say 503, which is the right answer and not a failure.
  probe="$(C getblockhash "$((HEIGHT - 6))")"
  cb="$(C getblock "$probe" 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["tx"][0])')"
  t0=$(date +%s); code=503
  while [ "$code" = "503" ] && [ $(( $(date +%s) - t0 )) -lt "$INDEX_WAIT" ]; do
    code="$(curl -sS -o /dev/null -m 10 -w '%{http_code}' "http://127.0.0.1:$PORT/tx/$cb/status")"
    [ "$code" = "503" ] && sleep 2
  done
  if [ "$code" = "200" ]; then ok "the swap index was built in $(( $(date +%s) - t0 ))s (window $SWAP_WINDOW blocks)"
  else bad "the swap index did not answer within ${INDEX_WAIT}s (last status $code)"; fi

  # One python process does the comparisons: it asks the node through btx-cli
  # and the witness over HTTP, and prints PASS/FAIL lines this script counts.
  out="$(BTXCLI="$BTXCLI" DATADIR="$DATADIR" CONF="$CONF" RPCPORT="${BTX_RPC:+${RPC##*:}}" \
         BASE="http://127.0.0.1:$PORT" HEIGHT="$HEIGHT" PRUNEH="$PRUNEH" WINDOW="$SWAP_WINDOW" \
         python3 "$HERE/witness-swap-check.py" 2>&1)"
  while IFS= read -r line; do
    case "$line" in
      PASS\ *) ok "${line#PASS }" ;;
      FAIL\ *) bad "${line#FAIL }" ;;
      *) [ -n "$line" ] && echo "        $line" ;;
    esac
  done <<< "$out"
fi

echo
echo "── the same front the Esplora deployment uses, in front of it ──"
# The front is route-agnostic: point BTX_ESPLORA_ELECTRS at the witness instead
# of electrs and nothing about it changes. The freshness guardian works
# unchanged too, because it reads these same two routes.
if command -v caddy >/dev/null 2>&1 && caddy list-modules 2>/dev/null | grep -qx 'http.handlers.rate_limit'; then
  FPORT="${FRONT_PORT:-13401}"
  if (exec 3<>"/dev/tcp/127.0.0.1/$FPORT") 2>/dev/null; then
    exec 3<&- 2>/dev/null
    echo "  (127.0.0.1:$FPORT is in use; front checks skipped)"
  else
    RUN="$(mktemp -d)/run"; mkdir -p "$RUN"
    CADDY_ADMIN="${CADDY_ADMIN:-127.0.0.1:12019}" \
    BTX_ESPLORA_HOST="http://127.0.0.1:$FPORT" BTX_ESPLORA_RUN="$RUN" \
    BTX_ESPLORA_ELECTRS="127.0.0.1:$PORT" BTX_ESPLORA_BTXD_RPC="$RPC" \
      caddy run --config "$HERE/Caddyfile.template" --adapter caddyfile >/dev/null 2>&1 & pids+=($!)
    for _ in $(seq 1 40); do
      curl -sS -o /dev/null -m 2 "http://127.0.0.1:$FPORT/blocks/tip/height" 2>/dev/null && break
      sleep 0.5
    done
    ftip="$(curl -sS -m 5 "http://127.0.0.1:$FPORT/blocks/tip/height")"
    [ -n "$ftip" ] && [ "$ftip" -ge "$HEIGHT" ] 2>/dev/null && ok "the front serves the same tip through to the witness" || bad "front tip '$ftip'"
    co="$(curl -sSI -m 5 "http://127.0.0.1:$FPORT/blocks/tip/height" | tr -d '\r' | grep -ci '^access-control-allow-origin:')"
    [ "$co" = "1" ] && ok "CORS exactly once: the front is the only source" || bad "CORS appeared $co times"
    c="$(curl -sS -o /dev/null -m 5 -w '%{http_code}' "http://127.0.0.1:$FPORT/address/btx1z7nkymajxh9s089hm8f6ztasptx2nwlmgqqeh9ruxpn6klh3qa55sxvmjs5/utxo")"
    [ "$c" = "404" ] && ok "the money routes stay 404 through the front" || bad "an address route answered $c through the front"
    hdr_hash="$(C getblockhash "$((HEIGHT - 6))")"
    fh="$(curl -sS -m 10 "http://127.0.0.1:$FPORT/block/$hdr_hash/header")"
    [ "$fh" = "$(C getblockheader "$hdr_hash" false)" ] && ok "a swap route passes through the front unchanged" \
      || bad "the header through the front was '${fh:0:24}'"
    # And the real guardian, against the real census, if it can reach it.
    line="$(BTX_ESPLORA_RUN="$RUN" BTX_LOCAL_ESPLORA="http://127.0.0.1:$FPORT" bash "$HERE/btx-staleness-check.sh" 2>/dev/null)"
    case "$line" in
      state=fresh*)  ok "the guardian proved it on the heaviest measured chain: $line" ;;
      state=stale*)  ok "the guardian judged it, and this node is behind: $line" ;;
      state=unverified*) ok "the guardian could not prove it, and says so: $line" ;;
      *) bad "the guardian said nothing usable: '$line'" ;;
    esac
    rm -rf "$(dirname "$RUN")"
  fi
else
  echo "  (no caddy with the rate-limit plugin; front checks skipped)"
fi

echo
echo "──────────────────────────────────────────────"
printf '  passed %d, failed %d\n' "$pass" "$fail"
[ "$fail" -eq 0 ] || exit 1
if [ "${PRUNED:-false}" = "true" ]; then
  echo "  A PRUNED node served every route a wallet needs to settle a fork, and the five a swap checks."
else
  echo "  This node can serve as a fork witness and a swap index."
fi
