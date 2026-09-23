#!/usr/bin/env python3
"""Does an assumeutxo snapshot file hash to the value the engine compiles in?

Why this exists
---------------
A snapshot pin was proven by `loadtxoutset` on a real node, and a node can only
load once its headers are past the snapshot base. On 2026-09-23 a fresh v0.34.9
node could not get there: header pre-sync twice climbed to about 180000 and
started over, and after an hour and a half it had not finished, while a dozen
peers, the app's own bootstrap peers among them, failed with "low-work headers
sync failure". The question a load answers about the FILE needs no node: does
its UTXO set hash to the `hash_serialized` in the tag's `m_assumeutxo_data`?
This script answers it from the file.

What it checks
--------------
It recomputes hash_serialized exactly as btx v0.34.9 does (kernel/coinstats.cpp,
ComputeUTXOStats with HASH_SERIALIZED): one double-SHA-256 stream, nothing
written before the coins, and per coin, in the file's order and vout-sorted
within a txid, COutPoint || u32(height << 1 | coinbase) || CTxOut. The coins
are read back from the snapshot format (rpc/blockchain.cpp write_coins_to_file;
compressor.cpp for amounts and scripts). With --base it also checks the base
block hash in the file header.

It reproduces both published mainnet bases: the 203000 file, which a real load
proved in 0.6.13, gives 6754314323ab...2040, and the 219000 file gives
3c065aabb529...cc99.

It does NOT check the shielded section after the coins (for 219000 it is
byte-identical to 203000's), the engine's load path, or the header chain a node
needs. A real load still covers those.

Exit 0 on a match, 1 on a mismatch, 2 on a usage or parse error.

Usage:
    python3 scripts/check-snapshot-hash.py btx-assumeutxo-219000.dat \\
        --expect 3c065aabb529eaab5646825927d9f20a91426dc7e83b4890b575324f5bfccc99 \\
        --base dc51220bc7e5db96e29df9d817ae6179245d33eb8adcaaff765cfec83fdb87c3
"""

import argparse
import hashlib
import struct
import sys

SNAPSHOT_MAGIC = b"utxo\xff"
SHIELDED_MAGIC = b"shld\xfe"
SPECIAL_SCRIPTS = 6  # compressor.h nSpecialScripts
MAX_SCRIPT_SIZE = 10000  # script/script.h
P = 2**256 - 2**32 - 977  # secp256k1 field prime, for special scripts 4 and 5


def read_exact(f, n):
    b = f.read(n)
    if len(b) != n:
        raise EOFError(f"file ends early: wanted {n} bytes, got {len(b)}")
    return b


def read_varint(f):
    """serialize.h VARINT: MSB base-128, plus one for every continuation byte."""
    n = 0
    while True:
        ch = read_exact(f, 1)[0]
        n = (n << 7) | (ch & 0x7F)
        if ch & 0x80:
            n += 1
        else:
            return n


def read_compact_size(f):
    b = read_exact(f, 1)[0]
    if b < 253:
        return b
    if b == 253:
        return struct.unpack("<H", read_exact(f, 2))[0]
    if b == 254:
        return struct.unpack("<I", read_exact(f, 4))[0]
    return struct.unpack("<Q", read_exact(f, 8))[0]


def compact_size(n):
    if n < 253:
        return bytes([n])
    if n <= 0xFFFF:
        return b"\xfd" + struct.pack("<H", n)
    if n <= 0xFFFFFFFF:
        return b"\xfe" + struct.pack("<I", n)
    return b"\xff" + struct.pack("<Q", n)


def decompress_amount(x):
    """compressor.cpp DecompressAmount."""
    if x == 0:
        return 0
    x -= 1
    e = x % 10
    x //= 10
    if e < 9:
        d = (x % 9) + 1
        x //= 9
        n = x * 10 + d
    else:
        n = x + 1
    return n * 10**e


def decompress_pubkey(prefix, xb):
    """CPubKey::Decompress of a 0x02/0x03 key; None when x is not on the curve."""
    x = int.from_bytes(xb, "big")
    y2 = (pow(x, 3, P) + 7) % P
    y = pow(y2, (P + 1) // 4, P)
    if (y * y) % P != y2:
        return None
    if (y & 1) != (prefix & 1):
        y = P - y
    return b"\x04" + xb + y.to_bytes(32, "big")


def read_script(f, counts):
    """compressor.h ScriptCompression::Unser, scripts rebuilt in full."""
    n_size = read_varint(f)
    if n_size < SPECIAL_SCRIPTS:
        data = read_exact(f, 20 if n_size in (0, 1) else 32)
        counts[f"special_{n_size}"] += 1
        if n_size == 0:  # P2PKH
            return b"\x76\xa9\x14" + data + b"\x88\xac"
        if n_size == 1:  # P2SH
            return b"\xa9\x14" + data + b"\x87"
        if n_size in (2, 3):  # P2PK, compressed key
            return b"\x21" + bytes([n_size]) + data + b"\xac"
        pub = decompress_pubkey(n_size - 2, data)  # P2PK, uncompressed key
        if pub is None:  # DecompressScript fails and the script stays empty
            counts["bad_pubkey"] += 1
            return b""
        return b"\x41" + pub + b"\xac"
    n_size -= SPECIAL_SCRIPTS
    if n_size > MAX_SCRIPT_SIZE:  # replaced by a lone OP_RETURN
        read_exact(f, n_size)
        counts["oversize"] += 1
        return b"\x6a"
    return read_exact(f, n_size)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("snapshot", help="the snapshot file, e.g. btx-assumeutxo-219000.dat")
    ap.add_argument("--expect", required=True, help="hash_serialized from m_assumeutxo_data, as printed there")
    ap.add_argument("--base", help="blockhash from m_assumeutxo_data, checked against the file header")
    args = ap.parse_args()

    counts = {f"special_{i}": 0 for i in range(SPECIAL_SCRIPTS)}
    counts.update(bad_pubkey=0, oversize=0)
    try:
        with open(args.snapshot, "rb") as f:
            if read_exact(f, 5) != SNAPSHOT_MAGIC:
                print("not a UTXO snapshot: wrong magic", file=sys.stderr)
                return 2
            version = struct.unpack("<H", read_exact(f, 2))[0]
            network_magic = read_exact(f, 4).hex()
            base = read_exact(f, 32)[::-1].hex()
            declared = struct.unpack("<Q", read_exact(f, 8))[0]
            h = hashlib.sha256()
            coins = txids = 0
            while coins < declared:
                txid = read_exact(f, 32)
                outs = []
                for _ in range(read_compact_size(f)):
                    vout = read_compact_size(f)
                    code = read_varint(f)
                    value = decompress_amount(read_varint(f))
                    outs.append((vout, code, value, read_script(f, counts)))
                for vout, code, value, script in sorted(outs, key=lambda o: o[0]):
                    h.update(
                        txid
                        + struct.pack("<I", vout)
                        + struct.pack("<I", code)
                        + struct.pack("<q", value)
                        + compact_size(len(script))
                        + script
                    )
                coins += len(outs)
                txids += 1
            rest = f.read()
    except (OSError, EOFError) as e:
        print(f"cannot read {args.snapshot}: {e}", file=sys.stderr)
        return 2

    got = hashlib.sha256(h.digest()).digest()[::-1].hex()
    print(f"header      version {version}, network magic {network_magic}, base {base}")
    print(f"coins       {coins} of {declared} declared, in {txids} txids")
    print(f"scripts     {counts}")
    shielded = "a shielded section" if rest.startswith(SHIELDED_MAGIC) else "NOT a shielded section"
    print(f"after coins {len(rest)} bytes, {shielded}")
    print(f"computed    {got}")
    print(f"expected    {args.expect.lower()}")
    ok = got == args.expect.lower() and coins == declared
    if args.base is not None:
        base_ok = base == args.base.lower()
        print(f"base block  {'matches' if base_ok else 'DOES NOT MATCH'} {args.base.lower()}")
        ok = ok and base_ok
    print("MATCH" if ok else "MISMATCH")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
