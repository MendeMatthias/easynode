#!/bin/bash
# Decide whether this node is serving current data, ON THE CHAIN THE NETWORK IS
# ACTUALLY ON, and ALWAYS make that answer visible to callers. Ported into
# easyNode from the deployment behind api.btxscan.io; the reasoning below is
# that deployment's, kept because it was paid for with incidents, plus two of
# our own.
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
# THE WITNESS CHANGED ON THE WAY INTO THIS REPOSITORY. The original asked
# esplora.btxbyronbay.com and explorer.minebtx.com; both are gone. A first port
# asked api.btxscan.io instead. Then, on 2026-09-05, the network split at height
# 210496 and api.btxscan.io followed the minority branch for a day. As the
# witness it would have called a node on the live chain "stale" (hundreds of
# blocks ahead of btxscan, on a different branch) and a node on the same dead
# branch "fresh". A height from one explorer is not a fact about the network.
#
# THE WITNESS IS THE CENSUS. easybtx.com/api/nodes publishes `chains`: every
# chain any reachable node follows, measured from the nodes' own headers, with
# the one carrying the most work marked `heaviest`, its tip height, a prefix of
# its tip hash, and the height at which it left the others.
#
# WHAT THE CENSUS CAN AND CANNOT WITNESS, measured 2026-09-06 00:00Z. The census
# named chain A, tip 211404 (d5cdc194a5bbc8a7…), heaviest. This project's own
# validator held d5cdc194 at that minute as a `valid-headers` side tip of
# branchlen 1 while its ACTIVE chain ran to 211416, and its block at 211404 was
# a433ed21…, which is what api.btxscan.io serves there. The census's heaviest
# tip was a one-block orphan, and 33 minutes earlier the census had named a
# different chain heaviest whose tip this node also holds as a one-block side
# tip. So the census is a STRONG witness for "this endpoint is on a deep
# minority branch" — the 2026-09-05 shape, forked 389 blocks down for a day —
# and a WEAK one for "this endpoint holds the exact best block", because BTX
# mines races. A first version of these rules called a correct endpoint "on
# another chain" for not holding a one-block orphan; these do not.
#
# THE RULES, in this order. crates/btx-core/src/esplora_freshness.rs implements
# them identically for the easyNode app; change one and change the other.
#   local tip unknown                                   -> unverified
#   no census, older than 30 min, or no heaviest chain
#     with a usable tip                                 -> unverified
#   holds a DEEP competing chain's tip (forked more
#     than RACE_DEPTH below the heaviest tip)           -> unverified (another chain)
#   local tip >= census tip, our block there IS it      -> fresh
#   local tip >= census tip, our block there is not     -> unverified (a race)
#   more than TOLERANCE below the census tip            -> stale
#   within TOLERANCE, not comparable                    -> unverified
#
# The deep-branch test runs FIRST, before any height comparison: an overstated
# balance from the wrong chain reaching a signing wallet is worse than a stale
# one. What would sharpen all of this is `recentHashes` per chain in the public
# feed, a few blocks below each tip where a race has settled; then an endpoint
# could be placed on a chain positively rather than by elimination.
set -u

RUN_DIR="${BTX_ESPLORA_RUN:-/run}"
MARKER_STALE="$RUN_DIR/btx-stale"
MARKER_FRESH="$RUN_DIR/btx-fresh"
MARKER_UNVERIFIED="$RUN_DIR/btx-unverified"

LOCAL_BASE="${BTX_LOCAL_ESPLORA:-http://127.0.0.1:3000}"
CENSUS_URL="${BTX_CENSUS_URL:-https://easybtx.com/api/nodes}"
CENSUS_MAX_AGE="${BTX_CENSUS_MAX_AGE:-1800}"
TOLERANCE="${BTX_STALE_TOLERANCE:-3}"
# Same figure as esplora_freshness.rs::RACE_DEPTH.
RACE_DEPTH="${BTX_RACE_DEPTH:-6}"

set_state() {
	# Exactly one marker at a time, so the proxy can match on presence.
	mkdir -p "$RUN_DIR" 2>/dev/null
	rm -f "$MARKER_STALE" "$MARKER_FRESH" "$MARKER_UNVERIFIED"
	touch "$1"
}

# The decision, in python3 because the census is JSON and jq is not a given.
# Prints "state=<fresh|stale|unverified> ..." on one line and never fails on a
# network error, because a failed run must still write a marker.
line=$(python3 - "$LOCAL_BASE" "$CENSUS_URL" "$CENSUS_MAX_AGE" "$TOLERANCE" "$RACE_DEPTH" <<'PY'
import json, sys, time, urllib.request

local_base, census_url = sys.argv[1], sys.argv[2]
max_age, tol, race_depth = int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5])

def get(url, timeout):
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "easynode-staleness-check"})
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.read().decode("utf-8", "replace").strip()
    except Exception:
        return None

def uint(s):
    return int(s) if s is not None and s.isdigit() else None

def hash_at(h):
    v = get(f"{local_base}/block-height/{h}", 8)
    v = v.lower() if v else None
    return v if v and len(v) == 64 and all(c in "0123456789abcdef" for c in v) else None

def prefix_of(chain):
    p = str(chain.get("tipHash") or "").strip().lower()
    return p if len(p) >= 8 and all(c in "0123456789abcdef" for c in p) else None

def holds_tip(chain):
    """True/False when comparable, None when the served hash could not be read."""
    hh, prefix = chain.get("tipHeight"), prefix_of(chain)
    if hh is None or not prefix:
        return None
    ours = hash_at(int(hh))
    return None if ours is None else ours.startswith(prefix)

def out(state, **kw):
    print("state=" + state + "".join(f" {k}={v}" for k, v in kw.items()))
    sys.exit(0)

local = uint(get(f"{local_base}/blocks/tip/height", 5))
if local is None:
    out("unverified", why="local-down")

raw = get(census_url, 15)
try:
    census = json.loads(raw) if raw else None
except ValueError:
    census = None
if not isinstance(census, dict):
    out("unverified", why="no-census", local=local)
age = int(time.time()) - int(census.get("checkedAt") or 0)
if age > max_age:
    out("unverified", why="census-old", age=age, local=local)
chains = ((census.get("chains") or {}).get("chains") or [])
heaviest = next((c for c in chains if c.get("heaviest")), None)
if not heaviest or heaviest.get("tipHeight") is None or not prefix_of(heaviest):
    out("unverified", why="no-heaviest-chain", local=local)
hh = int(heaviest["tipHeight"])

# FIRST: are we positively on a chain that left the heaviest one deeply? That
# is the 2026-09-05 failure, and it is what the census witnesses well.
for other in chains:
    if other is heaviest or other.get("tipHeight") is None:
        continue
    fork = other.get("forkHeight")
    if fork is None or hh - int(fork) <= race_depth:
        continue  # a race, not a branch
    if int(other["tipHeight"]) <= local and holds_tip(other) is True:
        out("unverified", why="on-competing-chain", chain=other.get("id"),
            forked_at=fork, local=local, census=hh)

if local >= hh:
    match = holds_tip(heaviest)
    if match is True:
        out("fresh", local=local, census=hh, chain=heaviest.get("id"))
    if match is False:
        # NOT an accusation: no deep branch was found, so this is a race the
        # census caught mid-flight. It clears on the next cycle if so.
        out("unverified", why="race-at-census-tip", local=local, census=hh)
    out("unverified", why="no-served-hash", local=local, census=hh)

lag = hh - local
if lag > tol:
    out("stale", local=local, census=hh, lag=lag)
out("unverified", why="behind-not-comparable", local=local, census=hh, lag=lag)
PY
)

case "$line" in
	state=fresh*) set_state "$MARKER_FRESH" ;;
	state=stale*) set_state "$MARKER_STALE" ;;
	*)
		set_state "$MARKER_UNVERIFIED"
		[ -n "$line" ] || line="state=unverified why=decider-failed"
		;;
esac
echo "$line"
