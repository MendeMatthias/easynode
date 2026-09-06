#!/usr/bin/env bash
# The whole stack against a REAL BTX node: btxd -> electrs -> the Caddy front.
#
# WHY THIS EXISTS. test-front.sh proves the front behaves, against stubs.
# test-guardian.sh proves the freshness rules decide correctly, against stubs.
# Neither runs the vendored electrs, and until this script nothing in this
# repository ever had: the fork of Blockstream's indexer and the BTX consensus
# decode crate beside it were vendored, compiled, tested at the unit level, and
# never once asked to index a chain and serve a wallet route.
#
# It uses a throwaway REGTEST chain, which is the part that makes it cheap: a
# regtest block needs no GPU and no 124 GiB of history, so the whole run is
# seconds and the answer to "does this software actually work for BTX" is yes
# or no rather than an opinion.
#
# WHAT IT DOES NOT PROVE, said plainly so nobody quotes it as an acceptance:
# nothing about mainnet scale, nothing about the mainnet index, and nothing
# about an endpoint's correctness against another source. That is
# scripts/verify-esplora.sh, which needs a full unpruned chain.
#
#   deploy/esplora/test-stack.sh
#
# Needs: btxd and btx-cli (BTXD_BIN=/path, or on PATH, or in
# ~/.local/btx/*/<platform>/bin), electrs, python3, curl. Skips with exit 0
# when a binary is missing. NOTHING touches ~/.easybtx: its own datadir under
# /tmp, its own ports, removed at the end.
set -uo pipefail

export PATH="$HOME/.local/bin:$PATH"
HERE="$(cd "$(dirname "$0")" && pwd)"

find_btx() {
  local n="$1" p
  p="$(command -v "$n" 2>/dev/null)" && { printf '%s' "$p"; return; }
  for d in "${BTXD_BIN:-}" "$HOME"/.local/btx/*/*/bin; do
    [ -n "$d" ] && [ -x "$d/$n" ] && { printf '%s' "$d/$n"; return; }
  done
}
BTXD="$(find_btx btxd)"
BTXCLI="$(find_btx btx-cli)"
if [ -z "$BTXD" ] || [ -z "$BTXCLI" ]; then
  echo "SKIP: no btxd/btx-cli found. Set BTXD_BIN to the directory holding them."
  exit 0
fi
if ! command -v electrs >/dev/null 2>&1; then
  echo "SKIP: no electrs on PATH. Build one with deploy/esplora/build-electrs.sh."
  exit 0
fi

RPC="${RPC_PORT:-18443}"
P2P="${P2P_PORT:-18444}"
EPORT="${ELECTRS_PORT:-13200}"
EELECTRUM="${ELECTRUM_PORT:-13201}"
FPORT="${FRONT_PORT:-13202}"
BLOCKS="${BLOCKS:-20}"

for port in "$RPC" "$P2P" "$EPORT" "$EELECTRUM" "$FPORT"; do
  if (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
    exec 3<&- 2>/dev/null
    echo "SKIP: 127.0.0.1:$port is in use; override the *_PORT variables."
    exit 0
  fi
done

WORK="$(mktemp -d)"
DATADIR="$WORK/data"; DBDIR="$WORK/electrs-db"; RUN="$WORK/run"
mkdir -p "$DATADIR" "$DBDIR" "$RUN"
pids=()
cleanup() {
  "$BTXCLI" -datadir="$DATADIR" -rpcport="$RPC" stop >/dev/null 2>&1
  sleep 2
  for p in "${pids[@]:-}"; do [ -n "$p" ] && kill "$p" 2>/dev/null; done
  wait 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT

pass=0; fail=0
ok()  { printf '  \033[32mPASS\033[0m  %s\n' "$*"; pass=$((pass+1)); }
bad() { printf '  \033[31mFAIL\033[0m  %s\n' "$*"; fail=$((fail+1)); }
C() { "$BTXCLI" -datadir="$DATADIR" -rpcport="$RPC" "$@"; }

# Network-specific settings must live in the [regtest] section or btxd refuses
# to start ("only applied on regtest network when in [regtest] section").
cat > "$DATADIR/btx.conf" <<CONF
regtest=1
server=1
prune=0
txindex=0
[regtest]
listen=0
rpcbind=127.0.0.1
rpcallowip=127.0.0.1
fallbackfee=0.0001
rpcport=$RPC
port=$P2P
CONF

echo "── btxd, on its own regtest chain ──"
"$BTXD" -datadir="$DATADIR" -conf="$DATADIR/btx.conf" > "$WORK/btxd.log" 2>&1 &
pids+=($!)
for _ in $(seq 1 90); do C getblockchaininfo >/dev/null 2>&1 && break; sleep 1; done
if ! C getblockchaininfo >/dev/null 2>&1; then
  bad "btxd did not come up"; tail -15 "$WORK/btxd.log" | sed 's/^/        /'
  echo; echo "  passed $pass, failed $fail"; exit 1
fi
ok "btxd answers RPC on regtest ($("$BTXD" -version | head -1))"

# The shipped binaries are built with -DENABLE_WALLET=OFF, so there is no
# getnewaddress to mine to. A BTX output is P2MR: witness version 2 with a
# 32-byte program, bech32m over the network's HRP. Encode one directly, using
# the first program from the vendored address oracle.
ADDR=$(python3 - <<'PY'
CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
GEN = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3]
def polymod(v):
    chk = 1
    for x in v:
        b = chk >> 25
        chk = ((chk & 0x1ffffff) << 5) ^ x
        for i in range(5):
            chk ^= GEN[i] if ((b >> i) & 1) else 0
    return chk
def hrp_expand(h):
    return [ord(c) >> 5 for c in h] + [0] + [ord(c) & 31 for c in h]
def convertbits(data, frm, to):
    acc = bits = 0; ret = []; maxv = (1 << to) - 1
    for b in data:
        acc = (acc << frm) | b; bits += frm
        while bits >= to:
            bits -= to; ret.append((acc >> bits) & maxv)
    if bits: ret.append((acc << (to - bits)) & maxv)
    return ret
hrp = "btxrt"
prog = bytes.fromhex("355c704b3ce572331cae5cc322f37b7c3893f1d64dd3a0b35539bd243553f3be")
data = [2] + convertbits(prog, 8, 5)                       # witness version 2
chk = polymod(hrp_expand(hrp) + data + [0] * 6) ^ 0x2bc830a3   # bech32m
print(hrp + "1" + "".join(CHARSET[d] for d in data + [(chk >> 5 * (5 - i)) & 31 for i in range(6)]))
PY
)
if C validateaddress "$ADDR" 2>&1 | grep -q '"isvalid": true'; then
  ok "a P2MR regtest address encodes to something btxd accepts"
else
  bad "btxd rejected the constructed address $ADDR"
  echo; echo "  passed $pass, failed $fail"; exit 1
fi

echo
echo "── a real chain, mined ──"
gen="$(timeout 600 "$BTXCLI" -datadir="$DATADIR" -rpcport="$RPC" generatetoaddress "$BLOCKS" "$ADDR" 2>&1)"
height="$(C getblockcount 2>&1)"
if [ "$height" -ge 1 ] 2>/dev/null; then
  ok "mined $height blocks (regtest needs no GPU for the MatMul proof)"
else
  bad "mining produced no blocks"; printf '%s\n' "$gen" | head -8 | sed 's/^/        /'
  echo; echo "  passed $pass, failed $fail"; exit 1
fi

echo
echo "── electrs indexes it and serves the wallet's routes ──"
# --daemon-dir is the BASE datadir: electrs appends the network directory when
# it looks for the cookie, so passing <datadir>/regtest sends it to
# <datadir>/regtest/regtest/.cookie and it never authenticates.
electrs --network regtest --daemon-dir "$DATADIR" --daemon-rpc-addr "127.0.0.1:$RPC" \
  --db-dir "$DBDIR" --http-addr "127.0.0.1:$EPORT" --electrum-rpc-addr "127.0.0.1:$EELECTRUM" \
  --cors '*' --jsonrpc-import -v > "$WORK/electrs.log" 2>&1 &
pids+=($!)
up=0
for _ in $(seq 1 120); do
  curl -sS -o /dev/null -m 2 "http://127.0.0.1:$EPORT/blocks/tip/height" 2>/dev/null && { up=1; break; }
  sleep 1
done
if [ "$up" -ne 1 ]; then
  bad "electrs never served /blocks/tip/height"
  tail -20 "$WORK/electrs.log" | sed 's/^/        /'
  echo; echo "  passed $pass, failed $fail"; exit 1
fi
etip="$(curl -sS -m 5 "http://127.0.0.1:$EPORT/blocks/tip/height")"
[ "$etip" = "$height" ] && ok "indexed to tip $etip" || bad "electrs tip '$etip', node height '$height'"

# The decisive one: the hash electrs serves at a height must be the hash btxd
# reports there. That is rust-btx decoding real BTX blocks - a 182-byte header
# with a trailing MatMul payload - and agreeing with the node that made them.
ehash="$(curl -sS -m 5 "http://127.0.0.1:$EPORT/block-height/$height" | tr -d '\r\n')"
nhash="$(C getblockhash "$height" | tr -d '\r\n')"
if [ -n "$ehash" ] && [ "$ehash" = "$nhash" ]; then
  ok "/block-height/$height equals btxd's getblockhash (${ehash:0:16}…): the BTX decode is right"
else
  bad "/block-height/$height was '${ehash:0:20}', btxd says '${nhash:0:20}'"
fi

for r in /blocks /mempool; do
  c="$(curl -sS -m 5 -o /dev/null -w '%{http_code}' "http://127.0.0.1:$EPORT$r")"
  [ "$c" = "200" ] && ok "$r -> 200" || bad "$r -> $c"
done

# The address index is the money path: it is what coin selection reads, and
# the defect that retired Byron Bay was here rather than in any block route.
utxo="$(curl -sS -m 10 "http://127.0.0.1:$EPORT/address/$ADDR/utxo")"
n="$(printf '%s' "$utxo" | grep -o '"txid"' | wc -l)"
[ "$n" -ge 1 ] && ok "/address/<addr>/utxo lists $n coinbase output(s): the address index works" \
  || bad "/address/<addr>/utxo returned '${utxo:0:100}'"

echo
echo "── and the front, in front of all of it ──"
if command -v caddy >/dev/null 2>&1 && caddy list-modules 2>/dev/null | grep -qx 'http.handlers.rate_limit'; then
  touch "$RUN/btx-fresh"
  CADDY_ADMIN="${CADDY_ADMIN:-127.0.0.1:12019}" \
  BTX_ESPLORA_HOST="http://127.0.0.1:$FPORT" BTX_ESPLORA_RUN="$RUN" \
  BTX_ESPLORA_ELECTRS="127.0.0.1:$EPORT" BTX_ESPLORA_BTXD_RPC="127.0.0.1:$RPC" \
    caddy run --config "$HERE/Caddyfile.template" --adapter caddyfile > "$WORK/caddy.log" 2>&1 &
  pids+=($!)
  for _ in $(seq 1 40); do
    curl -sS -o /dev/null -m 2 "http://127.0.0.1:$FPORT/blocks/tip/height" 2>/dev/null && break
    sleep 0.5
  done
  ftip="$(curl -sS -m 5 "http://127.0.0.1:$FPORT/blocks/tip/height")"
  [ "$ftip" = "$height" ] && ok "the front serves the same tip, through to electrs" || bad "front tip '$ftip'"
  fr="$(curl -sSI -m 5 "http://127.0.0.1:$FPORT/blocks/tip/height" | tr -d '\r' | grep -i '^x-btx-freshness:' | awk '{print $2}')"
  [ "$fr" = "fresh" ] && ok "X-Btx-Freshness: fresh, from the marker on disk" || bad "freshness was '$fr'"
  co="$(curl -sSI -m 5 "http://127.0.0.1:$FPORT/blocks/tip/height" | tr -d '\r' | grep -ci '^access-control-allow-origin:')"
  [ "$co" = "1" ] && ok "CORS exactly once, end to end" || bad "CORS appeared $co times"
else
  echo "  (no caddy with the rate-limit plugin; front checks skipped)"
fi

echo
echo "──────────────────────────────────────────────"
printf '  passed %d, failed %d\n' "$pass" "$fail"
[ "$fail" -eq 0 ] || exit 1
echo "  The stack works on a real BTX chain. This is NOT an acceptance:"
echo "  scripts/verify-esplora.sh against a full unpruned mainnet node is."
