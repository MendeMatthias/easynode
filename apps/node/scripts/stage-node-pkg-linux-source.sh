#!/usr/bin/env bash
# Stage a SOURCE-BUILT btxd for the LINUX BTX Node app.
#
# Why this exists: `stage-node-pkg-linux.sh` downloads a published upstream
# release tarball, which is the right default — but during a network incident
# the fix we need may only exist on a branch. This takes a btxd you compiled
# yourself and produces the same self-contained node-pkg tree, so the app can
# ship a node upstream has not tagged yet. Linux counterpart of
# stage-node-pkg-mac-source.sh.
#
# Build btxd first (from a checkout of btxchain/btx at whatever ref you need):
#
#   cmake -B build -G Ninja -DCMAKE_BUILD_TYPE=Release \
#     -DBUILD_DAEMON=ON -DBUILD_CLI=ON \
#     -DBUILD_UTIL=OFF -DBUILD_TX=OFF -DBUILD_WALLET_TOOL=OFF \
#     -DBUILD_GUI=OFF -DBUILD_BENCH=OFF -DBUILD_TESTS=OFF -DBUILD_FUZZ_BINARY=OFF \
#     -DENABLE_WALLET=ON -DWITH_SQLITE=ON
#   cmake --build build -j"$(nproc)"
#
# ⚠ Boost headers are needed even with BUILD_TESTS=OFF; pass
#   -DBoost_INCLUDE_DIR=<prefix>/usr/include if they are not on a system path.
#   libevent is resolved through a CMake CONFIG package first and only falls
#   back to pkg-config when none is found, so an unrelated LibeventConfig.cmake
#   on the search path (a Windows Anaconda under /mnt/c, in our case) is picked
#   up and fails. Keep /mnt/c off PATH when building under WSL.
#
# ⚠ Build from a PRISTINE checkout. btxd embeds whether its source tree was
#   dirty (cmake/script/GenerateBuildInfo.cmake runs
#   `git status --porcelain -uall -- CMakeLists.txt cmake src contrib/matmul-v4`)
#   and fails its own MatMul RC production canary with
#   `build_provenance_mismatch` -> ready=0 if it was. That node runs, syncs and
#   holds peers while silently NOT validating, and nothing on screen says so.
#   Editing one line of CMakeLists.txt is enough to trigger it.
#   An out-of-source `build/` dir at the repo root is NOT one of the watched
#   paths, so `cmake -B build` is safe.
#
# ⚠ The binary may report an OLDER version than the tag it ships under —
#   BTX's pr/0.33.3-network-stability never bumped CLIENT_VERSION_BUILD, so a
#   build of it says v0.33.2. That is why `.btxd-version` is written from what
#   btxd ACTUALLY reports: provisioning verifies against that file, not against
#   the install directory name (installer::provision_node_package).
#
# Usage:
#   stage-node-pkg-linux-source.sh <btx-build-dir> [expected-version]
#   e.g. stage-node-pkg-linux-source.sh ~/btx-src/btx/build v0.33.2
set -euo pipefail

SRC_BIN="${1:?path to the btx build dir (containing bin/btxd)}"
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

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "error: this stages the LINUX node package and must run on Linux." >&2
  echo "       macOS source staging is stage-node-pkg-mac-source.sh." >&2
  exit 1
fi
command -v patchelf >/dev/null || {
  echo "error: patchelf is required (apt install patchelf)." >&2; exit 1; }

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
LIBDIR="$DEST/bin/lib"

# ── Vendor the non-system shared-library closure ───────────────────────────
# Upstream's release ELFs are fully static; a local build is generally NOT —
# it picks up whatever libevent/sqlite the build host had. A user's machine
# may have neither, and certainly not at our build prefix's path.
#
# "System" = the glibc/gcc runtime every glibc Linux already has. Anything
# else (libevent, libsqlite3, and whatever those pull in) travels with us.
# Same transitive fixed-point walk as the macOS script: a vendored .so can
# itself pull another.
is_system_lib() {
  case "$1" in
    libc.so.*|libm.so.*|libdl.so.*|librt.so.*|libpthread.so.*|libresolv.so.*|\
    libgcc_s.so.*|libstdc++.so.*|libatomic.so.*|ld-linux*.so.*|linux-vdso.so.*) return 0 ;;
    *) return 1 ;;
  esac
}

# ldd lines look like `libfoo.so.1 => /path/to/libfoo.so.1 (0x…)`. Emit only
# resolved absolute paths of non-system libraries.
nonsystem_deps() {
  ldd "$1" 2>/dev/null | awk '{ for (i = 1; i <= NF; i++) if ($i == "=>" && $(i+1) ~ /^\//) print $1, $(i+1) }' \
    | while read -r soname path; do is_system_lib "$soname" || echo "$path"; done
}

changed=1
while [[ $changed -eq 1 ]]; do
  changed=0
  while IFS= read -r dep; do
    [[ -z "$dep" ]] && continue
    base="$(basename "$dep")"
    if [[ ! -f "$LIBDIR/$base" ]]; then
      cp -L "$dep" "$LIBDIR/$base"; chmod u+w "$LIBDIR/$base"; changed=1
    fi
  done < <({ for b in "$DEST"/bin/btxd "$DEST"/bin/btx-cli; do nonsystem_deps "$b"; done
             for l in "$LIBDIR"/*.so*; do [[ -f "$l" ]] && nonsystem_deps "$l"; done; } | sort -u)
done

VENDORED="$(find "$LIBDIR" -maxdepth 1 -name '*.so*' -type f | wc -l | tr -d ' ')"

# Executables live in bin/ and their libs in bin/lib/, so the binaries look at
# $ORIGIN/lib; the vendored libs share one directory and look at $ORIGIN.
# RUNPATH (the patchelf default) is NOT inherited by a dependency's own
# lookups, which is exactly why the libs need their own entry.
if [[ "$VENDORED" -gt 0 ]]; then
  for b in "$DEST"/bin/btxd "$DEST"/bin/btx-cli; do
    patchelf --set-rpath '$ORIGIN/lib' "$b"
  done
  for l in "$LIBDIR"/*.so*; do
    [[ -f "$l" ]] && patchelf --set-rpath '$ORIGIN' "$l"
  done
else
  rmdir "$LIBDIR"
fi

# ── Sanity: nothing unresolved, nothing reaching outside the tree ──────────
# Checked with an EMPTY LD_LIBRARY_PATH so a path the build host happens to
# export cannot mask a missing dependency on a user's machine.
for b in "$DEST"/bin/btxd "$DEST"/bin/btx-cli; do
  if out="$(env -u LD_LIBRARY_PATH ldd "$b" 2>&1)" && echo "$out" | grep -q "not found"; then
    echo "error: $b has unresolved libraries:" >&2
    echo "$out" | grep "not found" >&2
    exit 1
  fi
  while IFS= read -r p; do
    case "$p" in
      "$DEST"/*|/lib/*|/lib64/*|/usr/lib/*|/usr/lib64/*) ;;
      *) echo "error: $b still resolves $p from outside the staged tree." >&2; exit 1 ;;
    esac
  done < <(env -u LD_LIBRARY_PATH ldd "$b" 2>/dev/null \
             | awk '{ for (i = 1; i <= NF; i++) if ($i == "=>" && $(i+1) ~ /^\//) print $(i+1) }')
done

# Declare which btxd this package carries. The install tag and the reported
# version legitimately differ for branch builds (BTX's 0.33.3 PR never bumped
# CLIENT_VERSION_BUILD), and provisioning verifies against THIS, not the tag.
# The regex must accept FOUR segments. `v[0-9]+\.[0-9]+\.[0-9]+` truncates a reseal
# tag: btxd v0.33.4.1 reports "BTX daemon version v0.33.4.1" and that pattern
# captures `v0.33.4`. Provisioning compares this marker against the binary's
# banner with a whole-token match, so every install is then refused as "not
# v0.33.4" on a package that is in fact exactly right - and on the returning-user
# upgrade path that refusal is SWALLOWED, so the app just keeps launching the
# old tag. The mac scripts were fixed; these were not. See commands.rs, which
# states the invariant in prose: "keep the staging script's regex able to
# capture four segments".
env -u LD_LIBRARY_PATH "$DEST/bin/btxd" --version 2>/dev/null | head -1 \
  | grep -oE 'v[0-9]+(\.[0-9]+)+' > "$DEST/.btxd-version"
[[ -s "$DEST/.btxd-version" ]] || {
  echo "error: staged bin/btxd failed to run or reported no version" >&2; exit 1; }
echo "==> declares btxd $(cat "$DEST/.btxd-version")"

# The CLI ships in the same package and the app shells out to it; a package
# whose btx-cli cannot start is a broken package.
env -u LD_LIBRARY_PATH "$DEST/bin/btx-cli" --version >/dev/null 2>&1 || {
  echo "error: staged bin/btx-cli failed to run" >&2; exit 1; }

echo "==> staged $(du -sh "$DEST" | cut -f1) at $DEST"
env -u LD_LIBRARY_PATH "$DEST/bin/btxd" --version | head -1
if [[ "$VENDORED" -gt 0 ]]; then
  echo "==> self-contained: vendored $VENDORED shared object(s), RPATH \$ORIGIN/lib"
  (cd "$LIBDIR" && ls -1 *.so* | sed 's/^/    /')
else
  echo "==> self-contained: no non-system shared libraries to vendor"
fi
echo "==> NOTE: this package is x86_64-only, like stage-node-pkg-linux.sh."
