#!/usr/bin/env bash
# Stage a SOURCE-BUILT btxd for the macOS BTX Node app.
#
# Why this exists: `stage-node-pkg.sh` downloads a published upstream release
# tarball, which is the right default — but during a network incident the fix we
# need may only exist on a branch or an unmerged PR. This script takes a btxd you
# compiled yourself and produces the same self-contained node-pkg tree, so the app
# can ship a node that upstream has not tagged yet.
#
# Build btxd first (from a checkout of btxchain/btx at whatever ref you need):
#
#   cmake -B build -G Ninja -DCMAKE_BUILD_TYPE=Release \
#     -DBUILD_DAEMON=ON -DBUILD_CLI=ON \
#     -DBUILD_UTIL=OFF -DBUILD_TX=OFF -DBUILD_WALLET_TOOL=OFF \
#     -DBUILD_GUI=OFF -DBUILD_BENCH=OFF -DBUILD_TESTS=OFF -DBUILD_FUZZ_BINARY=OFF \
#     -DENABLE_WALLET=ON -DWITH_SQLITE=ON \
#     -DBTX_ENABLE_METAL=ON -DBTX_MATMUL_METAL_PRECOMPILE_KERNELS=OFF
#   cmake --build build -j$(sysctl -n hw.ncpu)
#
# ★ PRECOMPILE_KERNELS=OFF matters: compiling the Metal shaders at build time
#   needs `xcrun metal`, which ships with full Xcode and NOT with the Command Line
#   Tools. With it OFF, btxd compiles its shaders at runtime instead — which is
#   what upstream's own release effectively does anyway (it references four
#   metallibs and ships two). Verified: an M2 Pro passes the RC production canary
#   this way with cpu_fallbacks=0.
#
# ⚠ Runtime shader compilation needs a FOREGROUND btxd. A daemonised process
#   cannot reach MTLCompilerService and silently falls back to CPU. The app never
#   passes -daemon; keep it that way.
#
# Usage:
#   stage-node-pkg-mac-source.sh <btx-build-dir> [expected-version]
#   e.g. stage-node-pkg-mac-source.sh ~/src/btx/build v0.33.3
set -euo pipefail

SRC_BIN="${1:?path to the btx build dir (containing bin/btxd)}"
# Keep the ROOT before descending into bin/. Since upstream 0.34.1 the macOS
# release ships bin/btxd as a #!/bin/sh wrapper that execs ../libexec/btxd.real,
# so the real ELF lives one level up from SRC_BIN and copying only bin/ produces
# a package whose btxd exits 127 with "BTX packaged binary is missing".
SRC_ROOT="$SRC_BIN"
[[ -d "$SRC_BIN/bin" ]] && SRC_BIN="$SRC_BIN/bin"
EXPECT="${2:-}"

APP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="$APP_DIR/src-tauri/resources/node-pkg"

# The engine pin, defaulted rather than optional.
#
# ⚠ THIS IS THE RELEASE PATH, and until 2026-09-04 it was the ONE staging path
# with no pin check at all. `EXPECT` was optional, so omitting the argument
# staged whatever btxd happened to be in the build dir, and the app then
# installed it into a directory named after NODE_RELEASE_TAG and derived its
# version-gated flags from that name. The guard added to the two upstream
# tarball scripts did not cover this one, which is the only one a release
# actually uses.
#
# So: with no argument, the expected version IS the pin. With an argument, the
# argument must agree with the pin. There is no longer a way to stage a release
# engine without saying which one and being checked on it.
# shellcheck source=lib/engine-pin.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib/engine-pin.sh"
if [[ -z "$EXPECT" ]]; then
  EXPECT="$(engine_pin_tag "$APP_DIR")"
  echo "==> no version given; using the engine pin $EXPECT"
else
  assert_matches_engine_pin "$APP_DIR" "${EXPECT#v}"
fi

for f in btxd btx-cli; do
  [[ -x "$SRC_BIN/$f" ]] || { echo "error: $SRC_BIN/$f not found or not executable" >&2; exit 1; }
done

if [[ -n "$EXPECT" ]] && ! "$SRC_BIN/btxd" --version 2>/dev/null | grep -q "$EXPECT"; then
  echo "error: btxd is not $EXPECT (reports: $("$SRC_BIN/btxd" --version 2>&1 | head -1))" >&2
  echo "       NODE_RELEASE_TAG decides the install path and node.rs derives its" >&2
  echo "       version-gated flags from it, so a mismatch here breaks the launch." >&2
  exit 1
fi

rm -rf "$DEST"; mkdir -p "$DEST/bin" "$DEST/bin/lib"
cp "$SRC_BIN/btxd" "$SRC_BIN/btx-cli" "$DEST/bin/"
chmod +x "$DEST/bin/btxd" "$DEST/bin/btx-cli"

# Carry libexec/ when the source has it. A source-built tree usually does not,
# so its absence is normal and not an error. An upstream release tarball always
# does, and without this the staged wrapper has nothing to exec.
if [[ -d "$SRC_ROOT/libexec" ]]; then
  mkdir -p "$DEST/libexec"
  cp "$SRC_ROOT/libexec/"* "$DEST/libexec/"
  chmod +x "$DEST/libexec/"* 2>/dev/null || true
  echo "==> carried libexec/ ($(ls "$DEST/libexec" | tr '\n' ' '))"
fi

# ── Vendor the transitive Homebrew closure ─────────────────────────────────
# A clean user Mac has no Homebrew. Same fixed-point walk as stage-node-pkg.sh:
# transitive matters (a vendored dylib can itself pull another). Binaries and
# their libs end up in the same tree, so both rewrite to @loader_path/lib/… and
# the libs reference each other at @loader_path/… (they share a directory).
LIBDIR="$DEST/bin/lib"
hb_deps() { otool -L "$1" | awk 'NR>1 && ($1 ~ /^\/opt\/homebrew\// || $1 ~ /^\/usr\/local\/opt\//) {print $1}'; }

changed=1
while [[ $changed -eq 1 ]]; do
  changed=0
  while IFS= read -r dep; do
    [[ -z "$dep" ]] && continue
    base="$(basename "$dep")"
    if [[ ! -f "$LIBDIR/$base" ]]; then
      cp "$dep" "$LIBDIR/$base"; chmod u+w "$LIBDIR/$base"; changed=1
    fi
  done < <({ for b in "$DEST"/bin/btxd "$DEST"/bin/btx-cli; do hb_deps "$b"; done
             for l in "$LIBDIR"/*.dylib; do [[ -f "$l" ]] && hb_deps "$l"; done; } | sort -u)
done

# Rewrite: executables in bin/ point at @loader_path/lib/<name>; the vendored
# dylibs share one directory, so they point at @loader_path/<name>.
for b in "$DEST"/bin/btxd "$DEST"/bin/btx-cli; do
  while IFS= read -r dep; do
    [[ -z "$dep" ]] && continue
    install_name_tool -change "$dep" "@loader_path/lib/$(basename "$dep")" "$b"
  done < <(hb_deps "$b")
done
for l in "$LIBDIR"/*.dylib; do
  [[ -f "$l" ]] || continue
  install_name_tool -id "@loader_path/$(basename "$l")" "$l"
  while IFS= read -r dep; do
    [[ -z "$dep" ]] && continue
    install_name_tool -change "$dep" "@loader_path/$(basename "$dep")" "$l"
  done < <(hb_deps "$l")
done

# install_name_tool invalidates signatures; the app re-signs at provision time
# too, but the staged tree must be runnable for the check below.
for f in "$LIBDIR"/*.dylib "$DEST"/bin/btxd "$DEST"/bin/btx-cli; do
  [[ -f "$f" ]] && codesign -f -s - "$f" >/dev/null 2>&1 || true
done

# ── Sanity: nothing may still reach for Homebrew, and it must run ──────────
if otool -L "$DEST"/bin/btxd "$DEST"/bin/btx-cli "$LIBDIR"/*.dylib 2>/dev/null \
   | grep -E '/opt/homebrew/|/usr/local/opt/'; then
  echo "error: Homebrew-prefixed dependencies remain after patching (above)." >&2
  exit 1
fi

# Declare which btxd this package carries. The install tag and the reported
# version legitimately differ for branch builds (BTX's 0.33.3 PR never bumped
# CLIENT_VERSION_BUILD), and provisioning verifies against THIS, not the tag.
#
# ⚠ The regex must accept FOUR segments. It was `v[0-9]+\.[0-9]+\.[0-9]+`, which
# truncates a reseal tag: btxd v0.33.4.1 reports "BTX daemon version v0.33.4.1"
# and that pattern captured `v0.33.4`. Provisioning then compares the marker
# against the binary's banner with a whole-token match, so `v0.33.4` never
# equals `v0.33.4.1` and every install is refused as "not v0.33.4" — on a
# package that is in fact exactly right.
# ⚠ This block used to be `... 2>/dev/null | head -1 | grep -oE ...` and it
# COULD NOT FAIL. When the staged btxd could not run, stderr went to /dev/null,
# grep matched nothing, .btxd-version was written EMPTY, and the script carried
# on and reported success. That is exactly how a package whose btxd exits 127
# would have shipped. Run it first, on its own line, and check the output is a
# real version before anything else happens.
STAGED_BANNER="$("$DEST/bin/btxd" --version 2>&1 | head -1)" || true
if ! grep -qE 'v[0-9]+(\.[0-9]+)+' <<<"$STAGED_BANNER"; then
  echo "error: the STAGED btxd does not run. It reported:" >&2
  echo "       $STAGED_BANNER" >&2
  echo "       Staging copies bin/ and, when present, libexec/. Since upstream" >&2
  echo "       0.34.1 bin/btxd is a wrapper that execs ../libexec/btxd.real, so" >&2
  echo "       a missing libexec/ produces exactly this." >&2
  exit 1
fi
grep -oE 'v[0-9]+(\.[0-9]+)+' <<<"$STAGED_BANNER" > "$DEST/.btxd-version"
echo "==> declares btxd $(cat "$DEST/.btxd-version")"

echo "==> staged $(du -sh "$DEST" | cut -f1) at $DEST"
"$DEST/bin/btxd" --version | head -1
echo "==> self-contained: vendored $(ls "$LIBDIR" | wc -l | tr -d ' ') dylib(s)"
echo "==> NOTE: no precompiled metallibs — btxd builds its Metal shaders at"
echo "    runtime. That requires a FOREGROUND btxd (never -daemon)."
