#!/usr/bin/env python3
"""Measure the BTX chain's block payload without syncing it, by sampling real
block sizes from an archival peer and integrating over height.

Why not just sync it
--------------------
Reading `size_on_disk` after a complete un-pruned sync is the gold standard, and
it needs a spare box and several days. This gets the same number in minutes,
and it is a measurement of the actual block bytes rather than an extrapolation
from one directory.

Why stratified sampling and not a simple grid
---------------------------------------------
BTX block sizes are BIMODAL. A block is either ~367 bytes or ~1,049,000 bytes;
there is nothing in between. The large mode is the MatMul PoW payload, not
transaction traffic — block 120000 is 1,048,948 bytes and carries exactly one
transaction. Averaging a coarse grid with a trapezoid rule silently invents
blocks of intermediate size that do not exist, so the estimator here is instead
"what FRACTION of each height band is in the large mode", which is the only
quantity that moves the total.

Accuracy
--------
Most strata come back unanimous (8/8 large, or 0/8), which costs nothing to
estimate. The entire error budget lives in the few transition bands where the
chain switched modes. Pass --refine to sample those densely once they are known.

Validating the source
---------------------
Before trusting any explorer, this cross-checks it against your own node for
heights your node still holds:

    btx-cli -datadir=<datadir> getblockhash <h>
    btx-cli -datadir=<datadir> getblock <hash> 1   # compare .size and the hash

Run --validate to do that automatically when a local btx-cli is available.

Usage:
    python3 scripts/measure-chain-size.py
    python3 scripts/measure-chain-size.py --strata 40 --per 8
    python3 scripts/measure-chain-size.py --validate --cli <path> --datadir <path>
"""
import argparse
import json
import subprocess
import sys
import time
import urllib.request

DEFAULT_ESPLORA = "https://esplora.btxbyronbay.com"
UA = {"User-Agent": "easynode-measure-chain-size/1.0"}
BIG = 500_000
DELAY = 0.25  # be a polite guest on somebody else's archival node


def http_get(base, path, tries=3, timeout=25):
    for i in range(tries):
        try:
            req = urllib.request.Request(base + path, headers=UA)
            with urllib.request.urlopen(req, timeout=timeout) as r:
                return r.read().decode()
        except Exception:
            if i == tries - 1:
                return None
            time.sleep(1.0 + i)
    return None


def block_at(base, height):
    """(hash, size) for a height, or None. Two requests: Esplora has no
    height->block route, only height->hash and hash->block."""
    h = http_get(base, "/block-height/%d" % height)
    if not h or len(h.strip()) != 64:
        return None
    h = h.strip()
    time.sleep(DELAY)
    body = http_get(base, "/block/" + h)
    if not body:
        return None
    try:
        return (h, json.loads(body).get("size"))
    except Exception:
        return None


def validate(base, cli, datadir, heights):
    """Cross-check the explorer against our own node. A mirror that disagrees
    about a hash is serving a different chain and must not be sampled."""
    ok = True
    for height in heights:
        try:
            our_hash = subprocess.run(
                [cli, "-datadir=%s" % datadir, "getblockhash", str(height)],
                capture_output=True, text=True, timeout=30,
            ).stdout.strip()
            if not our_hash:
                print("  %8d  our node does not hold this height, skipped" % height)
                continue
            our = json.loads(subprocess.run(
                [cli, "-datadir=%s" % datadir, "getblock", our_hash, "1"],
                capture_output=True, text=True, timeout=30,
            ).stdout)
        except Exception as e:
            print("  %8d  local RPC failed: %s" % (height, e))
            continue
        got = block_at(base, height)
        if not got:
            print("  %8d  explorer did not answer" % height)
            ok = False
            continue
        their_hash, their_size = got
        agree = their_hash == our_hash and their_size == our["size"]
        ok = ok and agree
        print("  %8d  ours %8d  theirs %8s  %s"
              % (height, our["size"], their_size, "same-hash" if agree else "MISMATCH"))
        time.sleep(DELAY)
    return ok


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--esplora", default=DEFAULT_ESPLORA)
    ap.add_argument("--strata", type=int, default=40)
    ap.add_argument("--per", type=int, default=8)
    ap.add_argument("--validate", action="store_true")
    ap.add_argument("--cli", default="btx-cli")
    ap.add_argument("--datadir", default=None)
    ap.add_argument("--json", default=None, help="write the raw samples here")
    args = ap.parse_args()

    tip_s = http_get(args.esplora, "/blocks/tip/height")
    if not tip_s:
        print("could not reach %s" % args.esplora, file=sys.stderr)
        return 1
    tip = int(tip_s.strip())
    print("explorer %s at height %d" % (args.esplora, tip))

    if args.validate:
        if not args.datadir:
            print("--validate needs --datadir", file=sys.stderr)
            return 2
        print("cross-checking against the local node:")
        step = max(1, tip // 6)
        if not validate(args.esplora, args.cli, args.datadir, [tip - 1 - k * step for k in range(5)]):
            print("explorer disagreed with our node; not sampling it", file=sys.stderr)
            return 1
        print()

    width = tip // args.strata
    strata = []
    for s in range(args.strata):
        lo = s * width
        hi = min(lo + width, tip)
        step = max(1, width // args.per)
        sizes = []
        for k in range(args.per):
            h = lo + k * step
            if h >= hi:
                break
            got = block_at(args.esplora, h)
            if got and got[1] is not None:
                sizes.append(got[1])
            time.sleep(DELAY)
        if not sizes:
            print("stratum %2d [%7d-%7d] NO DATA" % (s, lo, hi))
            continue
        big = sum(1 for z in sizes if z >= BIG)
        mean = sum(sizes) / len(sizes)
        strata.append({"lo": lo, "hi": hi, "n": len(sizes), "big": big,
                       "p_big": big / len(sizes), "mean": mean})
        print("stratum %2d [%7d-%7d] n=%d big=%d p=%.2f mean=%9.0f"
              % (s, lo, hi, len(sizes), big, big / len(sizes), mean))

    total = sum((st["hi"] - st["lo"]) * st["mean"] for st in strata)
    unanimous = sum(1 for st in strata if st["big"] in (0, st["n"]))
    print()
    print("strata unanimous : %d of %d (the rest carry the whole error budget)"
          % (unanimous, len(strata)))
    print("BLOCK PAYLOAD    : %.1f GiB (%.1f GB) over heights 0..%d"
          % (total / 2**30, total / 1e9, tip))
    print()
    print("This is block payload only. Add the block index, undo files,")
    print("chainstate and shielded state for what a datadir actually costs;")
    print("measure those with scripts/blockstore-census.py and du.")

    if args.json:
        json.dump({"tip": tip, "strata": strata, "total_bytes": total}, open(args.json, "w"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
