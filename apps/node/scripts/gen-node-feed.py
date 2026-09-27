#!/usr/bin/env python3
"""Generate the BTX Node updater feed (latest-node.json).

The node is ONE cross-platform app on ONE manifest with ONE top-level
`version`, unlike the miner's separate per-platform feeds.

⚠ THE STALE-KEY TRAP — the reason this script mints URLs instead of taking
them. The manifest carries a single version, so every platform key in it is a
claim that "this platform's asset exists at this version". If you publish
0.6.5 for Linux but leave a `darwin-aarch64` entry pointing at its old 0.6.4
asset, every Mac enters a PERMANENT UPDATE LOOP: offered 0.6.5, downloads
0.6.4 bytes, verifies (the signature is genuine), installs, still reports
0.6.4, gets offered 0.6.5 again. Nothing errors. It simply never converges.

This script cannot produce that state — it always derives each URL from the
version and tag being published — but only as long as you pass signatures ONLY
for platforms that actually ship an asset on that tag.

⚠ A PLATFORM SUBSET IS SUPPORTED, AND IS THE SAFE CHOICE. Absent is the
documented safe state: a client whose platform key is missing sees "no update
available" and stays exactly where it is. That is correct whenever the other
platforms already have the build you wanted them on. The live feed has in fact
carried a single platform since 0.6.2. This script previously demanded all
three, which is why the last three releases bypassed it and hand-assembled the
feed, losing the signature verification below — the thing it exists for.

At least one platform is required; each one supplied is verified.

The `signature` values are the raw contents of the tauri/minisign `.sig` files
(NOT a path or URL). The `url` values point at the release assets the updater
downloads: the Mac .app.tar.gz, the Linux .AppImage, the Windows -setup.exe.

Verification performed here: each signature must be a well-formed tauri/minisign
signature AND must carry the same key id as the public key baked into the app
(tauri.conf.json → plugins.updater.pubkey). That catches a feed signed with the
wrong key — which the client would reject at update time, after shipping.
Verifying the signature against the actual FILE BYTES needs the artifact and is
done by build-node-feed.sh with minisign -V.

Usage:
  # Linux-only release (mac + windows stay where they are):
  gen-node-feed.py --version 0.6.5 --tag node-v0.6.5 \
     --linux-sig <file> --notes "..." --pub-date ... --out latest-node.json

  # All three:
  gen-node-feed.py --version 0.5.1 --tag node-v0.5.1 \
     --mac-sig <file> --linux-sig <file> --win-sig <file> ... --out latest-node.json

  gen-node-feed.py --self-test
"""
import argparse
import base64
import binascii
import json
import os
import re
import tempfile

REPO = "https://github.com/MendeMatthias/EasyBTX-releases/releases/download"
# Release asset names follow the node convention BTX-Node_<ver>_<arch>.<ext>.
#
# linux-x86_64 is the AppImage, deliberately, and there is no `.deb` key: the
# updater cannot install one. BundleType::Deb reaches install_deb and fails
# InvalidUpdaterFormat, and when bundle detection returns None it falls through
# to install_appimage, which renames over a root-owned /usr path and gets
# EACCES. Adding a deb entry here would not give deb users an update path; it
# would make every one of them download ~450 MB on a six-hour timer to reach a
# guaranteed failure. They are told to update by hand instead (README, and the
# banner the app shows when an install fails).
ASSET = {
    "darwin-aarch64": "BTX-Node_{v}_aarch64.app.tar.gz",
    "linux-x86_64": "BTX-Node_{v}_amd64.AppImage",
    "windows-x86_64": "BTX-Node_{v}_x64-setup.exe",
}

_HERE = os.path.dirname(os.path.abspath(__file__))
TAURI_CONF = os.path.join(_HERE, "..", "src-tauri", "tauri.conf.json")


def _key_id(blob):
    """Extract the 8-byte minisign key id from a decoded pubkey/signature line.

    Both structures are base64 of: algorithm[2] || key_id[8] || payload.
    The algorithm bytes differ by design (`Ed` for a public key, `ED` for a
    prehashed signature), so only the key id is comparable.
    """
    if len(blob) < 10:
        raise ValueError("minisign blob too short to contain a key id")
    return blob[2:10]


def pubkey_key_id(pubkey_b64):
    """Key id of the app's embedded updater public key.

    tauri.conf.json stores the pubkey base64-wrapped around a whole minisign
    public-key FILE; a bare key line is accepted too.
    """
    try:
        decoded = base64.b64decode(pubkey_b64.strip(), validate=True)
    except (binascii.Error, ValueError):
        raise ValueError("updater pubkey is not valid base64")
    text = decoded.decode("utf-8", "replace")
    lines = [ln.strip() for ln in text.splitlines() if ln.strip()]
    if not lines:
        raise ValueError("updater pubkey decoded to nothing")
    # Whole-file form: comment line then the key line. Bare form: just the key.
    key_line = lines[1] if len(lines) > 1 else lines[0]
    try:
        return _key_id(base64.b64decode(key_line, validate=True))
    except (binascii.Error, ValueError):
        raise ValueError("updater pubkey body is not valid base64")


def load_pubkey_from_conf(path=TAURI_CONF):
    with open(path) as f:
        conf = json.load(f)
    try:
        return conf["plugins"]["updater"]["pubkey"]
    except (KeyError, TypeError):
        raise ValueError(f"no plugins.updater.pubkey in {path}")


def _signed_name(decoded):
    """The file name a minisign signature was made over: the `file:` field of
    its trusted comment, read from the THIRD line exactly as the app's
    verifier reads it (minisign-verify's Signature::decode). The first line is
    an untrusted comment and may say anything."""
    lines = decoded.split("\n")
    if len(lines) < 3 or not lines[2].startswith("trusted comment: "):
        return None
    for field in lines[2][len("trusted comment: "):].rstrip("\r").split("\t"):
        if field.startswith("file:"):
            return field[len("file:"):]
    return None


def _check_sig(target, sig, want_key_id=None, want_name=None):
    if not sig or not sig.strip():
        raise ValueError(f"{target}: empty signature")
    # A tauri .sig file is base64 wrapping the real minisign signature, which
    # begins with "untrusted comment:". Decode and check that, so a stray path,
    # URL, or truncated blob is rejected before it can poison the feed.
    try:
        decoded = base64.b64decode(sig.strip(), validate=True).decode("utf-8", "replace")
    except (binascii.Error, ValueError):
        raise ValueError(f"{target}: signature is not valid base64 (not a tauri .sig?)")
    if "untrusted comment:" not in decoded:
        raise ValueError(f"{target}: decoded signature is not a minisign signature")
    if want_key_id is None:
        return
    lines = [ln.strip() for ln in decoded.splitlines() if ln.strip()]
    if len(lines) < 2:
        raise ValueError(f"{target}: signature file has no signature line")
    try:
        got = _key_id(base64.b64decode(lines[1], validate=True))
    except (binascii.Error, ValueError):
        raise ValueError(f"{target}: signature line is not valid base64")
    if got != want_key_id:
        # Displayed the way minisign prints it (reversed), so the value can be
        # compared by eye with the pubkey's own comment line.
        raise ValueError(
            f"{target}: signed with key id {got[::-1].hex().upper()}, but the app "
            f"trusts {want_key_id[::-1].hex().upper()} — this feed would be "
            f"REJECTED by every client. Wrong signing key."
        )
    if want_name is None:
        return
    # easyNode 0.6.32 and later take an update only when every entry's build
    # was signed under the release name for its platform and this version
    # (update_binding.rs). One entry signed under another name, the bundler's
    # "easyBTX Node.app.tar.gz" for instance, and they refuse the release, so
    # this feed would stop their updates. Refused here, before it can ship.
    named = _signed_name(decoded)
    if named != want_name:
        raise ValueError(
            f"{target}: signed as {named!r}, but this release must be signed as "
            f"{want_name!r}. easyNode 0.6.32 and later refuse any other name. "
            f"Rename the file to {want_name} and sign it again "
            f"(build-node-feed.sh does both)."
        )


def build_feed(version, tag, notes, pub_date, mac_sig=None, linux_sig=None,
               win_sig=None, pubkey=None):
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError(f"version must be X.Y.Z, got {version!r}")
    if version not in tag:
        raise ValueError(f"tag {tag!r} does not contain version {version!r}")
    want = pubkey_key_id(pubkey) if pubkey else None
    sigs = {
        "darwin-aarch64": mac_sig,
        "linux-x86_64": linux_sig,
        "windows-x86_64": win_sig,
    }
    supplied = {t: s for t, s in sigs.items() if s and s.strip()}
    if not supplied:
        raise ValueError(
            "no platform signatures supplied — a feed with no platforms offers "
            "nothing to anyone. Pass at least one of --mac-sig/--linux-sig/--win-sig."
        )
    platforms = {}
    for target, sig in supplied.items():
        _check_sig(target, sig, want, ASSET[target].format(v=version))
        # URL is DERIVED, never passed in: that is what makes the stale-key
        # loop described at the top of this file unrepresentable here.
        url = f"{REPO}/{tag}/{ASSET[target].format(v=version)}"
        platforms[target] = {"signature": sig.strip(), "url": url}
    return {"version": version, "notes": notes, "pub_date": pub_date, "platforms": platforms}


def write_atomic(path, data):
    text = json.dumps(data, indent=2) + "\n"
    json.loads(text)  # never write something we can't read back
    d = os.path.dirname(os.path.abspath(path)) or "."
    os.makedirs(d, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=d, suffix=".tmp")
    try:
        with os.fdopen(fd, "w") as f:
            f.write(text)
        os.replace(tmp, path)
    except BaseException:
        if os.path.exists(tmp):
            os.remove(tmp)
        raise


def _fake_sig(key_id=b"\x01\x02\x03\x04\x05\x06\x07\x08", name="x"):
    """A structurally valid tauri .sig carrying a chosen key id and file name."""
    body = base64.b64encode(b"ED" + key_id + b"\x00" * 64).decode()
    raw = (
        "untrusted comment: signature from tauri secret key\n"
        f"{body}\n"
        f"trusted comment: timestamp:0\tfile:{name}\nZg==\n"
    )
    return base64.b64encode(raw.encode()).decode()


def _named(target, version, key_id=b"\x01\x02\x03\x04\x05\x06\x07\x08"):
    """A fake signature for `target`, signed under its release name."""
    return _fake_sig(key_id, ASSET[target].format(v=version))


def _fake_pubkey(key_id=b"\x01\x02\x03\x04\x05\x06\x07\x08"):
    body = base64.b64encode(b"Ed" + key_id + b"\x00" * 32).decode()
    raw = f"untrusted comment: minisign public key: X\n{body}\n"
    return base64.b64encode(raw.encode()).decode()


def self_test():
    pub = _fake_pubkey()
    mac, linux, win = (_named(t, "0.5.1") for t in
                       ("darwin-aarch64", "linux-x86_64", "windows-x86_64"))
    feed = build_feed("0.5.1", "node-v0.5.1", "notes", "2026-07-15T00:00:00Z",
                      mac, linux, win, pubkey=pub)
    assert set(feed["platforms"]) == {"darwin-aarch64", "linux-x86_64", "windows-x86_64"}, feed
    assert feed["platforms"]["darwin-aarch64"]["url"].endswith(
        "node-v0.5.1/BTX-Node_0.5.1_aarch64.app.tar.gz"
    ), feed
    assert feed["platforms"]["linux-x86_64"]["url"].endswith("BTX-Node_0.5.1_amd64.AppImage")
    assert feed["platforms"]["windows-x86_64"]["url"].endswith("BTX-Node_0.5.1_x64-setup.exe")
    assert feed["version"] == "0.5.1"

    # A single-platform feed is legitimate, and its URL is minted at the NEW
    # version — the stale-key loop cannot be expressed.
    only_linux = build_feed("0.6.5", "node-v0.6.5", "n", "d",
                            linux_sig=_named("linux-x86_64", "0.6.5"), pubkey=pub)
    assert set(only_linux["platforms"]) == {"linux-x86_64"}, only_linux
    assert only_linux["platforms"]["linux-x86_64"]["url"].endswith(
        "node-v0.6.5/BTX-Node_0.6.5_amd64.AppImage"
    ), only_linux

    # Mac-only and windows-only both work too.
    assert set(build_feed("0.6.4", "node-v0.6.4", "n", "d",
                          mac_sig=_named("darwin-aarch64", "0.6.4"),
                          pubkey=pub)["platforms"]) == {"darwin-aarch64"}
    assert set(build_feed("0.6.4", "node-v0.6.4", "n", "d",
                          win_sig=_named("windows-x86_64", "0.6.4"),
                          pubkey=pub)["platforms"]) == {"windows-x86_64"}

    # Signatures are still verified, and a wrong signing key is caught.
    wrong = _named("linux-x86_64", "0.5.1", b"\x09\x09\x09\x09\x09\x09\x09\x09")
    for bad in (
        lambda: build_feed("9.9.9", "node-v0.5.1", "", "", mac, linux, win),   # version/tag mismatch
        lambda: build_feed("0.5.1", "node-v0.5.1", "", ""),                  # no platforms at all
        lambda: build_feed("0.5.1", "node-v0.5.1", "", "", mac_sig="not a sig"),  # bad sig
        lambda: build_feed("0.5", "node-v0.5", "", "", mac_sig=mac),         # non-semver
        lambda: build_feed("0.5.1", "node-v0.5.1", "", "", linux_sig=wrong, pubkey=pub),  # wrong key
    ):
        try:
            bad()
            raise AssertionError("expected ValueError")
        except ValueError:
            pass

    # Every entry must be signed under ITS release name at THIS version, or
    # easyNode 0.6.32+ refuses the whole release (update_binding.rs).
    unbound = (
        # The bundler's own signature, which named the 0.6.30 Mac build.
        ("darwin-aarch64", _fake_sig(name="easyBTX Node.app.tar.gz")),
        # A genuine older build, offered as this version.
        ("linux-x86_64", _named("linux-x86_64", "0.5.0")),
        # Another platform's build under this platform's key.
        ("linux-x86_64", _named("windows-x86_64", "0.5.1")),
        # The miner, which the same key signs.
        ("linux-x86_64", _fake_sig(name="easyBTX_0.5.1_amd64.AppImage")),
    )
    for target, bad_sig in unbound:
        arg = {"darwin-aarch64": "mac_sig", "linux-x86_64": "linux_sig"}[target]
        try:
            build_feed("0.5.1", "node-v0.5.1", "", "", pubkey=pub, **{arg: bad_sig})
            raise AssertionError(f"accepted an unbound {target} signature")
        except ValueError as e:
            assert "must be signed as" in str(e), e

    # The untrusted first line is not the trusted comment, however it reads.
    forged = base64.b64decode(_named("linux-x86_64", "0.5.0")).decode().split("\n")
    forged[0] = "trusted comment: timestamp:0\tfile:BTX-Node_0.5.1_amd64.AppImage"
    forged = base64.b64encode("\n".join(forged).encode()).decode()
    try:
        build_feed("0.5.1", "node-v0.5.1", "", "", linux_sig=forged, pubkey=pub)
        raise AssertionError("read the untrusted comment as the signed one")
    except ValueError:
        pass

    # The REAL embedded pubkey must parse, so a conf change cannot silently
    # disable key-id verification.
    real = pubkey_key_id(load_pubkey_from_conf())
    assert len(real) == 8, real
    print(f"gen-node-feed self-test OK (app trusts key id {real[::-1].hex().upper()})")


def _read(path):
    with open(path) as f:
        return f.read()


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--self-test", action="store_true")
    p.add_argument("--version")
    p.add_argument("--tag")
    p.add_argument("--mac-sig")
    p.add_argument("--linux-sig")
    p.add_argument("--win-sig")
    p.add_argument("--notes", default="")
    p.add_argument("--pub-date", default="")
    p.add_argument("--out")
    p.add_argument("--pubkey-conf", default=TAURI_CONF,
                   help="tauri.conf.json holding plugins.updater.pubkey")
    a = p.parse_args()
    if a.self_test:
        return self_test()
    if not (a.version and a.tag and a.out):
        p.error("need --version --tag --out")
    if not (a.mac_sig or a.linux_sig or a.win_sig):
        p.error("need at least one of --mac-sig / --linux-sig / --win-sig")
    feed = build_feed(
        a.version, a.tag, a.notes, a.pub_date,
        _read(a.mac_sig) if a.mac_sig else None,
        _read(a.linux_sig) if a.linux_sig else None,
        _read(a.win_sig) if a.win_sig else None,
        pubkey=load_pubkey_from_conf(a.pubkey_conf),
    )
    names = ", ".join(sorted(feed["platforms"]))
    print(f"wrote {a.out} — version {feed['version']}, platforms: {names}")
    missing = sorted(set(ASSET) - set(feed["platforms"]))
    if missing:
        print(f"NOTE: no key for {', '.join(missing)} — those clients stay on "
              f"their current build and download nothing, which is the intended "
              f"safe state when they are not part of this release.")
        print("      (Mechanically the plugin returns TargetsNotFound; main.ts "
              "only surfaces check errors on a MANUAL check, so the automatic "
              "one is silent. Expect 'Check now' on those platforms to say "
              "\"Couldn't check right now — are you online?\" — harmless.)")
    write_atomic(a.out, feed)


if __name__ == "__main__":
    main()
