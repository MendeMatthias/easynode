#!/usr/bin/env bash
# Stage the bundled BTX node package for an easyBTX Node build — and make it
# SELF-CONTAINED (no Homebrew on the user's machine).
#
# The app bundles the WHOLE v0.33.2 release package tree (bin/ launcher
# wrappers + libexec/*.real Mach-O daemons + metal shader libs, ~25 MB) under
# src-tauri/resources/node-pkg/. The tree is gitignored (same rationale as the
# miner's resources/bin + resources/node-bin: keep binaries out of the source
# tree) and must be staged before `tauri build` / `tauri dev`.
#
# Upstream's darwin build links libevent from Homebrew (/opt/homebrew or
# /usr/local); everything else it needs is a system library. A clean user Mac
# has no Homebrew, so we vendor those dylibs into libexec/lib/ and rewrite the
# load commands to @loader_path — the wrapper's own dependency check skips
# @-prefixed entries, and install_name_tool + codesign keep the Mach-Os valid.
#
# Source resolution order:
#   1. $EASYBTX_NODE_PKG_SRC (explicit override)
#   2. ~/btx-node-research/btx-0.33.2  (the live-proven local package)
#
# Usage:  apps/node/scripts/stage-node-pkg.sh
set -euo pipefail

APP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="$APP_DIR/src-tauri/resources/node-pkg"

VERSION="0.33.2"
TARBALL_URL="https://github.com/btxchain/btx/releases/download/v${VERSION}/btx-${VERSION}-arm64-apple-darwin.tar.gz"
# From the release's signed SHA256SUMS. Upstream has re-generated release assets
# in place before — a silent swap must FAIL here, never ship unnoticed.
TARBALL_SHA256="d48e87c6e032db6322cbce3487993afc295cd235ee49183687efe649c72b5ed7"

SRC="${EASYBTX_NODE_PKG_SRC:-}"
if [[ -z "$SRC" && -x "$HOME/btx-node-research/btx-$VERSION/bin/btxd" ]]; then
  SRC="$HOME/btx-node-research/btx-$VERSION"
fi
if [[ -z "$SRC" ]]; then
  # No local package: fetch the pinned release, exactly like the Linux leg.
  # `shasum -a 256`, NOT `sha256sum` — the latter is not on stock macOS.
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  echo "==> downloading $TARBALL_URL"
  curl -fsSL --retry 3 -o "$tmp/pkg.tgz" "$TARBALL_URL"
  echo "$TARBALL_SHA256  $tmp/pkg.tgz" | shasum -a 256 -c -
  tar -xzf "$tmp/pkg.tgz" -C "$tmp"
  SRC="$tmp/btx-$VERSION"
fi

if [[ ! -x "$SRC/bin/btxd" ]]; then
  echo "error: no BTX node package at $SRC (expected bin/btxd)." >&2
  echo "       Set EASYBTX_NODE_PKG_SRC to the extracted btx-$VERSION release dir" >&2
  echo "       (the extracted release tar: bin/ + libexec/)." >&2
  exit 1
fi

# The tree must BE v$VERSION. `NODE_RELEASE_TAG` in commands.rs decides the
# install path, and node.rs derives the version-gated CLI flags from that path —
# so staging the wrong bytes here arms `-matmulrcexecution` against a btxd that
# rejects it fatally. Cheap to check, expensive to miss.
if ! "$SRC/bin/btxd" --version 2>/dev/null | grep -q "v${VERSION}"; then
  echo "error: $SRC/bin/btxd is not v$VERSION (reported: $("$SRC/bin/btxd" --version 2>&1 | head -1))" >&2
  exit 1
fi

rm -rf "$DEST"
mkdir -p "$DEST"
# Only the runtime tree: bin/ wrappers + libexec/ daemons+shaders. contrib/ and
# doc/ are source-repo extras the app never reads.
cp -R "$SRC/bin" "$DEST/bin"
cp -R "$SRC/libexec" "$DEST/libexec"

# ── Vendor non-system dylibs TRANSITIVELY (Homebrew) ────────────────────────
# The upstream darwin build links some libs from Homebrew and a clean user Mac
# has none. We copy the full TRANSITIVE closure of Homebrew deps into
# libexec/lib/ — transitive is required, e.g. btxd → libzmq → libsodium (which
# v0.33.1 pulled in, since it kept ZMQ enabled). The closure is computed at stage
# time rather than listed, which is what let v0.33.2 change its dependency set
# underneath us for free: it drops ZMQ entirely and adds libomp (OpenMP), so the
# v0.33.2 mac closure is libevent{_core,_extra,_pthreads} + libomp + libsqlite3.
# Then rewrite every reference to a @loader_path-relative
# path and re-sign, so the bundle runs with no Homebrew. Note the two different
# rewrite targets below: the .real binaries live in libexec/ (→ @loader_path/lib/…)
# while the vendored dylibs live in libexec/lib/ and reference each other in the
# SAME dir (→ @loader_path/…). Getting that wrong sends the loader to
# libexec/lib/lib/… and the daemon won't start.
LIBDIR="$DEST/libexec/lib"
mkdir -p "$LIBDIR"

# Print the Homebrew-prefixed dependencies of a Mach-O (one per line).
hb_deps() {
  otool -L "$1" | awk 'NR>1 && ($1 ~ /^\/opt\/homebrew\// || $1 ~ /^\/usr\/local\/opt\//) {print $1}'
}

# Fixed-point copy of the transitive closure (bash 3.2 safe: no assoc arrays).
# Homebrew paths never contain spaces, so word-splitting the dep list is fine.
for _pass in 1 2 3 4 5 6 7 8; do
  copied=0
  for f in "$DEST"/libexec/*.real "$LIBDIR"/*.dylib; do
    [ -e "$f" ] || continue
    for dep in $(hb_deps "$f"); do
      name="$(basename "$dep")"
      [ -f "$LIBDIR/$name" ] && continue
      if [[ ! -f "$dep" ]]; then
        echo "error: $dep is linked (transitively) but not installed on this machine." >&2
        echo "       brew install the matching formula (e.g. libevent sqlite zeromq) and retry." >&2
        exit 1
      fi
      cp "$dep" "$LIBDIR/$name"
      chmod 644 "$LIBDIR/$name"
      copied=$((copied + 1))
    done
  done
  [ "$copied" -eq 0 ] && break
done

# Rewrite each vendored dylib's own id (cosmetic; deps are rewritten explicitly).
for lib in "$LIBDIR"/*.dylib; do
  install_name_tool -id "@loader_path/$(basename "$lib")" "$lib" 2>/dev/null
done
# .real binaries (in libexec/) → vendored dylibs (in libexec/lib/).
for f in "$DEST"/libexec/*.real; do
  for dep in $(hb_deps "$f"); do
    install_name_tool -change "$dep" "@loader_path/lib/$(basename "$dep")" "$f" 2>/dev/null
  done
done
# vendored dylibs reference their siblings in the SAME dir.
for lib in "$LIBDIR"/*.dylib; do
  for dep in $(hb_deps "$lib"); do
    install_name_tool -change "$dep" "@loader_path/$(basename "$dep")" "$lib" 2>/dev/null
  done
done

# Re-sign everything install_name_tool touched (dylibs + binaries); the app
# re-signs again at provision time, but the staged tree must be runnable for the
# sanity check below.
for lib in "$LIBDIR"/*.dylib; do codesign -f -s - "$lib"; done
for f in "$DEST"/libexec/*.real; do codesign -f -s - "$f"; done

# ── Sanity: no Homebrew references anywhere in the tree (binaries OR dylibs) ─
if otool -L "$DEST"/libexec/*.real "$LIBDIR"/*.dylib 2>/dev/null | grep -E '/opt/homebrew/|/usr/local/opt/'; then
  echo "error: Homebrew-prefixed dependencies remain after patching (above)." >&2
  exit 1
fi

echo "==> staged node package: $(du -sh "$DEST" | cut -f1) at $DEST"
"$DEST/bin/btxd" --version 2>/dev/null | head -1 || {
  echo "error: staged bin/btxd failed to run" >&2
  exit 1
}
echo "==> self-contained: vendored $(ls "$LIBDIR" | wc -l | tr -d ' ') dylib(s) into libexec/lib/"

# ── Metal shader inventory (WARNING only, never fatal) ──────────────────────
# btxd names its Metal libraries at runtime, and upstream's v0.33.2 darwin
# tarball ships TWO of the FOUR it references: matmul_v4_accel_kernels and
# matmul_v4_bmx4_accel_kernels are absent though their build paths are baked
# into the binary. Those are the MatMul *v4* kernels, i.e. exactly the ones the
# v4.7 fork needs, so btxd compiles them from embedded source at runtime
# instead. That works — an M2 Pro completed a full 4096-dim RC episode with
# cpu_fallbacks=0 — but ONLY in a foreground process: a daemonised btxd cannot
# reach MTLCompilerService and silently falls back to CPU. easyBTX never passes
# -daemon (see crates/btx-core/src/node.rs), which is what keeps this safe.
#
# This is a warning, not a gate: failing here would block every Mac build with
# no remedy available to us. It exists so a future release that FIXES the
# packaging is noticed, and so nobody rediscovers this from scratch.
missing_metallibs=""
for ref in $(strings -a "$DEST"/libexec/btxd.real 2>/dev/null \
             | grep -oE '/metal/[A-Za-z0-9_]+\.metallib' | sort -u); do
  [[ -f "$DEST/libexec${ref}" ]] || missing_metallibs="$missing_metallibs ${ref#/metal/}"
done
if [[ -n "$missing_metallibs" ]]; then
  echo "warning: upstream ships no metallib for:$missing_metallibs" >&2
  echo "         btxd will compile these shaders at runtime. Expected for v$VERSION." >&2
  echo "         Requires a FOREGROUND btxd — never add -daemon." >&2
fi
