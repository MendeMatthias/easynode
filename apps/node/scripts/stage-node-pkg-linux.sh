#!/usr/bin/env bash
# Stage the bundled BTX node package for an easyBTX Node LINUX build.
#
# Mirrors stage-node-pkg.sh (macOS) but is far simpler: no .so vendoring or
# rpath pass — download, verify the pinned sha256, copy, sanity-run.
#
# The upstream Linux binaries USED to be fully static ELFs. Since v0.34.9 they
# are not: libexec/btxd.real NEEDs libssl.so.3 and libcrypto.so.3 (OpenSSL 3.5,
# bundled in the archive's top-level lib/), plus libzmq, libevent, sqlite and
# libgomp from the host. bin/btxd, the wrapper, puts ../lib on LD_LIBRARY_PATH
# when it finds libssl.so.3 there, so those two are staged in lib/ beside bin/
# and libexec/. From v0.34.12 the archive's lib/ also holds btx-qt's runtime
# (Qt 6, libstdc++, libevent, libzmq, libgomp and about 90 more); staging all
# of it would put those in front of the host's copies for btxd too, so only the
# OpenSSL pair is taken, which is all v0.34.9's lib/ held.
#
# Source resolution order:
#   1. $EASYBTX_NODE_PKG_SRC (explicit override: an extracted package dir)
#   2. download the pinned upstream release tarball (sha256-verified)
#
# Usage:  apps/node/scripts/stage-node-pkg-linux.sh
set -euo pipefail

VERSION="0.34.16"
TARBALL_URL="https://github.com/btxchain/btx/releases/download/v${VERSION}/btx-${VERSION}-x86_64-linux-gnu.tar.gz"
# Upstream has re-generated release assets in place before, and a silent swap
# must FAIL here, never ship unnoticed. v0.34.16 publishes a SHA256SUMS file
# again (unsigned); this is its line for this archive, it equals GitHub's
# asset digest, and a download on 2026-11-25 hashed to it. It pins the bytes,
# not a signature.
TARBALL_SHA256="9bbfff065299b3d6e863ecdeb0e54917788c1c0395cfa1d2b61bc40bf364932b"
# NOTE: upstream publishes no `aarch64-linux-gnu` asset, so there is no ARM-Linux
# node to stage. This script is x86_64-only by construction and always was; the
# gap is called out here so nobody spends an afternoon looking for the tarball.
# ⚠ THIS DOES NOT PRODUCE A GPU VALIDATOR, AND IT IS NOT THE RELEASE PATH.
#
# An earlier version of this comment said the plain build was fine because "the
# node app does not mine". That reasoning is wrong and worth correcting in
# place: the node app does not mine, but it DOES validate, and since the MatMul
# v4.7 fork validation is the thing that needs the GPU. A node staged from the
# plain tarball has no CUDA backend, so it can never advertise
# NODE_MATMUL_CONSENSUS no matter what card is in the machine.
#
# Two separate reasons this tarball is a developer convenience only:
#
#   1. No CUDA. Upstream also publishes a -cuda13 archive for that (and a
#      -cuda12 one up to v0.34.9), which this deliberately does not fetch,
#      because see (2). v0.34.9's CUDA fatbins are also Blackwell-only
#      (sm_120), per its release notes.
#   2. glibc. The official Linux binaries need glibc 2.38, and Ubuntu LTS
#      machines do not have it, so they will not run for most people anyway.
#
# The SHIPPED Linux app is built from the official source tag on Ubuntu 22.04
# and carries its own GPU maths library and kernels (sm_75 through sm_120)
# inside the package, which is why the AppImage is around 445 MB. See the 0.6.17
# entry in apps/node/CHANGELOG.md.
#
# So: use this to get a Linux node running for development. Do not use it to
# build something you intend to validate with, and do not use it for a release.

APP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="$APP_DIR/src-tauri/resources/node-pkg"

# Refuse to stage a version the app will then refuse. See scripts/lib/engine-pin.sh.
# shellcheck source=lib/engine-pin.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib/engine-pin.sh"
assert_matches_engine_pin "$APP_DIR" "$VERSION"
# Both upstream archive formats (0.34 .tar.gz, 0.35 .tar.xz + SHA384), the
# layout gate and the exact-version check. See the file.
# shellcheck source=lib/release-archive.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib/release-archive.sh"

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "error: this stages the LINUX node package and must run on Linux." >&2
  echo "       macOS staging is stage-node-pkg.sh." >&2
  exit 1
fi

SRC="${EASYBTX_NODE_PKG_SRC:-}"
if [[ -z "$SRC" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  echo "==> downloading $TARBALL_URL"
  curl -fsSL --retry 3 -o "$tmp/pkg.archive" "$TARBALL_URL"
  release_archive_verify "$tmp/pkg.archive" "$TARBALL_SHA256"
  SRC="$(release_archive_extract "$tmp/pkg.archive" "$tmp/x")"
fi

if [[ ! -x "$SRC/bin/btxd" ]]; then
  echo "error: no BTX node package at $SRC (expected bin/btxd)." >&2
  echo "       Set EASYBTX_NODE_PKG_SRC to an extracted release package dir." >&2
  exit 1
fi
# The copy below is libexec/*.real-shaped; refuse 0.35's layout until adapted.
require_034_layout "$SRC"
if ! btxd_reports_exact_version "$SRC/bin/btxd" "$VERSION"; then
  echo "error: $SRC/bin/btxd is not exactly v$VERSION (reported: $("$SRC/bin/btxd" --version 2>&1 | head -1))" >&2
  exit 1
fi

rm -rf "$DEST"
mkdir -p "$DEST"
# Only the runtime tree: bin/ wrappers + libexec/ daemons, and from lib/ only
# the bundled OpenSSL when the archive carries it (see the header). contrib/
# and doc/ are source-repo extras the app never reads.
cp -R "$SRC/bin" "$DEST/bin"
cp -R "$SRC/libexec" "$DEST/libexec"
for ssl in "$SRC"/lib/libssl.so* "$SRC"/lib/libcrypto.so*; do
  [ -e "$ssl" ] || continue
  mkdir -p "$DEST/lib"
  cp -P "$ssl" "$DEST/lib/"
done
# From 0.34.16 libexec/btxd.real links libpython for the SwapNet worker, also
# when it runs with -swapnet=0, and upstream's archive carries that runtime in
# lib/: libpython3.X.so.1.0, the standard library in python3.X/, third-party
# modules at the build host's site path (python3/dist-packages on a Debian
# host) and the libraries only those modules load in python-private/. btxd
# looks for exactly that shape beside the libpython it loaded
# (src/swapnet/embed.cpp), so it is copied as it is. An archive without it,
# which is 0.34.15 and everything older, copies nothing here.
# CHECKED 2026-11-25 against the listing of the 0.34.16 archive, not by a
# run: its lib/ holds libpython3.14.so.1.0, python3.14/, python3/dist-packages/
# (cryptography, cffi, bcrypt) and python-private/, which are the names the
# loop below copies. The copy itself has not been run to the end: this script
# asks the archive's btxd for its version first, that binary needs glibc 2.38,
# and the machine the listing was read on is Ubuntu 22.04.
for py in "$SRC"/lib/libpython3.*.so* "$SRC"/lib/python3* "$SRC"/lib/python-private; do
  [ -e "$py" ] || continue
  mkdir -p "$DEST/lib"
  cp -RP "$py" "$DEST/lib/"
done
# Without the model plane, exactly as stage-node-pkg.sh does and for the same
# reason: the archive is built WITH_MODELNET=ON, an ON btxd starts btx-modeld on
# 0.0.0.0:29447 by itself, the release engine is built OFF, and the app calls
# none of these. Without btx-modeld btxd logs that it is missing and continues.
# The model plane is still compiled into this btxd; only the helper is gone.
for helper in btx-modeld btx-modelcheck btx-open btx-capability btx-capabilityd btx-hcpd btx-hosted; do
  rm -f "$DEST/bin/$helper" "$DEST/libexec/$helper.real"
done
# ...and without the GUI v0.34.12 added (bin/btx-qt, libexec/btx-qt.real and
# the qt.conf beside it). The app never starts it, and its Qt runtime in lib/
# is not staged above.
rm -f "$DEST/bin/btx-qt" "$DEST/libexec/btx-qt.real" "$DEST/libexec/qt.conf"
chmod +x "$DEST"/bin/* "$DEST"/libexec/*

echo "==> staged node package: $(du -sh "$DEST" | cut -f1) at $DEST"
"$DEST/bin/btxd" --version 2>/dev/null | head -1 || {
  echo "error: staged bin/btxd failed to run" >&2
  exit 1
}

# Declare which btxd this package carries, in the marker provisioning reads.
#
# WITHOUT this file, provision_node_package derives the expected version from
# the INSTALL DIRECTORY (i.e. NODE_RELEASE_TAG) and rejects a btxd reporting
# anything else. Those legitimately differ for a branch build — BTX's
# `pr/0.33.3-network-stability` never bumped CLIENT_VERSION_BUILD, so it reports
# v0.33.2 while our install tag must move for re-provisioning to happen — and
# the mismatch refuses the whole tree on the USER's machine, after the ~450 MB
# snapshot download. Harmless for a plain tagged release (marker == tag).
# See crates/btx-core/src/installer.rs (BTXD_VERSION_MARKER).
# The regex must accept FOUR segments. `v[0-9]+\.[0-9]+\.[0-9]+` truncates a reseal
# tag: btxd v0.33.4.1 reports "BTX daemon version v0.33.4.1" and that pattern
# captures `v0.33.4`. Provisioning compares this marker against the binary's
# banner with a whole-token match, so every install is then refused as "not
# v0.33.4" on a package that is in fact exactly right - and on the returning-user
# upgrade path that refusal is SWALLOWED, so the app just keeps launching the
# old tag. The mac scripts were fixed; these were not. See commands.rs, which
# states the invariant in prose: "keep the staging script's regex able to
# capture four segments".
if ! "$DEST/bin/btxd" --version 2>/dev/null | head -1 \
     | grep -oE 'v[0-9]+(\.[0-9]+)+' > "$DEST/.btxd-version"; then
  echo "error: could not parse a version out of bin/btxd --version" >&2
  exit 1
fi
echo "==> declares btxd $(cat "$DEST/.btxd-version")"
