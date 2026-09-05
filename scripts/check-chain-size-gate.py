#!/usr/bin/env python3
"""Does the install gate still cover the chain? Compare a fresh measurement
against the constants the app ships.

Why this exists
---------------
DISK_REQUIRED_FRESH was set at 120 GiB from a 105 GB reading on 2026-07-12. By
2026-09-04 the chain was 123.8 GiB. Nothing re-measured, nothing compared, and
for eight weeks the gate was BELOW the chain it exists to gate - waving through
installs that would run out of disk halfway. The number was wrong in seven
places by the time anyone looked.

Fixing the number once does not fix that. This script is the comparison that
was missing, and the chain-size-watch workflow runs it on a schedule.

What it checks
--------------
  1. MEASURED_CHAIN_PAYLOAD_GIB in setup.rs is not stale: the fresh measurement
     must be within --tolerance GiB of it. Above means the constant needs
     re-measuring (and the doc updating); the chain does not shrink, so below
     means the measurement itself is suspect.
  2. DISK_REQUIRED_FRESH still exceeds the measured chain by at least --headroom
     GiB - the working room a fresh install needs on top of the blocks.

Exit 0 when both hold, 1 when either fails, 2 on a usage or parse error. A
measurement that could not be taken is NOT this script's problem: pass it a
real JSON or do not run it.

Usage:
    python3 scripts/measure-chain-size.py --json /tmp/chain.json
    python3 scripts/check-chain-size-gate.py /tmp/chain.json
"""
import argparse
import json
import re
import sys
from pathlib import Path

GIB = 1024 ** 3
SETUP_RS = Path(__file__).resolve().parent.parent / "crates" / "btx-core" / "src" / "setup.rs"


def read_constants(path):
    src = path.read_text(encoding="utf-8")
    m1 = re.search(r"pub const MEASURED_CHAIN_PAYLOAD_GIB:\s*u64\s*=\s*(\d+)\s*;", src)
    m2 = re.search(r"pub const DISK_REQUIRED_FRESH:\s*u64\s*=\s*(\d+)\s*\*\s*1024\s*\*\s*1024\s*\*\s*1024\s*;", src)
    if not m1 or not m2:
        raise SystemExit("could not read MEASURED_CHAIN_PAYLOAD_GIB / DISK_REQUIRED_FRESH from %s" % path)
    return int(m1.group(1)), int(m2.group(1))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("measurement", help="JSON written by measure-chain-size.py --json")
    ap.add_argument("--setup-rs", default=str(SETUP_RS))
    ap.add_argument("--tolerance", type=float, default=4.0,
                    help="GiB the constant may lag the measurement before it is stale")
    ap.add_argument("--headroom", type=float, default=12.0,
                    help="GiB the gate must exceed the measured chain by")
    args = ap.parse_args()

    try:
        m = json.load(open(args.measurement, encoding="utf-8"))
        measured_gib = float(m["total_bytes"]) / GIB
        tip = int(m["tip"])
        strata = m.get("strata", [])
    except Exception as e:
        print("could not read the measurement: %s" % e, file=sys.stderr)
        return 2

    # A sample with holes is not a measurement. measure-chain-size prints
    # NO DATA for a stratum the explorer would not answer; refuse to judge on it.
    if not strata:
        print("the measurement has no strata; refusing to compare against nothing", file=sys.stderr)
        return 2
    unanimous = sum(1 for s in strata if s["big"] in (0, s["n"]))

    constant_gib, gate_gib = read_constants(Path(args.setup_rs))

    print("chain tip                 : %d" % tip)
    print("measured block payload    : %.1f GiB  (%d/%d strata unanimous)" % (measured_gib, unanimous, len(strata)))
    print("MEASURED_CHAIN_PAYLOAD_GIB: %d GiB" % constant_gib)
    print("DISK_REQUIRED_FRESH       : %d GiB" % gate_gib)
    print()

    ok = True
    drift = measured_gib - constant_gib
    if drift > args.tolerance:
        print("STALE: the chain measures %.1f GiB, %.1f GiB above the constant the app ships." % (measured_gib, drift))
        print("       Re-measure with --validate on a box with a node, then update")
        print("       MEASURED_CHAIN_PAYLOAD_GIB and docs/archival-capacity.md together.")
        ok = False
    elif drift < -args.tolerance:
        print("SUSPECT: the chain measures %.1f GiB, BELOW the shipped constant by %.1f GiB." % (measured_gib, -drift))
        print("         Chains do not shrink. The explorer is behind, on a fork, or")
        print("         the sample is wrong. Do not trust this run.")
        ok = False
    else:
        print("constant OK: within %.1f GiB of the measurement" % abs(drift))

    room = gate_gib - measured_gib
    if room < 0:
        print("GATE BELOW THE CHAIN: DISK_REQUIRED_FRESH (%d GiB) is %.1f GiB SMALLER than the" % (gate_gib, -room))
        print("                      chain it gates. Every fresh install it admits will run out")
        print("                      of disk. This is the 2026-07..09 failure, back again. Raise it.")
        ok = False
    elif room < args.headroom:
        print("GATE TOO LOW: DISK_REQUIRED_FRESH (%d GiB) is only %.1f GiB above the chain;" % (gate_gib, room))
        print("              %.0f GiB of working room is the floor. A fresh install can pass" % args.headroom)
        print("              the preflight and run out of disk. Raise the gate.")
        ok = False
    else:
        print("gate OK: %.1f GiB of room above the measured chain" % room)

    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
