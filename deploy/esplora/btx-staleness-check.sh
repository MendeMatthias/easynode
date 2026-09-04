#!/bin/bash
# Decide whether this node is serving current data, and ALWAYS make that answer
# visible to callers. Ported into easyNode from the deployment behind
# api.btxscan.io; the reasoning below is that deployment's, kept because it was
# paid for with incidents.
#
# WHY THIS EXISTS
# On 2026-08-10 the MatMul v4.7 fork activated at height 185,000. Validating a
# block past it needs a qualified GPU, and a VM without one froze at 184,999
# while staying fast and healthy and answering 200 with a dead chain. Ordinary
# failover only reacts to a source being DOWN; a stalled node is not down, it is
# a confident wrong answer.
#
# THE BUG THIS REPLACES (2026-08-13): the previous version used one reference,
# explorer.minebtx.com. minebtx went 503, `curl -f` returned an empty body, the
# integer check failed, and the script REMOVED the marker — losing the witness
# was treated as proof of health, and a four-day-old chain was served labelled
# `local`. Worse, the proxy's fallback pointed at the SAME dead host, so
# reference and remedy died together.
#
# THE RULE: only positive evidence of freshness clears the marker. No witness
# means keep the last known state and say `unverified`. Never silently claim to
# be current.
#
# ⚠ THE DEFAULT WITNESS LIST CHANGED ON THE WAY INTO THIS REPOSITORY, and the
# reason is the bug above. The original defaulted to
#     esplora.btxbyronbay.com + explorer.minebtx.com
# and BOTH are now gone: minebtx answers 503, and byronbay is retiring — probed
# 2026-09-04 its tip read 209778 twice, minutes apart, against a network at
# 210266. Shipping those defaults would have recreated the exact failure this
# script exists to prevent, on day one, in every operator's install.
#
# WHAT A WITNESS IS AND IS NOT. byronbay is usable as a freshness witness and
# for block-level reads, and must NEVER be used for /address/* money routes:
# its UTXO index under-reports spends IN THE COMMON RANGE — on one address it
# lists 35 outputs as unspent that a good node proves spent at height 184,472,
# below both tips and below the fork, overstating that balance by ~4,042 BTX.
# Overstated balances reaching a signing wallet are far worse than stale ones.
#
# Never rely on ONE witness. Add more as they come back.
set -u

MARKER_STALE=/run/btx-stale
MARKER_FRESH=/run/btx-fresh
MARKER_UNVERIFIED=/run/btx-unverified

LOCAL_URL=${BTX_LOCAL_URL:-http://127.0.0.1:3000/blocks/tip/height}
# Witnesses, tried in order until one answers with an integer.
REFS=${BTX_REF_URLS:-"https://api.btxscan.io/blocks/tip/height"}
TOLERANCE=${BTX_STALE_TOLERANCE:-3}
# The Epoch A split. A node on the wrong side of this is not stale, it is on a
# different chain, and that is a louder problem than lag.
BRANCH_H=${BTX_BRANCH_CHECK_HEIGHT:-187661}

is_uint() { case "${1:-}" in ('' | *[!0-9]*) return 1 ;; (*) return 0 ;; esac; }

set_state() {
	# Exactly one marker at a time, so the proxy can match on presence.
	rm -f "$MARKER_STALE" "$MARKER_FRESH" "$MARKER_UNVERIFIED"
	touch "$1"
}

LOCAL=$(curl -fsS -m 5 "$LOCAL_URL" 2>/dev/null | tr -d '[:space:]')

REF=""
REF_BASE=""
for u in $REFS; do
	v=$(curl -fsS -m 8 "$u" 2>/dev/null | tr -d '[:space:]')
	if is_uint "$v"; then
		REF="$v"
		REF_BASE="${u%/blocks/tip/height}"
		break
	fi
done

if ! is_uint "$LOCAL"; then
	# Our own node is unreachable. Say so; do not guess.
	set_state "$MARKER_UNVERIFIED"
	echo "state=local-down local=? ref=${REF:-none}"
	exit 0
fi

# Are we even on the same chain? Compare the block HASH at the split height. A
# height alone proves nothing: on 2026-08-24 two mirrors agreed on 199,296 and
# both were wrong.
if [ -n "$REF_BASE" ] && [ "$LOCAL" -ge "$BRANCH_H" ]; then
	ours=$(curl -fsS -m 8 "http://127.0.0.1:3000/block-height/$BRANCH_H" 2>/dev/null | tr -d '[:space:]')
	theirs=$(curl -fsS -m 8 "$REF_BASE/block-height/$BRANCH_H" 2>/dev/null | tr -d '[:space:]')
	if [ -n "$ours" ] && [ -n "$theirs" ] && [ "$ours" != "$theirs" ]; then
		set_state "$MARKER_STALE"
		echo "state=divergent-branch local=$LOCAL ref=$REF split_height=$BRANCH_H ours=$ours theirs=$theirs"
		exit 0
	fi
fi

if ! is_uint "$REF"; then
	# No witness answered. Keep whatever we last knew and say so. This is the
	# line the 2026-08-13 version got wrong by clearing the marker instead.
	set_state "$MARKER_UNVERIFIED"
	echo "state=unverified local=$LOCAL ref=none"
	exit 0
fi

LAG=$((REF - LOCAL))
if [ "$LAG" -gt "$TOLERANCE" ]; then
	set_state "$MARKER_STALE"
	echo "state=stale local=$LOCAL ref=$REF lag=$LAG"
else
	set_state "$MARKER_FRESH"
	echo "state=fresh local=$LOCAL ref=$REF lag=$LAG"
fi
