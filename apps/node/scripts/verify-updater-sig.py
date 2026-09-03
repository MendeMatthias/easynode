#!/usr/bin/env python3
"""Verify a Tauri updater `.sig` against the app's embedded minisign public key.

This is `minisign -V -P <pubkey> -x <decoded .sig> -m <artifact>`, in Python.

Why it exists. `build-node-feed.sh` refuses to publish a feed it cannot verify,
and that refusal is the single check that catches a feed signed with the wrong
key. Until now the only way to satisfy it was the `minisign` binary, which is
not on every publishing machine (no Homebrew on the release Mac as of
2026-08-25, and on Windows `minisign` lives only inside WSL while `npx tauri`
runs Windows-side, so no ONE shell had the whole toolchain). The documented
workaround was to do the steps by hand, which is exactly how three releases
ended up skipping verification altogether.

Standard library plus `cryptography`, which ships with the toolchain already.

Format, for the next reader:

  A minisign public key line is base64 of
      algorithm[2] || key_id[8] || ed25519_public_key[32]
  A minisign signature line is base64 of
      algorithm[2] || key_id[8] || ed25519_signature[64]
  A Tauri `.sig` file is that whole minisign file, base64-wrapped again.

  Algorithm `Ed` signs the file bytes directly. Algorithm `ED` signs
  BLAKE2b-512 of the file bytes ("prehashed"). Both are handled; Tauri emits
  `Ed`, but a future signer flipping to prehashed must not silently fail open.

  The trailing global signature covers `signature || trusted_comment` and is
  what makes the trusted comment tamper-evident. It is checked too — skipping
  it would leave the filename and timestamp in the feed unauthenticated.

Exit status is 0 only when every check passes.
"""

import base64
import hashlib
import json
import sys
from pathlib import Path

try:
    from cryptography.exceptions import InvalidSignature
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
except ImportError:  # pragma: no cover - environment problem, not logic
    sys.exit(
        "verify-updater-sig.py needs the `cryptography` package "
        "(python3 -m pip install cryptography), or install minisign instead."
    )

LEGACY = b"Ed"      # signs the raw file
PREHASHED = b"ED"   # signs BLAKE2b-512(file)


def _die(msg):
    sys.exit(f"SIGNATURE VERIFICATION FAILED: {msg}")


def _b64(value, what):
    try:
        return base64.b64decode(value.strip(), validate=True)
    except Exception:
        _die(f"{what} is not valid base64")


def parse_pubkey(pubkey_b64):
    """Return (algorithm, key_id, ed25519_key) from tauri.conf.json's pubkey."""
    outer = _b64(pubkey_b64, "updater pubkey")
    lines = [ln for ln in outer.decode("utf-8", "replace").splitlines() if ln.strip()]
    if not lines:
        _die("updater pubkey decoded to nothing")
    raw = _b64(lines[-1], "updater pubkey body")
    if len(raw) != 42:
        _die(f"updater pubkey is {len(raw)} bytes, expected 42")
    return raw[:2], raw[2:10], raw[10:]


def parse_sig(sig_path):
    """Return (algorithm, key_id, signature, trusted_comment, global_sig)."""
    try:
        text = Path(sig_path).read_text()
    except OSError as e:
        # An absent .sig is the most likely operator error of all (a build that
        # skipped the updater bundle), and it must read as a refusal, not a
        # stack trace — a traceback in a release script invites "probably
        # nothing" far more than one clear line does.
        _die(f"{sig_path}: cannot read the signature file ({e.strerror})")
    outer = _b64(text, f"{sig_path}")
    lines = [ln for ln in outer.decode("utf-8", "replace").splitlines() if ln.strip()]
    # untrusted comment / signature / trusted comment / global signature
    if len(lines) < 4:
        _die(f"{sig_path}: expected 4 lines in the minisign file, got {len(lines)}")
    raw = _b64(lines[1], "signature line")
    if len(raw) != 74:
        _die(f"signature line is {len(raw)} bytes, expected 74")
    marker = "trusted comment: "
    if not lines[2].startswith(marker):
        _die("third line is not a trusted comment")
    trusted = lines[2][len(marker):].encode()
    return raw[:2], raw[2:10], raw[10:], trusted, _b64(lines[3], "global signature")


def main(argv):
    if len(argv) != 3:
        sys.exit("usage: verify-updater-sig.py <artifact> <artifact.sig> "
                 "<path/to/tauri.conf.json>"[:0] or
                 "usage: verify-updater-sig.py <tauri.conf.json> <artifact>")
    conf_path, artifact = argv[1], argv[2]
    sig_path = artifact + ".sig"

    try:
        conf = json.loads(Path(conf_path).read_text())
        pubkey_b64 = conf["plugins"]["updater"]["pubkey"]
    except (OSError, ValueError, KeyError):
        _die(f"no plugins.updater.pubkey in {conf_path}")

    pk_alg, pk_id, key_bytes = parse_pubkey(pubkey_b64)
    sig_alg, sig_id, signature, trusted, global_sig = parse_sig(sig_path)

    # The key-id check is what catches a release signed with a DIFFERENT key.
    # It is cheap and it fails loudly, so it runs before the maths.
    if sig_id != pk_id:
        _die(
            f"signed with key id {sig_id.hex().upper()} but the app embeds "
            f"{pk_id.hex().upper()} — this build would be rejected at update time"
        )
    if sig_alg not in (LEGACY, PREHASHED):
        _die(f"unknown signature algorithm {sig_alg!r}")
    if pk_alg != LEGACY:
        _die(f"unexpected public key algorithm {pk_alg!r}")

    data = Path(artifact).read_bytes()
    signed = hashlib.blake2b(data, digest_size=64).digest() if sig_alg == PREHASHED else data

    key = Ed25519PublicKey.from_public_bytes(key_bytes)
    try:
        key.verify(signature, signed)
    except InvalidSignature:
        _die(f"{artifact}: the signature does not match the artifact bytes")

    # The trusted comment carries the filename and timestamp the feed shows.
    # Unverified, those are attacker-controlled text.
    try:
        key.verify(global_sig, signature + trusted)
    except InvalidSignature:
        _die(f"{artifact}: the trusted comment is not authentic")

    print(f"OK  {Path(artifact).name}")
    print(f"    key id        {pk_id.hex().upper()}")
    print(f"    algorithm     {sig_alg.decode()} "
          f"({'prehashed BLAKE2b' if sig_alg == PREHASHED else 'raw file'})")
    print(f"    trusted       {trusted.decode('utf-8', 'replace')}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
