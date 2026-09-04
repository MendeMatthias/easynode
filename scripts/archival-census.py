#!/usr/bin/env python3
"""Count how many connected peers can actually serve you a block body.

`NETWORK` means "I keep the whole chain" and `NETWORK_LIMITED` means "I keep the
last few hundred blocks". Neither says whether the peer is on your chain. A node
that has fallen behind can only be rescued by a peer that is BOTH archival AND
current, and on BTX most of the archival capacity is on the wrong side of the
MatMul v4.7 fork: it advertises `NETWORK` and has never synced a header with you.

Currency needs a stated threshold or the same peer set gives different answers.
The default is 20 blocks, about half an hour at 90 s a block. Whatever you use,
print your own height beside the count, which is what --json emits.

Read-only: one `getpeerinfo` and one `getblockcount`.

Usage:
    python3 scripts/archival-census.py --datadir ~/.easybtx
    python3 scripts/archival-census.py --datadir ~/.easybtx --within 6 --json
"""
import argparse
import collections
import json
import subprocess
import sys


def cli(binary, datadir, *args):
    out = subprocess.run(
        [binary, "-datadir=%s" % datadir, *args],
        capture_output=True, text=True, timeout=60,
    )
    if out.returncode != 0:
        raise SystemExit("btx-cli failed (%d): %s" % (out.returncode, out.stderr.strip()))
    return out.stdout


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cli", default="btx-cli")
    ap.add_argument("--datadir", required=True)
    ap.add_argument("--within", type=int, default=20,
                    help="a peer is current if synced_headers is within this many blocks")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()

    height = int(cli(args.cli, args.datadir, "getblockcount").strip())
    peers = json.loads(cli(args.cli, args.datadir, "getpeerinfo"))

    def svc(p):
        return p.get("servicesnames", []) or []

    archival = [p for p in peers if "NETWORK" in svc(p)]
    limited = [p for p in peers if "NETWORK" not in svc(p) and "NETWORK_LIMITED" in svc(p)]
    no_headers = [p for p in archival if (p.get("synced_headers") or -1) <= 0]
    current = [p for p in archival
               if 0 < (p.get("synced_headers") or -1) and height - p["synced_headers"] <= args.within]

    result = {
        "our_height": height,
        "within": args.within,
        "peers": len(peers),
        "archival": len(archival),
        "limited_only": len(limited),
        "neither": len(peers) - len(archival) - len(limited),
        "archival_no_headers": len(no_headers),
        "archival_and_current": len(current),
        "archival_and_current_peers": [
            {"addr": p.get("addr"), "subver": p.get("subver"), "headers": p.get("synced_headers")}
            for p in current
        ],
        "versions": dict(collections.Counter(p.get("subver", "?") for p in peers)),
    }

    if args.json:
        json.dump(result, sys.stdout, indent=1)
        print()
        return 0

    print("our height              : %d" % height)
    print("peers connected         : %d" % len(peers))
    print("  advertise NETWORK     : %d  (of which %d have synced no headers with us)"
          % (len(archival), len(no_headers)))
    print("  NETWORK_LIMITED only  : %d" % len(limited))
    print("  neither               : %d" % (len(peers) - len(archival) - len(limited)))
    print("ARCHIVAL AND CURRENT    : %d   (current = within %d blocks)"
          % (len(current), args.within))
    for p in current:
        print("    %-24s %-16s headers %s" % (p.get("addr"), p.get("subver"), p.get("synced_headers")))
    print()
    print("peer versions:")
    for v, n in collections.Counter(p.get("subver", "?") for p in peers).most_common():
        print("   %-18s %d" % (v, n))
    return 0


if __name__ == "__main__":
    sys.exit(main())
