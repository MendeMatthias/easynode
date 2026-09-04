#!/usr/bin/env python3
"""Census a Bitcoin-Core-style `blocks/` directory: how many block records it
holds, how many bytes they occupy, and the span of their header timestamps.

Why this exists
---------------
`du -sh` on a blocks directory answers the wrong question. Block files are
preallocated in 128 MiB chunks, a pruned node's directory holds only a retained
window, and a `blocks.preadopt-*` backup holds blocks that were downloaded and
never connected. None of those numbers is "what the chain costs". This walks the
actual record framing and counts the actual block bytes.

BTX block sizes are BIMODAL — a block is either ~367 bytes or ~1.049 MB, the
large mode being the MatMul PoW payload rather than transaction traffic — so the
mean alone is misleading. The split is reported separately.

Notes
-----
* Bitcoin Core 28+ obfuscates block files with the 8-byte key in `xor.dat`.
  Every byte is XORed with `key[offset % 8]`. Without that step the framing
  reads as noise, so this is not optional.
* Record framing is: 4-byte network magic, 4-byte little-endian payload size,
  then the payload. A file's preallocated tail is zeroes, which de-XOR to the
  key repeating, so the magic check is what ends each file.
* The block header's timestamp is at payload offset 68 (version 4, prev 32,
  merkle 32, then time). BTX's extra MatMul fields come after the classic 80,
  so that offset still holds.
* READ-ONLY. It opens block files and never the LevelDB indexes, so it is safe
  to run against a datadir with a live btxd attached to it.

Usage:  python3 scripts/blockstore-census.py <datadir>/blocks
"""
import datetime
import os
import struct
import sys

BIG = 500_000  # anything at or above this is the large mode; the gap is ~1000x


def census(d):
    xor_path = os.path.join(d, "xor.dat")
    xor = b"\x00" * 8
    if os.path.exists(xor_path):
        with open(xor_path, "rb") as f:
            xor = (f.read(8) + b"\x00" * 8)[:8]

    def unxor(buf, off):
        return bytes(b ^ xor[(off + i) % 8] for i, b in enumerate(buf))

    files = sorted(x for x in os.listdir(d) if x.startswith("blk") and x.endswith(".dat"))
    big = small = big_bytes = small_bytes = 0
    tmin = tmax = None
    magic0 = None

    for name in files:
        path = os.path.join(d, name)
        fsz = os.path.getsize(path)
        with open(path, "rb") as f:
            off = 0
            while off + 8 <= fsz:
                f.seek(off)
                head = f.read(8)
                if len(head) < 8:
                    break
                head = unxor(head, off)
                magic, (size,) = head[:4], struct.unpack("<I", head[4:8])
                if size == 0 or magic == b"\x00\x00\x00\x00":
                    break  # preallocated tail
                if magic0 is None:
                    magic0 = magic
                # Stop the file honestly rather than guessing past lost framing.
                if magic != magic0 or size > 64 * 2**20 or off + 8 + size > fsz:
                    break
                if size >= 80:
                    hdr = unxor(f.read(80), off + 8)
                    (ts,) = struct.unpack("<I", hdr[68:72])
                    if 1_700_000_000 < ts < 2_000_000_000:
                        tmin = ts if tmin is None else min(tmin, ts)
                        tmax = ts if tmax is None else max(tmax, ts)
                if size >= BIG:
                    big += 1
                    big_bytes += size
                else:
                    small += 1
                    small_bytes += size
                off += 8 + size

    return {
        "files": len(files),
        "records": big + small,
        "big": big,
        "small": small,
        "bytes": big_bytes + small_bytes,
        "big_bytes": big_bytes,
        "small_bytes": small_bytes,
        "t_first": tmin,
        "t_last": tmax,
    }


def main():
    if len(sys.argv) != 2:
        print(__doc__.strip().splitlines()[-1], file=sys.stderr)
        return 2
    d = sys.argv[1]
    if not os.path.isdir(d):
        print("not a directory: %s" % d, file=sys.stderr)
        return 2
    r = census(d)
    # A directory with no block files must not report "0 bytes" and exit 0. A
    # census that silently succeeds on nothing is how a wrong size gets quoted.
    if r["files"] == 0:
        print("no blk*.dat files in %s - is this a blocks/ directory?" % d, file=sys.stderr)
        return 1
    if r["records"] == 0:
        print("%d blk*.dat files in %s but no readable block records - wrong magic, "
              "or an xor.dat that does not match these files" % (r["files"], d), file=sys.stderr)
        return 1
    n = r["records"]
    fmt = lambda t: datetime.datetime.utcfromtimestamp(t).strftime("%Y-%m-%d %H:%M") if t else "n/a"
    print("dir              : %s" % d)
    print("blk files        : %d" % r["files"])
    print("block records    : %d" % r["records"])
    print("  >= %d B    : %d (%.1f%%), %.2f GiB" % (BIG, r["big"], 100.0 * r["big"] / n, r["big_bytes"] / 2**30))
    print("  <  %d B    : %d (%.1f%%), %.4f GiB" % (BIG, r["small"], 100.0 * r["small"] / n, r["small_bytes"] / 2**30))
    print("total payload    : %.2f GiB (%.2f GB)" % (r["bytes"] / 2**30, r["bytes"] / 1e9))
    print("mean per record  : %.0f B" % (r["bytes"] / n))
    print("header timestamps: %s .. %s UTC" % (fmt(r["t_first"]), fmt(r["t_last"])))
    return 0


if __name__ == "__main__":
    sys.exit(main())
