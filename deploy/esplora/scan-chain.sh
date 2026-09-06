#!/usr/bin/env bash
# Full-chain decoder validation: prove rust-btx decodes EVERY mainnet block
# byte-exactly (decode + byte-identical re-encode + txids computed) before
# letting electrs index. Ported from btx-esplora; the logic is unchanged, the
# paths come from the environment instead of one host's layout.
#
# Run it on the machine that holds the UNPRUNED datadir. Sequential,
# resumable, logs failures and keeps going, so one run reveals ALL problem
# blocks rather than the first.
#
# Usage: scan-chain.sh [start_height] [end_height]
# Environment:
#   BTX_CLI    the btx-cli invocation, e.g. "btx-cli -datadir=$HOME/.easybtx"
#   DECODER    rust-btx's decode_block example binary; build it with
#              (cd deploy/esplora/rust-btx && cargo build --release --examples)
#   OUT_DIR    where the log and the failure list go (default: .)
set -u
BCLI="${BTX_CLI:-btx-cli}"
HERE="$(cd "$(dirname "$0")" && pwd)"
DEC="${DECODER:-$HERE/rust-btx/target/release/examples/decode_block}"
OUT="${OUT_DIR:-.}"
[ -x "$DEC" ] || {
  echo "decoder not found at $DEC (build it: cd $HERE/rust-btx && cargo build --release --examples)" >&2
  exit 2
}
START="${1:-0}"
TIP=$($BCLI getblockcount) || { echo "btx-cli did not answer getblockcount" >&2; exit 2; }
END="${2:-$TIP}"
FAILS="$OUT/scan-chain-failures-${START}-${END}.txt"
LOG="$OUT/scan-chain-${START}-${END}.log"
ERR="$(mktemp)"
trap 'rm -f "$ERR"' EXIT
: > "$FAILS"
echo "$(date -u +%FT%TZ) scanning $START..$END (tip $TIP)" | tee -a "$LOG"
fail_count=0
for ((h=START; h<=END; h++)); do
  HASH=$($BCLI getblockhash "$h" 2>/dev/null) || { echo "$h GETHASH_FAIL" >> "$FAILS"; fail_count=$((fail_count+1)); continue; }
  DECODED=$($BCLI getblock "$HASH" 0 2>/dev/null | "$DEC" 2>"$ERR") || {
    echo "$h $HASH $(tr '\n' ' ' < "$ERR" | head -c 300)" >> "$FAILS"
    fail_count=$((fail_count+1)); continue; }
  # sampled deep check: every 1000th block, our computed txids must equal the node's
  if (( h % 1000 == 0 )); then
    OURS=$(echo "$DECODED" | cut -d' ' -f3- | tr ' ' '\n' | sort)
    NODES=$($BCLI getblock "$HASH" 1 2>/dev/null | python3 -c 'import sys,json; [print(t) for t in json.load(sys.stdin)["tx"]]' | sort)
    if [ "$OURS" != "$NODES" ]; then
      echo "$h $HASH TXID_MISMATCH (ours vs node)" >> "$FAILS"; fail_count=$((fail_count+1))
    fi
  fi
  if (( h % 5000 == 0 )); then
    echo "$(date -u +%FT%TZ) at height $h/$END, failures so far: $fail_count" | tee -a "$LOG"
  fi
done
echo "$(date -u +%FT%TZ) DONE $START..$END — failures: $fail_count" | tee -a "$LOG"
if (( fail_count > 0 )); then echo "SEE $FAILS"; exit 1; fi
echo "CHAIN CLEAN: every block decodes + re-encodes byte-exactly"
