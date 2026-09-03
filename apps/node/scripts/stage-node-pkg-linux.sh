#!/usr/bin/env bash
# Stage the bundled BTX node package for an easyBTX Node LINUX build.
#
# Mirrors stage-node-pkg.sh (macOS) but is far simpler: the upstream Linux
# release binaries are FULLY STATIC ELFs (verified: `file` reports
# "statically linked" for libexec/*.real), so there is no .so vendoring or
# rpath pass — download, verify the pinned sha256, copy, sanity-run.
#
# Source resolution order:
#   1. $EASYBTX_NODE_PKG_SRC (explicit override: an extracted package dir)
#   2. download the pinned upstream release tarball (sha256-verified)
#
# Usage:  apps/node/scripts/stage-node-pkg-linux.sh
set -euo pipefail

VERSION="0.33.2"
TARBALL_URL="https://github.com/btxchain/btx/releases/download/v${VERSION}/btx-${VERSION}-x86_64-linux-gnu.tar.gz"
# From the release's signed SHA256SUMS. Upstream has re-generated release
# assets in place before — a silent swap must FAIL here, never ship unnoticed.
TARBALL_SHA256="3bc67d222f2afa7607b91ba87856206b9975afc5a4c3aec9fe782a26fc9f4310"
# NOTE: v0.33.2 dropped the `aarch64-linux-gnu` asset that v0.33.1 published, so
# there is no upstream ARM-Linux node to stage for the MatMul v4.7 fork. This
# script is x86_64-only by construction and always was; the gap is called out
# here so nobody spends an afternoon looking for the ARM tarball.

APP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="$APP_DIR/src-tauri/resources/node-pkg"

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
# Only the runtime tree: bin/ wrappers + libexec/ static daemons. contrib/ and
# doc/ are source-repo extras the app never reads.
cp -R "$SRC/bin" "$DEST/bin"
cp -R "$SRC/libexec" "$DEST/libexec"
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
if ! "$DEST/bin/btxd" --version 2>/dev/null | head -1 \
     | grep -oE 'v[0-9]+\.[0-9]+\.[0-9]+' > "$DEST/.btxd-version"; then
  echo "error: could not parse a version out of bin/btxd --version" >&2
  exit 1
fi
echo "==> declares btxd $(cat "$DEST/.btxd-version")"
