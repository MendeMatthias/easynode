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
#
# ⚠ EVERY ONE OF THESE IS ANCIENT, AND THAT WAS A HOLE. The highest fixed
# height here is 205000. The 2026-09-05 split forked at 210496 and a mirror sat
# on the losing side of it for a day. Two endpoints on opposite sides of that
# split agree at every height in this list, because all of them are below the
# fork — so this section would have printed eight PASSes for an endpoint on a
# dead branch. Below a fork point every chain is byte-identical; that is the
# whole reason the census publishes settled hashes above one.
#
# So a RECENT height is added at run time, from the census, and the fixed list
# stays for the old reorg window it documents. The census check below is the
# one that places the endpoint on a chain; this makes the height list stop
# implying a coverage it never had.
WITNESS_HEIGHTS="${WITNESS_HEIGHTS:-150000 180000 187660 187661 187662 190000 199297 205000}"
# Filled from the census once it is read, so the loop also asks about a height
# ABOVE the most recent fork the network has seen.
RECENT_WITNESS=""

pass=0; fail=0; futurefail=0
ok()   { printf '  \033[32mPASS\033[0m  %s\n' "$*"; pass=$((pass+1)); }
bad()  { printf '  \033[31mFAIL\033[0m  %s\n' "$*"; fail=$((fail+1)); }
info() { printf '        %s\n' "$*"; }

# A capability the wallet does not have yet. Kept because this line has moved in
# both directions and will move again: the rule is that the gate requires
# exactly what the wallet actually calls, read from its own egress validator,
# and reports anything else separately so it cannot refuse a wallet-fit
# endpoint for a capability nobody can use.
#
# ⚠ /blocks and /block-height were reported here until 2026-09-06, because
# validate_esplora_route DENIED them. It does not any more (pq-wallet@169b413:
# both are permitted, is_height_segment gates the height, and the chain-health
# feature calls /blocks/tip/height then /block-height/<h> against a witness).
# They are required checks below.
future() { printf '  \033[33mFUTURE\033[0m  %s\n' "$*"; futurefail=$((futurefail+1)); }

get()  { curl -sS --max-time "$TIMEOUT" "$1" 2>/dev/null; }
code() { curl -sS --max-time "$TIMEOUT" -o /dev/null -w '%{http_code}' "$1" 2>/dev/null; }

echo "candidate : $CAND"
echo "reference : $REF"

# ── the candidate must not BE the reference ─────────────────────────────────
# The UTXO set comparison is the check this whole script exists for, and it is
# a comparison. Point it at one host twice and it agrees with itself perfectly,
# every time, and the script prints "may be advertised to a wallet". That is a
# silent vacuous pass on the one check that catches a Byron-Bay-class index.
#
# It is not hypothetical: the first run of this gate in this repository was
# exactly that, because there was no second endpoint to compare against yet.
# SELF_COMPARE=1 allows it for exercising the script, and says so in the
# verdict so nobody quotes that run as an acceptance.
selfcmp=0
if [ "$CAND" = "$REF" ]; then
  if [ "${SELF_COMPARE:-}" = "1" ]; then
    selfcmp=1
    echo
    echo "⚠ SELF_COMPARE=1: candidate and reference are the SAME host. The set"
    echo "  comparison below compares this endpoint with itself and cannot fail."
    echo "  This run exercises the script. It accepts nothing."
  else
    echo
    echo "ABORT: the candidate and the reference are the same host ($CAND)." >&2
    echo "       The UTXO set comparison would compare it with itself, agree, and" >&2
    echo "       report a pass that means nothing. Give REF_API a DIFFERENT known-good" >&2
    echo "       endpoint, or set SELF_COMPARE=1 if you are only exercising this script." >&2
    exit 2
  fi
fi
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

# ── 0b. the reference must be on the chain the network is on ────────────────
# An explorer is one server. On 2026-09-05 api.btxscan.io sat on a minority
# branch for a day; a UTXO comparison against it would have blessed a wrong
# candidate and refused a right one. So before anything is compared, ask the
# chain census (easybtx.com/api/nodes: which chain carries the most work,
# measured from every reachable node's headers) for the heaviest chain's tip
# and check that the reference holds it. No census: a warning, and the tip
# checks fall back to the reference alone, said out loud. A reference on
# another chain: ABORT, unless REF_ALLOW_OFFCHAIN=1 says you know.
CENSUS_URL="${CENSUS_URL:-https://easybtx.com/api/nodes}"
# How far below the heaviest tip a competing chain must fork before holding its
# tip means a different chain rather than the losing side of a race. Same
# figure as crates/btx-core/src/esplora_freshness.rs::RACE_DEPTH.
RACE_DEPTH="${RACE_DEPTH:-6}"
census_tip=""; census_prefix=""; census_chain=""; census_deep=""
census_json="$(curl -sS --max-time "$TIMEOUT" -A "verify-esplora" "$CENSUS_URL" 2>/dev/null)"
if [ -n "$census_json" ]; then
  census_read="$(printf '%s' "$census_json" | python3 -c '
import json, sys, time
try:
    d = json.load(sys.stdin)
except Exception:
    sys.exit(0)
if time.time() - int(d.get("checkedAt") or 0) > 1800:
    sys.exit(0)
chains = (d.get("chains") or {}).get("chains") or []
heavy = next((c for c in chains if c.get("heaviest") and c.get("tipHeight") is not None and c.get("tipHash")), None)
if not heavy:
    sys.exit(0)
depth = int(sys.argv[1])
print(heavy["tipHeight"], str(heavy["tipHash"]).lower(), heavy.get("id") or "?")
# Every OTHER chain that forked more than RACE_DEPTH below the heaviest tip:
# holding one of these tips is being on another chain, not losing a race.
for c in chains:
    if c is heavy or c.get("tipHeight") is None or not c.get("tipHash"):
        continue
    f = c.get("forkHeight")
    if f is not None and int(heavy["tipHeight"]) - int(f) > depth:
        print(c["tipHeight"], str(c["tipHash"]).lower(), c.get("id") or "?", f)
' "$RACE_DEPTH" 2>/dev/null)"
  read -r census_tip census_prefix census_chain <<<"$(printf '%s' "$census_read" | head -1)"
  census_deep="$(printf '%s' "$census_read" | tail -n +2)"
  # A settled height from the heaviest chain: recent, above every fork the
  # census knows, and therefore the only height in the whole comparison that
  # can tell two sides of a current split apart.
  RECENT_WITNESS="$(printf '%s' "$census_json" | python3 -c '
import json, sys
try:
    d = json.load(sys.stdin)
except Exception:
    sys.exit(0)
for c in ((d.get("chains") or {}).get("chains") or []):
    if c.get("heaviest"):
        for b in reversed(c.get("settled") or []):
            if isinstance(b.get("height"), int):
                print(b["height"]); sys.exit(0)
' 2>/dev/null)"
fi

# Does the endpoint at $1 positively serve a DEEP competing chain's tip? Echoes
# a description when it does, nothing when it does not.
on_deep_branch() {
  local base="$1" h prefix id fork got
  [ -n "$census_deep" ] || return 0
  while read -r h prefix id fork; do
    [ -n "$h" ] || continue
    got="$(get "$base/block-height/$h" | tr -d '\r\n' | tr 'A-F' 'a-f')"
    case "$got" in
      "$prefix"*) echo "chain $id, which left the heaviest chain at height $fork (its tip $h is ${prefix}…)"; return 0 ;;
    esac
  done <<<"$census_deep"
}
if [ -n "$census_tip" ]; then
  echo "census    : heaviest chain $census_chain, tip $census_tip (${census_prefix}…)"
  [ -n "$census_deep" ] && echo "          : $(printf '%s\n' "$census_deep" | grep -c .) competing chain(s) forked more than $RACE_DEPTH blocks down"
  # The decisive question is the deep one. A mismatch at the heaviest tip is
  # NOT: measured 2026-09-06 00:00Z, the census's heaviest tip was a one-block
  # orphan that this project's own validator held as a side tip while its
  # active chain ran twelve blocks past it, and api.btxscan.io served the same
  # block there as the validator. An earlier version of this check aborted on
  # exactly that, which would have refused a correct reference.
  ref_deep="$(on_deep_branch "$REF")"
  if [ -n "$ref_deep" ]; then
    echo "ABORT: reference $REF serves $ref_deep" >&2
    echo "       Its UTXO sets would bless the wrong chain." >&2
    if [ "${REF_ALLOW_OFFCHAIN:-}" = "1" ]; then
      echo "       REF_ALLOW_OFFCHAIN=1: continuing against an off-chain reference, as instructed." >&2
    else
      exit 3
    fi
  else
    ref_at="$(get "$REF/block-height/$census_tip" | tr -d '\r\n' | tr 'A-F' 'a-f')"
    case "$ref_at" in
      "$census_prefix"*) echo "reference holds the heaviest measured chain's tip" ;;
      '') echo "note: reference does not answer /block-height/$census_tip; it is on no deep branch, which is what matters here" ;;
      *)  echo "note: reference serves ${ref_at:0:16}… at $census_tip, the census ${census_prefix}… — a mining race the census caught mid-flight, not a branch (no deep divergence found)" ;;
    esac
  fi
else
  echo "WARN: the chain census could not be read; tip checks fall back to the reference alone, which is one explorer"
fi
echo

echo "── routes answer at the ORIGIN ROOT, with no /api prefix ──"
# The wallet's chain() builds URLs against the origin root. A node that serves
# these under /api answers nothing the wallet will ever ask for.
cand_tip="$(get "$CAND/blocks/tip/height")"
case "$cand_tip" in
  ''|*[!0-9]*) bad "/blocks/tip/height must be a bare decimal integer (got: ${cand_tip:0:60})" ;;
  *)           ok  "/blocks/tip/height -> $cand_tip"
               if [ -n "$census_tip" ]; then
                 # The census is a snapshot a few minutes old, so a live node
                 # is normally AT or AHEAD of it; behind by more than a few
                 # blocks is the problem. Chain identity is checked below.
                 if [ "$cand_tip" -ge "$census_tip" ]; then info "at or ahead of the census tip ($census_tip)"
                 elif [ $(( census_tip - cand_tip )) -le 6 ]; then info "within $(( census_tip - cand_tip )) of the census tip"
                 else bad "$(( census_tip - cand_tip )) blocks behind the heaviest measured chain's tip ($census_tip)"; fi
               else
                 d=$(( ref_tip > cand_tip ? ref_tip - cand_tip : cand_tip - ref_tip ))
                 # A few blocks is normal: the attested tip legitimately trails the
                 # mined tip. Tens of blocks is not.
                 if [ "$d" -le 6 ]; then info "within $d of the reference"
                 else bad "$d blocks from the reference tip ($ref_tip)"; fi
               fi ;;
esac

# /blocks is a REQUIRED route since 2026-09-06. The wallet reads the recent
# block listing to judge how old the chain's newest block is, which is the
# signal that caught a source answering 200 from a chain that had stopped 29.5
# hours earlier. An endpoint that 404s it (Byron Bay does) breaks that reading.
blocks_code="$(code "$CAND/blocks")"
if [ "$blocks_code" = "200" ]; then
  n=$(get "$CAND/blocks" | grep -o '"height"' | wc -l)
  if [ "$n" -ge 1 ]; then ok "/blocks -> 200 with $n entries"
  else bad "/blocks returned 200 but no blocks: the wallet cannot read the chain's age from it"; fi
else
  bad "/blocks -> $blocks_code: the wallet reads this route for the chain's age (esplora.btxbyronbay.com 404s it, which is why it fails here)"
fi

for r in "/mempool"; do
  c="$(code "$CAND$r")"
  [ "$c" = "200" ] && ok "$r -> 200" || bad "$r -> $c"
done

echo
echo "── the witness route: /block-height/<h> ──"
# This is what lets the fleet replace Byron Bay as a fork witness, and the wallet
# calls it today. A height alone proves nothing: on 2026-08-24 two mirrors agreed
# on 199,296 and both were wrong.
if [ -n "$census_tip" ]; then
  cand_deep="$(on_deep_branch "$CAND")"
  if [ -n "$cand_deep" ]; then
    bad "this endpoint serves $cand_deep"
    info "every balance it serves is a balance on that chain, and a wallet cannot tell."
    info "This is the 2026-09-05 shape: a mirror that stayed on a branch for a day."
  else
    a="$(get "$CAND/block-height/$census_tip" | tr -d '\r\n' | tr 'A-F' 'a-f')"
    case "$a" in
      "$census_prefix"*) ok "/block-height/$census_tip is the heaviest measured chain's tip (${census_prefix}…)" ;;
      '') future "/block-height/$census_tip not served: this endpoint cannot be placed on a chain positively (it is on no deep branch)" ;;
      *)  # Not a match and not empty. It might be a mining race, or it might
          # be an error body: electrs answers a height it does not have with a
          # 404 whose body is text, and `curl` without -f prints that body, so
          # "not the expected prefix" covered both. Only something that LOOKS
          # like a block hash is evidence of a race.
          case "$a" in
            [0-9a-f]*)
              if [ "${#a}" -eq 64 ]; then
                ok "on no deep branch: not on any chain the census says forked more than $RACE_DEPTH blocks down"
                info "it serves ${a:0:16}… at $census_tip where the census has ${census_prefix}…, which is a mining race, not a divergence"
              else
                bad "/block-height/$census_tip returned ${#a} hex characters, not a 64-character block hash: '${a:0:60}'"
              fi ;;
            *)  bad "/block-height/$census_tip did not return a block hash: '${a:0:60}'" ;;
          esac ;;
    esac
  fi
fi
wfail=0
if [ -n "$RECENT_WITNESS" ]; then
  info "adding height $RECENT_WITNESS from the census: every fixed height above is below the 2026-09-05 fork at 210496"
  WITNESS_HEIGHTS="$WITNESS_HEIGHTS $RECENT_WITNESS"
else
  info "no recent height available from the census; the fixed heights below are ALL under the 2026-09-05 fork at 210496 and cannot tell two sides of a current split apart"
fi
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

    # Prove each claimed-unspent output really is unspent — ASKING THE
    # REFERENCE, not the candidate.
    #
    # This queried $CAND until 2026-09-06, under a comment that said
    # "independent of the reference". It was not independent of anything: it
    # asked the same host, backed by the same index, to corroborate the list
    # that index had just produced.
    #
    # For the exact defect this gate exists to catch, the two routes cannot
    # disagree. In the vendored electrs, `utxo_delta` drops an outpoint on the
    # Spending history row and `lookup_spend` reads the TxEdgeRow, and
    # `index_transaction` writes both in the same batch from the same loop. A
    # spending transaction that was never indexed leaves NEITHER, so /utxo
    # lists the output as unspent and /outspend answers from
    # `impl Default for SpendingValue` — `spent: false`, HTTP 200. Byron Bay's
    # index did precisely that on 116 outputs. The loop therefore reached
    # bad_spend=0 every time and printed PASS, sometimes on the very line after
    # the set comparison had printed FAIL for the same address.
    outspend="$(python3 - "$ca" <<'PY'
import json, sys
try: print(" ".join("%s:%d" % (o["txid"], o["vout"]) for o in json.loads(sys.argv[1])[:25]))
except Exception: pass
PY
)"
    # An address whose UTXO set is EMPTY on both sides agrees trivially and
    # then skips this loop entirely, so it scored two passes while proving
    # nothing. The gate asks for addresses WITH SPEND HISTORY for a reason; say
    # so rather than banking the pass.
    if [ -z "$outspend" ]; then
      bad "$addr: no unspent outputs to prove. Pass an address that HOLDS coins and has spend history; an empty set agrees with anything."
      continue
    fi
    bad_spend=0; checked=0
    for op in $outspend; do
      t="${op%%:*}"; v="${op##*:}"
      sp="$(get "$REF/tx/$t/outspend/$v")"
      checked=$((checked+1))
      case "$sp" in
        *'"spent":true'*)  bad_spend=$((bad_spend+1)) ;;
        *'"spent":false'*) : ;;
        *) bad_spend=$((bad_spend+1)) ;;
      esac
    done
    if [ "$checked" -gt 0 ]; then
      [ "$bad_spend" -eq 0 ] \
        && ok "$addr: the reference agrees all $checked sampled outputs are unspent" \
        || bad "$addr: $bad_spend of $checked outputs the candidate calls unspent are spent, or unprovable, at the reference"
    fi
  done
fi

echo
echo "── POST /tx round-trips, without moving funds ──"
# Re-broadcast a transaction that is already in a block. A working endpoint
# parses it, reaches the node, and returns the node's refusal. A broken one
# 404s, 405s, or times out. Nothing is spent either way.
# The probe body is the fixed literal "00" - it never needed a real txid, and
# deriving one from /blocks meant an endpoint that does not serve /blocks (which
# the wallet cannot call anyway) silently SKIPPED the check for a route the
# wallet DOES require. POST /tx is one of the eight; prove it unconditionally.
postout="$(mktemp)"
trap 'rm -f "$postout"' EXIT
post_code="$(curl -sS --max-time "$TIMEOUT" -o "$postout" -w '%{http_code}' \
    -X POST --data-binary "00" "$CAND/tx" 2>/dev/null)"
body="$(head -c 120 "$postout" 2>/dev/null | tr -d '\n')"
case "$post_code" in
  400|422|500) ok "POST /tx reached the node and it rejected malformed hex ($post_code)"
               info "${body:0:100}" ;;
  404|405)     bad "POST /tx -> $post_code: the route is not served at all" ;;
  200)         bad "POST /tx accepted '00' as a transaction, which is wrong" ;;
  *)           bad "POST /tx -> $post_code (${body:0:80})" ;;
esac

echo
echo "──────────────────────────────────────────────"
printf '  passed %d, failed %d' "$pass" "$fail"
[ "$futurefail" -gt 0 ] && printf ', %d future-capability note(s)' "$futurefail"
printf '\n'
if [ "$fail" -eq 0 ]; then
  if [ "$selfcmp" -eq 1 ]; then
    echo "  NOT AN ACCEPTANCE: the candidate was compared with itself"
    echo "  (SELF_COMPARE=1), so the UTXO set check could not fail. Re-run"
    echo "  against a different known-good reference before advertising this."
  else
    echo "  This endpoint may be advertised to a wallet."
  fi
  if [ "$futurefail" -gt 0 ]; then
    echo "  Some capability notes above are not failures; read them."
  fi
  echo "  Advertising it is a second step: the wallet reaches an endpoint only"
  echo "  through its curated lists, so the origin must also be added there"
  echo "  (pq-wallet CHAIN_WITNESSES, or OFFICIAL_EXPLORERS +"
  echo "  PRODUCTION_EXPLORER_ORIGINS for the money routes) and released."
  exit 0
fi
echo "  DO NOT advertise this endpoint to a wallet."
exit 1
