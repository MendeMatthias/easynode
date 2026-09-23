#!/usr/bin/env bash
# Stage the bundled BTX node package for an easyBTX Node LINUX build.
#
# Mirrors stage-node-pkg.sh (macOS) but is far simpler: no .so vendoring or
# rpath pass — download, verify the pinned sha256, copy, sanity-run.
#
# The upstream Linux binaries USED to be fully static ELFs. v0.34.9's are not:
# libexec/btxd.real NEEDs libssl.so.3 and libcrypto.so.3 (OpenSSL 3.5, bundled
# in the archive's top-level lib/), plus libzmq, libevent, sqlite and libgomp
# from the host. bin/btxd, the wrapper, puts ../lib on LD_LIBRARY_PATH when it
# finds libssl.so.3 there, so lib/ is staged beside bin/ and libexec/.
#
# Source resolution order:
#   1. $EASYBTX_NODE_PKG_SRC (explicit override: an extracted package dir)
#   2. download the pinned upstream release tarball (sha256-verified)
#
# Usage:  apps/node/scripts/stage-node-pkg-linux.sh
set -euo pipefail

VERSION="0.34.9"
TARBALL_URL="https://github.com/btxchain/btx/releases/download/v${VERSION}/btx-${VERSION}-x86_64-linux-gnu.tar.gz"
# From the release's SHA256SUMS. Upstream has re-generated release assets in
# place before — a silent swap must FAIL here, never ship unnoticed. v0.34.9
# publishes that file UNSIGNED (no SHA256SUMS.asc), so this pins the bytes, not
# a signature.
TARBALL_SHA256="cf7a68e5aad53aad80a8d8eb04d0ba552c319c466dcf34af4c62fc41b0c20f5f"
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
#   1. No CUDA. Upstream also publishes -cuda12 and -cuda13 archives for that,
#      which this deliberately does not fetch, because see (2). v0.34.9's CUDA
#      fatbins are also Blackwell-only (sm_120), per its release notes.
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
  curl -fsSL --retry 3 -o "$tmp/pkg.tgz" "$TARBALL_URL"
  echo "$TARBALL_SHA256  $tmp/pkg.tgz" | sha256sum -c -
  tar -xzf "$tmp/pkg.tgz" -C "$tmp"
  SRC="$tmp/btx-$VERSION"
fi

if [[ ! -x "$SRC/bin/btxd" ]]; then
  echo "error: no BTX node package at $SRC (expected bin/btxd)." >&2
  echo "       Set EASYBTX_NODE_PKG_SRC to an extracted release package dir." >&2
  exit 1
fi

rm -rf "$DEST"
mkdir -p "$DEST"
# Only the runtime tree: bin/ wrappers + libexec/ daemons, and lib/ when the
# archive carries one (v0.34.9's bundled OpenSSL; see the header). contrib/ and
# doc/ are source-repo extras the app never reads.
cp -R "$SRC/bin" "$DEST/bin"
cp -R "$SRC/libexec" "$DEST/libexec"
if [[ -d "$SRC/lib" ]]; then
  cp -R "$SRC/lib" "$DEST/lib"
fi
# Without the model plane, exactly as stage-node-pkg.sh does and for the same
# reason: the archive is built WITH_MODELNET=ON, an ON btxd starts btx-modeld on
# 0.0.0.0:29447 by itself, the release engine is built OFF, and the app calls
# none of these. Without btx-modeld btxd logs that it is missing and continues.
for helper in btx-modeld btx-modelcheck btx-open btx-capability btx-capabilityd btx-hcpd btx-hosted; do
  rm -f "$DEST/bin/$helper" "$DEST/libexec/$helper.real"
done
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
