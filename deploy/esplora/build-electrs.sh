#!/usr/bin/env bash
# Build electrs (the BTX fork vendored beside this script) and install it.
#
# Ported from btx-esplora's deploy-to-vm.sh, minus the Azure half: that script
# rsync'd the sources to a VM and built there. This one builds where it runs.
#
# Usage:
#   deploy/esplora/build-electrs.sh                       # -> /usr/local/bin/electrs (sudo if needed)
#   PREFIX=$HOME/.local deploy/esplora/build-electrs.sh   # no root anywhere
#   JOBS=3 deploy/esplora/build-electrs.sh                # leave cores for btxd
#
# What it needs, and why:
#   - rustup. electrs/rust-toolchain.toml pins the toolchain (1.92.0 at the time
#     of the port) and rustup installs it on first use. A distro cargo is too
#     old: the Azure build found Ubuntu's 1.75 refusing electrs outright.
#   - a C++ compiler, cmake and libclang. rocksdb is compiled from source by
#     librocksdb-sys, and bindgen needs libclang for the bindings.
#       Debian/Ubuntu:  sudo apt install clang cmake build-essential
#     No root? `pip3 install --user libclang` ships a libclang.so, and this
#     script points LIBCLANG_PATH at it when it finds one. That library comes
#     without clang's builtin headers, so bindgen then fails on `stdbool.h`;
#     the script adds gcc's include directory, which has it. Verified on the
#     release box (Ubuntu 22.04, no sudo) on 2026-09-06.
#
# The first build compiles rocksdb and takes 10-20 minutes. Later builds are
# incremental.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
PREFIX="${PREFIX:-/usr/local}"
export PATH="$HOME/.cargo/bin:$PATH"

command -v cargo >/dev/null 2>&1 || {
  echo "cargo not found. Install rustup from https://rustup.rs and re-run." >&2
  exit 1
}

# libclang: the system one if present, else the pip one, else say what is missing.
if [ -z "${LIBCLANG_PATH:-}" ]; then
  # `ls a b` fails when EITHER pattern matches nothing, so this probe reported
  # "no system libclang" on a machine that had one in /usr/lib/x86_64-linux-gnu
  # but no /usr/lib/llvm-*/ directory — a normal Debian layout. It then fell
  # through to the pip library, which has no builtin headers, and the build
  # died on stdbool.h with a message about bindgen.
  have_system_libclang=0
  for pat in /usr/lib/llvm-*/lib/libclang*.so* /usr/lib/*/libclang*.so* /usr/lib64/libclang*.so*; do
    [ -e "$pat" ] && { have_system_libclang=1; break; }
  done
  if [ "$have_system_libclang" -eq 0 ]; then
    pyclang="$(python3 -c 'import clang, os; print(os.path.join(os.path.dirname(clang.__file__), "native"))' 2>/dev/null || true)"
    if [ -n "$pyclang" ] && [ -e "$pyclang/libclang.so" ]; then
      export LIBCLANG_PATH="$pyclang"
      echo "libclang: using $pyclang (pip install --user libclang)"
      # The pip library has no builtin headers. gcc's directory carries
      # stdbool.h, stddef.h and friends; hand it to bindgen's clang.
      gccinc="$(ls -d /usr/lib/gcc/*/*/include 2>/dev/null | sort -V | tail -1 || true)"
      if [ -n "$gccinc" ] && [ -e "$gccinc/stdbool.h" ]; then
        export BINDGEN_EXTRA_CLANG_ARGS="${BINDGEN_EXTRA_CLANG_ARGS:-} -isystem $gccinc"
        echo "bindgen: builtin headers from $gccinc"
      else
        echo "no gcc include directory with stdbool.h found; bindgen will probably fail" >&2
      fi
    else
      echo "libclang not found. Either 'sudo apt install clang cmake build-essential'" >&2
      echo "or, without root, 'pip3 install --user libclang' and re-run." >&2
      exit 1
    fi
  fi
fi

cd "$HERE/electrs"
echo "toolchain: $(rustc --version) / $(cargo --version)"
echo "building electrs --release (rocksdb from source; the first build is long)"
cargo build --release --bin electrs --locked ${JOBS:+-j "$JOBS"}

bin="$HERE/electrs/target/release/electrs"
[ -x "$bin" ] || { echo "build finished but $bin is missing" >&2; exit 1; }

mkdir -p "$PREFIX/bin" 2>/dev/null || true
if [ -w "$PREFIX/bin" ]; then
  install -m755 "$bin" "$PREFIX/bin/electrs"
else
  echo "installing to $PREFIX/bin needs root"
  sudo install -m755 "$bin" "$PREFIX/bin/electrs"
fi
echo "installed $PREFIX/bin/electrs"
# The fork prints upstream's version string; the line is informational.
"$PREFIX/bin/electrs" --version 2>/dev/null || true
