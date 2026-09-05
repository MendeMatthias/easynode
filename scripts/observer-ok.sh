#!/usr/bin/env bash
# Exit 0 only when the node observer's last row is fresh and `ok`.
#
# The gate that did not exist on 2026-09-05 (easynode#37): a release was
# signed, published and flipped live from a box whose own node had been
# 150–300 blocks behind a longer chain for hours, while node-observer.sh wrote
# that down every two minutes and nobody read it. The release scripts call
# this first, so a release cannot be cut while the box's own node is on a
# fork, down, silent, or simply unobserved.
#
# What "ok" means is the observer's business (scripts/node-observer.sh): since
# the same day it writes FORK when a headers-only branch outruns the active
# chain or headers outrun blocks for ten minutes. This script only refuses to
# proceed on anything but a fresh `ok`, and says which row it read.
#
# Usage:
#   scripts/observer-ok.sh              exit 0 = the last row is younger than
#                                       BTX_OBSERVER_MAX_AGE (300 s) and `ok`;
#                                       otherwise prints the row and exits 1
#   scripts/observer-ok.sh --self-test  prove it refuses the 2026-09-05 rows
#                                       and passes a healthy one
#
# Environment:
#   BTX_OBSERVER_TSV       the observer's TSV (default ~/node-observer.tsv)
#   BTX_OBSERVER_MAX_AGE   seconds a row may be old (default 300)
#   OBSERVER_OVERRIDE=1    skip the gate. Echoed loudly. For the day the
#                          observer is what is broken, never for the day the
#                          node is.
#
# No `timeout`, no GNU `date -d`: this also runs on the Mac, which has neither.
# python3 does the timestamp arithmetic, as it does in the observer itself.
set -euo pipefail

# check <tsv> <max_age_secs> <now>   (now: unix seconds, an ISO-8601 Z time, or
# "" for the wall clock). Prints one line either way; exit 0 only on a fresh ok.
check() {
  python3 - "$1" "$2" "$3" <<'PY'
import sys, time
from datetime import datetime, timezone

def parse(ts):
    return int(datetime.strptime(ts, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc).timestamp())

tsv, max_age, now = sys.argv[1], int(sys.argv[2]), sys.argv[3]
now = parse(now) if now.endswith("Z") else int(now) if now else int(time.time())
try:
    rows = [l.rstrip("\n") for l in open(tsv, encoding="utf-8") if l.strip()]
except OSError as e:
    print(f"observer-ok: cannot read {tsv}: {e}"); sys.exit(1)
if len(rows) < 2:
    print(f"observer-ok: {tsv} has a header and no rows; is the observer running?"); sys.exit(1)
last = rows[-1]
cols = last.split("\t")
if len(cols) < 8:
    print(f"observer-ok: malformed last row: {last!r}"); sys.exit(1)
try:
    age = now - parse(cols[0])
except ValueError:
    print(f"observer-ok: unreadable timestamp in the last row: {last!r}"); sys.exit(1)
state = cols[7]
if age > max_age:
    print(f"observer-ok: the last row is {age}s old (limit {max_age}s); the observer is not running: {last}"); sys.exit(1)
if state != "ok":
    print(f"observer-ok: the last state is {state!r}, not ok: {last}"); sys.exit(1)
print(f"observer-ok: {cols[0]} blocks={cols[1]} headers={cols[2]} behind={cols[3]} state=ok ({age}s old)")
PY
}

if [ "${1:-}" = "--self-test" ]; then
  t="$(mktemp -d)"; trap 'rm -rf "$t"' EXIT
  hdr=$'utc\tblocks\theaders\tbehind\tpeers\tarchival_at_tip\tstored_attestations\tstate'
  # Real rows from ~/node-observer.tsv on the release box on 2026-09-05
  # (docs/incident-2026-09-05-fork.md). The observer of that day wrote `ok` at
  # 302 behind; with its FORK rule those rows now read FORK, and this gate
  # must refuse them. The healthy row is from the same morning, at the tip.
  printf '%s\n%s\n' "$hdr" $'2026-09-05T18:40:35Z\t210865\t211167\t302\t20\t1\t3700\tFORK' > "$t/fork.tsv"
  printf '%s\n%s\n' "$hdr" $'2026-09-05T19:29:46Z\t210869\t211167\t298\t21\t1\t3703\tFORK' > "$t/fork2.tsv"
  printf '%s\n%s\n' "$hdr" $'2026-09-05T14:22:45Z\t210816\t210816\t0\t20\t1\t3650\tok'    > "$t/healthy.tsv"
  printf '%s\n%s\n' "$hdr" $'2026-09-05T14:22:45Z\t\t\t\t\t\t\tbtxd_down'                  > "$t/down.tsv"
  printf '%s\n' "$hdr" > "$t/empty.tsv"
  printf '%s\n%s\n' "$hdr" 'garbage' > "$t/malformed.tsv"

  fails=0
  expect() {  # <want: pass|fail> <label> <tsv> <now>
    set +e; out="$(check "$3" 300 "$4")"; rc=$?; set -e
    if [ "$1" = pass ] && [ "$rc" -ne 0 ]; then echo "self-test: $2 should PASS, got exit $rc: $out"; fails=1; fi
    if [ "$1" = fail ] && [ "$rc" -eq 0 ]; then echo "self-test: $2 should FAIL, got exit 0: $out"; fails=1; fi
  }
  expect fail "the 18:40Z FORK row"              "$t/fork.tsv"      2026-09-05T18:41:26Z
  expect fail "the 19:29Z FORK row"              "$t/fork2.tsv"     2026-09-05T19:30:00Z
  expect pass "a healthy row, 15 s old"          "$t/healthy.tsv"   2026-09-05T14:23:00Z
  expect fail "a healthy row, 7 min old"         "$t/healthy.tsv"   2026-09-05T14:30:00Z
  expect fail "a btxd_down row"                  "$t/down.tsv"      2026-09-05T14:23:00Z
  expect fail "a header with no rows"            "$t/empty.tsv"     2026-09-05T14:23:00Z
  expect fail "a malformed row"                  "$t/malformed.tsv" 2026-09-05T14:23:00Z
  expect fail "a missing file"                   "$t/nope.tsv"      2026-09-05T14:23:00Z
  if [ "$fails" -eq 0 ]; then echo "self-test: every case behaved"; exit 0; fi
  exit 1
fi

if [ "${OBSERVER_OVERRIDE:-}" = "1" ]; then
  echo "!!! OBSERVER_OVERRIDE=1: SKIPPING the node-observer gate. Nothing is checking that this box's own node is on the live chain. !!!" >&2
  exit 0
fi

TSV="${BTX_OBSERVER_TSV:-$HOME/node-observer.tsv}"
MAX_AGE="${BTX_OBSERVER_MAX_AGE:-300}"
if check "$TSV" "$MAX_AGE" ""; then
  exit 0
fi
echo "observer-ok: refusing. The node observer's last row is not a fresh 'ok'. Fix the node (or start the observer), or set OBSERVER_OVERRIDE=1 knowingly." >&2
exit 1
