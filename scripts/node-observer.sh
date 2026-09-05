#!/usr/bin/env bash
# Watch a running easyNode node, record what it is doing, and recover the two
# failure modes that actually take a home node off the network.
#
# ── WHY THIS IS A SHELL SCRIPT AND NOT PART OF THE APP ──────────────────────
# `crates/btx-core/src/watchdog.rs` already DIAGNOSES a stall, and it is good at
# it: it separates "the body never arrived" from "the body is banked and the
# attestation is missing", and it refuses to guess when it cannot measure. What
# it does not do is ACT. Nothing in the app restarts a node that died, and
# nothing dials a peer when the chain stops moving.
#
# So the recovery below ran as a private script on one machine for a day and a
# half before anybody wrote it down. It is here so that every operator gets it,
# and so that the behaviour is reviewable before any of it moves into the app,
# where restarting somebody's node without asking is a consent decision rather
# than an engineering one.
#
# ── WHAT IT RECOVERS, BOTH MEASURED RATHER THAN IMAGINED ────────────────────
#
#   1. btxd is not running at all — a WSL restart, or the app standing down.
#      Recovery is to start the app again.
#
#   2. A STALE PID whose number was reused after a reboot. The app then refuses
#      to start its own node forever, because something else now owns that pid.
#      Seen 2026-09-04. Recovery is to move the pid file aside — but ONLY when
#      no btxd is in the process table AND nothing is listening on the RPC port,
#      because clearing a pid file out from under a live node is worse than the
#      bug.
#
#   3. A tip that has not advanced for ~20 minutes while we are behind. That is
#      the body-starvation signature, and BTX has very little archival capacity:
#      of 19 peers on 2026-09-04, twelve advertised NETWORK and exactly ONE was
#      archival AND current (docs/archival-capacity.md). The fix is a peer, not
#      a restart, so this dials one and never bounces the node.
#
# ── WHAT IT ALARMS ON, SINCE 2026-09-05 ─────────────────────────────────────
#
#   4. A FORK. On 2026-09-05 this script wrote `behind` climbing from 2 to 302
#      over four hours, state `ok` on every row, while a release was cut from
#      the same box (docs/incident-2026-09-05-fork.md). A log nobody reads is
#      not an alarm. So the state column now says FORK, and one line goes to
#      stderr, when either holds:
#        - a headers-only branch in `getchaintips` is more than FORK_LEAD (6)
#          blocks longer than the active chain since their common ancestor
#          (the same rule crates/btx-core/src/fork.rs applies in the app), or
#        - headers are more than FORK_BEHIND (20) ahead of blocks for
#          FORK_ROWS (5) consecutive samples, ten minutes at the default
#          interval.
#      Nothing is recovered: which chain is right is not this script's call.
#      scripts/observer-ok.sh reads the state column, and the release scripts
#      refuse to run on anything but a fresh `ok`.
#
# ⚠ IT NEVER KILLS A LIVE btxd. Every recovery path above is gated on the node
# being absent or provably stuck; none of them stops a node that is working.
#
# Usage:
#   scripts/node-observer.sh &                 # uses the defaults below
#   BTX_CLI=... BTX_DATADIR=... scripts/node-observer.sh &
#
# Environment:
#   BTX_CLI        path to btx-cli          (default: the app's install path)
#   BTX_DATADIR    the node datadir         (default: ~/.easybtx)
#   BTX_START_CMD  command to restart the app when btxd is gone; if unset, the
#                  script REPORTS the outage and does not try to fix it
#   BTX_ARCHIVE_PEER  archival peer to dial on a stall (host, no port)
#   BTX_INTERVAL   seconds between samples   (default: 120)
#   BTX_FORK_LEAD    blocks a headers-only branch must lead ours by (default 6)
#   BTX_FORK_BEHIND  headers-minus-blocks that starts the clock (default 20)
#   BTX_FORK_ROWS    consecutive samples over it that mean FORK (default 5)
set -uo pipefail

CLI="${BTX_CLI:-$HOME/.local/btx/v0.34.5/linux-x86_64/bin/btx-cli}"
DD="${BTX_DATADIR:-$HOME/.easybtx}"
INTERVAL="${BTX_INTERVAL:-120}"
# esplora.btxbyronbay.com. Published in crates/btx-core/src/node.rs and in the
# shipped conf; a trusted mirror cannot work without pinning its peers, so this
# is public by necessity rather than by accident.
ARCHIVE_PEER="${BTX_ARCHIVE_PEER:-134.199.150.193}"
ARCHIVE_PORT="${BTX_ARCHIVE_PORT:-19335}"
START_CMD="${BTX_START_CMD:-}"
FORK_LEAD="${BTX_FORK_LEAD:-6}"
FORK_BEHIND="${BTX_FORK_BEHIND:-20}"
FORK_ROWS="${BTX_FORK_ROWS:-5}"

TSV="${BTX_OBSERVER_TSV:-$HOME/node-observer.tsv}"
LOG="${BTX_OBSERVER_LOG:-$HOME/node-observer.log}"
A=(-datadir="$DD")

command -v python3 >/dev/null || { echo "node-observer: python3 is required" >&2; exit 1; }
[ -x "$CLI" ] || { echo "node-observer: no btx-cli at $CLI (set BTX_CLI)" >&2; exit 1; }

[ -f "$TSV" ] || printf 'utc\tblocks\theaders\tbehind\tpeers\tarchival_at_tip\tstored_attestations\tstate\n' > "$TSV"
say() { printf '%s %s\n' "$(date -u +%FT%TZ)" "$*" >> "$LOG"; }

# Count peers that both advertise NETWORK and are at or above our own tip.
# `synced_headers` tracks live; `startingheight` freezes at the handshake and
# goes stale as we advance, which made an earlier version of this read 0 while
# the peers were in fact fine. Take the better of the two.
archival_at_tip() {
  "$CLI" "${A[@]}" getpeerinfo 2>/dev/null | python3 -c '
import sys, json
try:
    peers = json.load(sys.stdin)
except Exception:
    print(0); raise SystemExit
tip = int(sys.argv[1])
n = 0
for p in peers:
    h = max(p.get("synced_headers") or -1, p.get("startingheight") or -1)
    if "NETWORK" in (p.get("servicesnames") or []) and h >= tip:
        n += 1
print(n)
' "${1:-0}" 2>/dev/null || echo 0
}

# The largest lead of a headers-only branch over the active chain since their
# common ancestor, or 0. Same arithmetic as crates/btx-core/src/fork.rs: a
# branch whose fork point is at or past our tip merely extends our chain and is
# lag, not a fork; a branch shorter than ours since the split is a stale
# sibling. Only `headers-only` and `valid-headers` count: those are the tips
# whose bodies this node has never had.
fork_lead() {
  "$CLI" "${A[@]}" getchaintips 2>/dev/null | python3 -c '
import sys, json
try:
    tips = json.load(sys.stdin)
except Exception:
    print(0); raise SystemExit
active = [t for t in tips if t.get("status") == "active"]
if not active:
    print(0); raise SystemExit
active = int(active[0]["height"])
best = 0
for t in tips:
    if t.get("status") not in ("headers-only", "valid-headers"):
        continue
    fork = int(t["height"]) - int(t["branchlen"])
    if fork >= active:
        continue
    best = max(best, int(t["branchlen"]) - (active - fork))
print(best)
' 2>/dev/null || echo 0
}

json_int() { python3 -c '
import sys, json
try:
    print(json.load(sys.stdin).get(sys.argv[1], ""))
except Exception:
    print("")
' "$1" 2>/dev/null; }

last_blocks=""; stuck=0; behind_rows=0; fork_said=0
while true; do
  now="$(date -u +%FT%TZ)"; state=ok

  if ! pgrep -x btxd >/dev/null 2>&1; then
    state=btxd_down
    # Only touch pid files when nothing is listening either. A btxd that is
    # alive but momentarily missing from a pgrep race must never be "recovered".
    if ! ss -tln 2>/dev/null | grep -q ':19334'; then
      for f in btxd.pid easybtx-node.pid; do
        if [ -f "$DD/$f" ]; then
          mv "$DD/$f" "$DD/$f.stale-$(date -u +%Y%m%d%H%M%S)"
          say "cleared stale $f (no btxd in the process table, nothing on 19334)"
        fi
      done
      if [ -n "$START_CMD" ]; then
        say "btxd down -> $START_CMD"
        bash -c "$START_CMD" >> "$LOG" 2>&1
        state=restarted
      else
        say "btxd down and BTX_START_CMD is unset -> reporting only"
      fi
    else
      say "btxd absent from the process table but 19334 is listening; leaving it alone"
    fi
    printf '%s\t\t\t\t\t\t\t%s\n' "$now" "$state" >> "$TSV"
    sleep "$INTERVAL"; continue
  fi

  ci="$("$CLI" "${A[@]}" getblockchaininfo 2>/dev/null)"
  if [ -z "$ci" ]; then
    say "btxd is up but RPC is silent"
    printf '%s\t\t\t\t\t\t\trpc_silent\n' "$now" >> "$TSV"
    sleep "$INTERVAL"; continue
  fi

  blocks="$(printf '%s' "$ci" | json_int blocks)"
  headers="$(printf '%s' "$ci" | json_int headers)"
  # Derived from two fields that are always there, rather than read from
  # `behind_best_header`. That field does exist on 0.34.6 — checked — but the
  # earlier version of this script read it with a grep that returned empty on
  # any engine that omitted or renamed it, and empty then became 0 through a
  # default, which reads as "healthy" at exactly the moment it is not.
  # Subtracting two numbers we already have cannot fail that way.
  behind=$(( ${headers:-0} - ${blocks:-0} ))
  peers="$("$CLI" "${A[@]}" getconnectioncount 2>/dev/null)"
  arch="$(archival_at_tip "${blocks:-0}")"
  att="$("$CLI" "${A[@]}" getmatmultrustedstatus 2>/dev/null | json_int stored_attestations)"

  if [ "$blocks" = "$last_blocks" ]; then stuck=$((stuck+1)); else stuck=0; fi
  if [ "$behind" -gt "$FORK_BEHIND" ]; then behind_rows=$((behind_rows+1)); else behind_rows=0; fi
  last_blocks="$blocks"

  # Ten samples with no new block AND measurably behind. At the default
  # interval that is ~20 minutes, far outside a slow block and early enough to
  # beat the operator to the question.
  if [ "$stuck" -ge 10 ] && [ "$behind" -gt 2 ]; then
    state=stalled_no_progress
    if [ "${arch:-0}" -eq 0 ]; then
      say "STALL at $blocks, $behind behind, NO archival peer at or above our tip -> dialling $ARCHIVE_PEER"
      "$CLI" "${A[@]}" addnode "$ARCHIVE_PEER:$ARCHIVE_PORT" onetry >/dev/null 2>&1
      state=stalled_dialled_archive
    else
      # Worth distinguishing: with an archival peer present and the tip still
      # frozen, dialling another one is not the fix and would hide the real
      # cause. watchdog.rs is the thing that can tell you what it is.
      say "STALL at $blocks with ${arch} archival peer(s) present; not a peer problem"
    fi
    stuck=0
  fi

  # ── FORK (easynode#37) ─────────────────────────────────────────────────────
  # Overrides ok and the stall states: a stall on a fork is a fork first. Said
  # on stderr when it starts and every 15 samples (30 minutes) while it holds,
  # so a long one is not a single line lost in scrollback.
  lead="$(fork_lead)"
  if [ "${lead:-0}" -gt "$FORK_LEAD" ] || [ "$behind_rows" -ge "$FORK_ROWS" ]; then
    state=FORK
    if [ $((fork_said % 15)) -eq 0 ]; then
      msg="FORK: blocks $blocks, headers $headers, behind $behind for $behind_rows sample(s); longest branch this node cannot obtain leads ours by $lead"
      say "$msg"
      printf 'node-observer: %s\n' "$msg" >&2
    fi
    fork_said=$((fork_said+1))
  else
    [ "$fork_said" -gt 0 ] && say "fork condition cleared at blocks $blocks, headers $headers"
    fork_said=0
  fi

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$now" "$blocks" "$headers" "$behind" "${peers:-}" "${arch:-0}" "${att:-}" "$state" >> "$TSV"
  sleep "$INTERVAL"
done
