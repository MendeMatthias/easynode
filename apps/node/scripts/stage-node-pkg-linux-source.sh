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
#     -DENABLE_WALLET=ON -DWITH_SQLITE=ON -DWITH_MODELNET=OFF
#   cmake --build build -j"$(nproc)"
#
# ⚠ -DWITH_MODELNET=OFF from 0.34.7 on: upstream defaults the model network ON,
#   and ON refuses to configure below OpenSSL 3.5, which 22.04 does not have.
#
# ⚠ From 0.34.16 btxd embeds Python for its SwapNet worker (cmake/SwapNet.cmake)
#   and there is no switch for it: the build needs a Python >= 3.11 WITH headers
#   and the `cryptography` module, and 22.04's own Python is 3.10. Add
#   -DPython3_EXECUTABLE=<that python> to the line above. btxd then links
#   libpython3.X, also when run with -swapnet=0, so the package has to carry
#   that library, its standard library and cryptography. This script does that:
#   from an engine artifact it takes the runtime the engine build already
#   bundled and tested (lib/python3.X next to btxd), from a hand build it
#   bundles the interpreter that owns the libpython btxd resolves to.
#   .github/workflows/btxd-linux.yml is the worked example, including the
#   pinned interpreter and the pinned cryptography wheel.
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
  # The VERSION part of the key, not the key: btxd prints `v0.34.6`, never our
  # `-3013c2c2` qualifier, and the banner check below must match what it prints.
  EXPECT="$(engine_pin_version "$APP_DIR")"
  echo "==> no version given; using the engine pin $(engine_pin_tag "$APP_DIR") (btxd reports $EXPECT)"
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

# ── Stage: binaries, shared-library closure, Python runtime, run paths ─────
# A source build is not static: it picks up whatever libevent, sqlite and zmq
# the build host had, cuBLASLt from the CUDA toolkit, and from 0.34.16
# libpython. A user's machine may have none of them, so everything that is not
# glibc or the C++ runtime travels in bin/lib/ and every ELF finds it through
# an $ORIGIN run path.
#
# The code is in lib/engine-runtime-linux.sh and NOT here, because the engine
# build (.github/workflows/btxd-linux.yml) has to produce this same tree in
# order to start it on a clean machine before it uploads it. When the source is
# such an artifact, its lib/ is taken whole: the libraries and the Python
# runtime staged below are then the files that build tested, not look-alikes
# from this machine.
# shellcheck source=lib/engine-runtime-linux.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib/engine-runtime-linux.sh"
rm -rf "$DEST"; mkdir -p "$DEST"
engine_stage_tree "$SRC_BIN" "$DEST/bin"
LIBDIR="$DEST/bin/lib"
VENDORED=0
if [[ -d "$LIBDIR" ]]; then
  VENDORED="$(find "$LIBDIR" -maxdepth 1 -name '*.so*' -type f | wc -l | tr -d ' ')"
fi

# ── Sanity: every ELF in the tree, not only the two binaries ───────────────
# Nothing unresolved, nothing but glibc and the C++ runtime taken from outside
# the tree, no run path of the build machine, and nothing that needs a glibc
# newer than Ubuntu 22.04's 2.35. Checked without LD_LIBRARY_PATH so a path the
# build host happens to export cannot mask a library missing on a user's
# machine. One line per file is printed: that table is the record of what the
# package asks of the machine it lands on.
engine_check_tree "$DEST/bin"

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
  for py in "$LIBDIR"/python3.*; do
    [[ -d "$py" ]] || continue
    echo "==> embedded Python runtime: bin/lib/$(basename "$py") ($(du -sh "$py" | cut -f1), $(find "$py" -type f | wc -l | tr -d ' ') files)"
  done
else
  echo "==> self-contained: no non-system shared libraries to vendor"
fi
echo "==> NOTE: this package is x86_64-only, like stage-node-pkg-linux.sh."
