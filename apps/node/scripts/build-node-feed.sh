#!/usr/bin/env bash
# Build + verify the BTX Node updater feed (latest-node.json) for a release.
#
# The node is signed on the Mac per release (same as the miner). The macOS build
# already emits <app>.app.tar.gz + .sig; this script signs whichever artifacts
# you give it with the SAME updater key, assembles the feed, and — critically —
# verifies every signature against the app's embedded public key with minisign,
# so a feed that would be rejected at update time can never be published.
#
# ⚠ PLATFORM SUBSETS ARE SUPPORTED, AND ARE USUALLY WHAT YOU WANT. Since 0.6.2
# the btxd trains diverged and releases are per-platform. A feed that OMITS a
# platform key is valid: those clients stay on their current build, which is
# correct when they are not part of this release.
#
# Precisely: the omitted platform gets Err(TargetsNotFound) from the plugin
# (get_urls runs before the version compare), NOT a quiet "no update available".
# main.ts only surfaces check errors on a MANUAL check, so the automatic one is
# silent and nothing is downloaded — but "Check now" will tell those users
# "Couldn't check right now — are you online?". Harmless, and expected.
#
# The trap this avoids is the opposite move — keeping another platform's key
# while pointing it at its OLD asset. The manifest has ONE top-level version, so
# that mac would be offered 0.6.5, download 0.6.4 bytes, verify them, install
# them, still report 0.6.4, and be offered 0.6.5 again forever. Nothing errors.
# gen-node-feed.py makes that unrepresentable: it DERIVES every URL from the
# version being published, so a key is only ever present for an asset that
# exists at this version.
#
# This script and gen-node-feed.py both used to demand all three platforms,
# which is exactly why the last three releases hand-assembled the feed instead
# and silently skipped the minisign verification below. Subsets are supported so
# that the verifying path is the easy path.
#
# Usage:
#   build-node-feed.sh --version <ver> [--mac <app.tar.gz>] [--linux <.AppImage>]
#                      [--win <-setup.exe>] [--notes "..."]
#
#   # Linux-only release (mac + windows stay where they are):
#   build-node-feed.sh --version 0.6.5 --linux dist/BTX-Node_0.6.5_amd64.AppImage
#
#   # Full train:
#   build-node-feed.sh --version 0.5.1 --mac a.app.tar.gz --linux b.AppImage --win c.exe
#
# Requires: the updater key (empty password; see EASYBTX_UPDATER_KEY below),
# minisign, and npx tauri — all three reachable from ONE shell.
#
# ⚠ On the Windows/WSL publishing machine they are not. `npx tauri` runs on the
# Windows side and minisign only exists inside WSL, so this script cannot
# complete there. Do the same three steps by hand instead, which gives the same
# guarantees:
#   1. npx tauri signer sign -f "$KEY" -p "" <artifact>      (Windows)
#   2. minisign -V -P <pubkey> -x <decoded .sig> -m <artifact>   (WSL)
#   3. gen-node-feed.py --version … --linux-sig <artifact>.sig
# Step 3 independently re-checks that the signature carries the key id baked
# into tauri.conf.json, so a wrong key still cannot reach the feed.
set -euo pipefail

VERSION="" ; MAC_TGZ="" ; LINUX_APPIMAGE="" ; WIN_SETUP="" ; NOTES=""
while [ $# -gt 0 ]; do
  case "$1" in
    --version) VERSION="${2:?}" ; shift 2 ;;
    --mac)     MAC_TGZ="${2:?}" ; shift 2 ;;
    --linux)   LINUX_APPIMAGE="${2:?}" ; shift 2 ;;
    --win)     WIN_SETUP="${2:?}" ; shift 2 ;;
    --notes)   NOTES="${2:?}" ; shift 2 ;;
    -h|--help) sed -n '1,40p' "$0" ; exit 0 ;;
    *) echo "unknown argument: $1" >&2
       echo "usage: $0 --version <ver> [--mac <f>] [--linux <f>] [--win <f>] [--notes <s>]" >&2
       exit 1 ;;
  esac
done

[ -n "$VERSION" ] || { echo "--version is required (e.g. --version 0.6.5)" >&2; exit 1; }
if [ -z "$MAC_TGZ" ] && [ -z "$LINUX_APPIMAGE" ] && [ -z "$WIN_SETUP" ]; then
  echo "need at least one of --mac / --linux / --win — a feed with no platforms" >&2
  echo "offers nothing to anyone." >&2
  exit 1
fi
NOTES="${NOTES:-BTX Node ${VERSION}. See https://easybtx.com/node}"

# The updater signing key. $HOME/.tauri/easybtx.key is where the release recipe
# has always said it lives, and that is true on the mac. It is NOT the only
# place: on the Linux publishing machine the same key sits at the repo root as
# `updater.key`, and searching only the documented path led to a wrong
# conclusion that this machine could not sign at all. So allow an override, and
# say which file is being used rather than failing with a path nobody set.
KEY="${EASYBTX_UPDATER_KEY:-$HOME/.tauri/easybtx.key}"
# The public key baked into the app (tauri.conf.json plugins.updater.pubkey),
# decoded to the minisign form. Verifying against THIS is the whole point.
PUBKEY="RWSiwrxz2pJDXR4AchfYB5DQzvD8VbIE3D87Ft4D/km76xZIikK6XE9y"
HERE="$(cd "$(dirname "$0")" && pwd)"          # apps/node/scripts
REPO_ROOT="$(cd "$HERE/../../.." && pwd)"       # repo root
OUT_DIR="$REPO_ROOT/site/public/updater"
TAG="node-v${VERSION}"

[ -f "$KEY" ] || {
  echo "missing signing key: $KEY" >&2
  echo "set EASYBTX_UPDATER_KEY to its path if it lives somewhere else" >&2
  exit 1; }
echo "signing with key: $KEY"

# Verification is the one check that catches a feed signed with the wrong key,
# so it is never optional — but it no longer requires the `minisign` binary.
# Demanding it was how three releases ended up hand-assembling the feed and
# skipping this step entirely: the release Mac has no Homebrew, and on
# Windows/WSL `minisign` and `npx tauri` are never in the same shell. The
# Python verifier does the same Ed25519 check (key id, artifact signature, and
# the global signature over the trusted comment) with the `cryptography`
# package the toolchain already has.
VERIFIER="$HERE/verify-updater-sig.py"
if command -v minisign >/dev/null; then
  VERIFY_MODE=minisign
elif [ -f "$VERIFIER" ] && python3 -c 'import cryptography' 2>/dev/null; then
  VERIFY_MODE=python
  echo "minisign not found — verifying with $(basename "$VERIFIER")"
else
  echo "cannot verify signatures: no minisign, and no python3 cryptography for" >&2
  echo "$VERIFIER. Install either one. Refusing to build an unverified feed." >&2
  exit 1
fi

# Every artifact must exist before we sign anything, so a typo fails in a second
# rather than after signing half the release.
for f in "$MAC_TGZ" "$LINUX_APPIMAGE" "$WIN_SETUP"; do
  [ -z "$f" ] && continue
  [ -f "$f" ] || { echo "no such file: $f" >&2; exit 1; }
done

NODE_DIR="$HERE/.."   # apps/node — where @tauri-apps/cli is installed
sign() { # <abs file> -> writes <file>.sig next to it
  echo "signing $(basename "$1")"
  ( cd "$NODE_DIR" && npx tauri signer sign -f "$KEY" -p "" "$1" ) >/dev/null
}

verify() { # <file> — verify the tauri sig against the app's embedded pubkey.
  echo "verifying $(basename "$1") against the app's public key"
  if [ "$VERIFY_MODE" = python ]; then
    python3 "$VERIFIER" "$NODE_DIR/src-tauri/tauri.conf.json" "$1" \
      || { echo "SIGNATURE VERIFY FAILED for $1 — refusing to build feed"; exit 1; }
    return
  fi
  # A tauri .sig is base64 wrapping the real minisign signature, so decode first.
  local dec; dec="$(mktemp)"
  base64 -d -i "${1}.sig" > "$dec" 2>/dev/null || base64 -d < "${1}.sig" > "$dec"
  minisign -V -P "$PUBKEY" -x "$dec" -m "$1" >/dev/null \
    || { echo "SIGNATURE VERIFY FAILED for $1 — refusing to build feed"; rm -f "$dec"; exit 1; }
  rm -f "$dec"
}

# Mac .app.tar.gz.sig is produced by the build; only sign it if it's missing.
if [ -n "$MAC_TGZ" ]; then
  [ -f "${MAC_TGZ}.sig" ] || sign "$MAC_TGZ"
  verify "$MAC_TGZ"
fi
if [ -n "$LINUX_APPIMAGE" ]; then sign "$LINUX_APPIMAGE"; verify "$LINUX_APPIMAGE"; fi
if [ -n "$WIN_SETUP" ]; then sign "$WIN_SETUP"; verify "$WIN_SETUP"; fi

mkdir -p "$OUT_DIR"
ARGS=(--version "$VERSION" --tag "$TAG")
[ -n "$MAC_TGZ" ]        && ARGS+=(--mac-sig   "${MAC_TGZ}.sig")
[ -n "$LINUX_APPIMAGE" ] && ARGS+=(--linux-sig "${LINUX_APPIMAGE}.sig")
[ -n "$WIN_SETUP" ]      && ARGS+=(--win-sig   "${WIN_SETUP}.sig")
python3 "$HERE/gen-node-feed.py" "${ARGS[@]}" \
  --notes "$NOTES" --pub-date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --out "$OUT_DIR/latest-node.json"

echo
echo "feed written to $OUT_DIR/latest-node.json — every signature in it was"
echo "verified against the app key."
echo "Release-asset names the feed expects on $TAG:"
[ -n "$MAC_TGZ" ]        && echo "  BTX-Node_${VERSION}_aarch64.app.tar.gz   (= $MAC_TGZ)"
[ -n "$LINUX_APPIMAGE" ] && echo "  BTX-Node_${VERSION}_amd64.AppImage       (= $LINUX_APPIMAGE)"
[ -n "$WIN_SETUP" ]      && echo "  BTX-Node_${VERSION}_x64-setup.exe        (= $WIN_SETUP)"
echo
echo "Those assets MUST be attached to $TAG before the feed goes live."
exit 0
