#!/usr/bin/env bash
# The Linux engine package: its shared-library closure, the Python runtime btxd
# embeds from 0.34.16 on, and the checks that decide whether it may ship.
#
# WHY THIS FILE EXISTS. Until 0.34.16 the closure logic lived inside
# stage-node-pkg-linux-source.sh and ran once, on the installer runner. The
# engine build (.github/workflows/btxd-linux.yml) could not use it, so nothing
# ever started the package on a machine that was not a build machine. From
# 0.34.16 btxd links libpython (cmake/SwapNet.cmake) and needs that
# interpreter's standard library and `cryptography` at run time, which is far
# too much to ship untested. So the layout, the vendoring, the run paths and
# the checks are here, and BOTH the engine build and the staging script call
# them. The bytes the engine build starts in a clean container are the bytes
# the installer stages.
#
# THE LAYOUT (unchanged for the binaries, new for Python):
#
#   bin/btxd, bin/btx-cli              RUNPATH $ORIGIN/lib
#   bin/lib/*.so*                      RUNPATH $ORIGIN      every non-glibc library
#   bin/lib/libpython3.X.so.1.0        RUNPATH $ORIGIN      0.34.16 on
#   bin/lib/python3.X/                 standard library, lib-dynload, site-packages
#
# The Python part is not ours to choose. btxd's src/swapnet/embed.cpp
# (PackagedPythonHome) asks the loader where the libpython it is running came
# from, and uses the directory ABOVE it as the Python home only when that
# library sits in a directory named `lib` with `python3.X/os.py` and
# `python3.X/lib-dynload` beside it. `bin/lib/` is such a directory, so no
# wrapper script and no PYTHONHOME are needed (btxd ignores PYTHONHOME anyway:
# it initialises Python in isolated mode). Move libpython out of a directory
# called `lib`, or the standard library away from it, and `-swapnet=1` stops at
# init while `-swapnet=0` keeps working, which is the kind of break nobody
# notices until the feature is switched on.
#
# Source this file, or run it:
#
#   engine-runtime-linux.sh stage <dir with btxd + btx-cli> <dest bin dir>
#   engine-runtime-linux.sh check <bin dir> [report file]
#   engine-runtime-linux.sh clean-start <bin dir> <plain|swapnet0|swapnet1|masked>

# The oldest machines the engine must run on: Ubuntu 22.04. glibc 2.35, and the
# libstdc++ of GCC 12 that 22.04 ships (GLIBCXX_3.4.30, CXXABI_1.3.13). glibc
# and the C++ runtime are the only libraries taken from the user's machine.
ENGINE_GLIBC_MAX="${ENGINE_GLIBC_MAX:-2.35}"
ENGINE_GLIBCXX_MAX="${ENGINE_GLIBCXX_MAX:-3.4.30}"
ENGINE_CXXABI_MAX="${ENGINE_CXXABI_MAX:-1.3.13}"
# The image the clean start runs in. It supplies a glibc and nothing else: no
# Python, no libevent, no sqlite, no CUDA. Nothing from it reaches the package.
ENGINE_CLEAN_IMAGE="${ENGINE_CLEAN_IMAGE:-ubuntu:22.04}"
# Written by engine-python-bundle.py into bin/lib/python3.X/. Its presence is
# how staging tells a runtime that was already bundled (an engine artifact)
# from an interpreter prefix that still has to be bundled (a hand build).
ENGINE_PY_MANIFEST="EASYNODE-RUNTIME.json"

_engine_lib_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

_engine_err() { printf 'error: %s\n' "$*" >&2; }

# ── Reading ELF files ───────────────────────────────────────────────────────

engine_is_elf() {
  local magic=""
  [[ -f "$1" && ! -L "$1" ]] || return 1
  LC_ALL=C IFS= read -r -N 4 magic 2>/dev/null < "$1" || true
  [[ "$magic" == $'\x7fELF' ]]
}

# Every ELF file under a directory, sorted.
engine_elfs() {
  local f
  while IFS= read -r -d '' f; do
    if engine_is_elf "$f"; then printf '%s\n' "$f"; fi
  done < <(find "$1" -type f -print0 | sort -z)
}

engine_needed() {
  LC_ALL=C readelf -d "$1" 2>/dev/null \
    | awk '/\(NEEDED\)/ { gsub(/[][]/, "", $NF); print $NF }'
}

engine_runpath() {
  LC_ALL=C readelf -d "$1" 2>/dev/null \
    | awk '/\((RPATH|RUNPATH)\)/ { gsub(/[][]/, "", $NF); print $NF }' | head -1
}

# Every DT_RPATH and DT_RUNPATH value, one per line. The check reads both: an
# old-style RPATH is searched before LD_LIBRARY_PATH and would be the worse one
# to miss.
engine_runpath_entries() {
  LC_ALL=C readelf -d "$1" 2>/dev/null \
    | awk '/\((RPATH|RUNPATH)\)/ { gsub(/[][]/, "", $NF); print $NF }'
}

# The libpython btxd links, or nothing (every engine before 0.34.16).
engine_python_soname() {
  engine_needed "$1" | { grep -E '^libpython3\.[0-9]+t?\.so\.1\.0$' || true; } | head -1
}

# "System" = glibc and the GCC runtime, which every glibc Linux has. Everything
# else travels with the package: libevent, sqlite, zmq and what they pull in,
# cuBLASLt, libpython, and what the Python extension modules need.
#
# libatomic used to be on this list. It is part of GCC but NOT installed on a
# minimal system, so a binary that needed it would have passed staging and
# failed on a user's machine. Nothing links it today; if something ever does it
# is now vendored like any other library.
engine_is_system_lib() {
  case "$1" in
    libc.so.*|libm.so.*|libdl.so.*|librt.so.*|libpthread.so.*|libresolv.so.*|\
    libutil.so.*|libmvec.so.*|libgcc_s.so.*|libstdc++.so.*|ld-linux*.so.*|\
    linux-vdso.so.*) return 0 ;;
    *) return 1 ;;
  esac
}

# Where the loader finds SONAME for ELF on this machine, as an absolute path.
engine_resolve() {
  ldd "$1" 2>/dev/null | awk -v n="$2" '
    $1 == n && $2 == "=>" && $3 ~ /^\// { print $3; found = 1; exit }
    END { exit !found }'
}

# ── Vendoring ───────────────────────────────────────────────────────────────

# engine_vendor_closure LIBDIR ELF...
# Copy into LIBDIR every non-system library the given ELFs need, and what those
# need in turn, each under the exact name the loader asks for. A library that
# is already in LIBDIR is kept as it is: when staging from an engine artifact
# that is the copy the engine build tested.
engine_vendor_closure() {
  local libdir=$1; shift
  local -a queue=("$@")
  local elf dep path
  while [[ ${#queue[@]} -gt 0 ]]; do
    elf=${queue[0]}; queue=("${queue[@]:1}")
    while IFS= read -r dep; do
      [[ -n "$dep" ]] || continue
      if engine_is_system_lib "$dep"; then continue; fi
      [[ -f "$libdir/$dep" ]] && continue
      if ! path="$(engine_resolve "$elf" "$dep")"; then
        _engine_err "$elf needs $dep and the loader cannot find it on this machine."
        return 1
      fi
      cp -L "$path" "$libdir/$dep" || return 1
      chmod 644 "$libdir/$dep"
      queue+=("$libdir/$dep")
    done < <(engine_needed "$elf")
  done
}

# Set one file's run path, touching the file only when it differs. '' removes it.
engine_set_runpath() {
  local f=$1 want=$2 cur
  cur="$(engine_runpath "$f")"
  [[ "$cur" == "$want" ]] && return 0
  # Said out loud: this is the one place the package's bytes are changed.
  echo "    run path [${cur:-none}] -> [${want:-none}]  ${f##*/}"
  chmod u+w "$f"
  if [[ -z "$want" ]]; then
    patchelf --remove-rpath "$f"
  else
    patchelf --set-rpath "$want" "$f"
  fi
}

# engine_set_runpaths BINDIR
# RUNPATH (the patchelf default) is NOT inherited by a dependency's own
# lookups, so every file that loads something from bin/lib/ needs its own
# entry: the executables, the libraries beside each other, and the Python
# extension modules two directories further down. An ELF that needs nothing
# from the package gets NO run path, so no build-machine path ships.
engine_set_runpaths() {
  local bindir=$1 libdir="$1/lib" f dep rel nonsys
  for f in "$bindir/btxd" "$bindir/btx-cli"; do
    engine_set_runpath "$f" '$ORIGIN/lib' || return 1
  done
  [[ -d "$libdir" ]] || return 0
  while IFS= read -r f; do
    if [[ "$(dirname "$f")" == "$libdir" ]]; then
      engine_set_runpath "$f" '$ORIGIN' || return 1
      continue
    fi
    nonsys=0
    while IFS= read -r dep; do
      if ! engine_is_system_lib "$dep"; then nonsys=1; fi
    done < <(engine_needed "$f")
    if [[ $nonsys -eq 1 ]]; then
      rel="$(realpath --relative-to="$(dirname "$f")" "$libdir")"
      engine_set_runpath "$f" "\$ORIGIN/$rel" || return 1
    else
      engine_set_runpath "$f" '' || return 1
    fi
  done < <(engine_elfs "$libdir")
}

# ── The Python runtime ──────────────────────────────────────────────────────

# engine_stage_python SRC_BTXD SONAME LIBDIR
# Make LIBDIR hold the runtime for the libpython SRC_BTXD links. Already there
# (copied from an engine artifact): nothing to do. Otherwise bundle it from the
# interpreter that owns the library the loader resolves, which is what the
# engine build does and what a hand build on the release box would do.
engine_stage_python() {
  local btxd=$1 soname=$2 libdir=$3 ver resolved prefix python swapsrc
  ver="${soname#libpython}"; ver="${ver%%.so*}"; ver="${ver%t}"
  if [[ -f "$libdir/$soname" && -f "$libdir/python$ver/$ENGINE_PY_MANIFEST" ]]; then
    echo "==> Python $ver runtime: taken from the engine artifact, as its build tested it"
    return 0
  fi
  # A 0.34.16 build output cannot find its own libpython. Its run path is
  # `$ORIGIN/../lib`, the layout of upstream's archive, and next to build/bin
  # that is a directory of static libraries (read off the binary in run
  # 45512058871; up to 0.34.15 the run path named the CUDA toolkit directory
  # instead). Run 45509202706 died on exactly that: "libpython3.14.so.1.0:
  # cannot open shared object file" from build/bin/btxd, on the machine that
  # had just linked it. So the loader can only answer here when the library is
  # on its search path for another reason. When it is not, ENGINE_PYTHON says
  # which interpreter the build was given, and that interpreter's lib/ is
  # where the library it links lives.
  if resolved="$(engine_resolve "$btxd" "$soname")"; then
    resolved="$(realpath "$resolved")"
  elif [[ -n "${ENGINE_PYTHON:-}" && -f "$(dirname "$(dirname "$(readlink -f "$ENGINE_PYTHON")")")/lib/$soname" ]]; then
    resolved="$(dirname "$(dirname "$(readlink -f "$ENGINE_PYTHON")")")/lib/$soname"
  else
    _engine_err "btxd links $soname and the loader cannot find it on this machine."
    _engine_err "Stage from an engine artifact (it carries lib/python$ver), or set"
    _engine_err "ENGINE_PYTHON to the python$ver the build was configured with."
    return 1
  fi
  prefix="$(dirname "$(dirname "$resolved")")"
  python="${ENGINE_PYTHON:-$prefix/bin/python$ver}"
  if [[ ! -x "$python" ]]; then
    _engine_err "btxd links $resolved, but there is no interpreter at $python"
    _engine_err "to take its standard library from. Set ENGINE_PYTHON to the python$ver"
    _engine_err "the build was configured with."
    return 1
  fi
  swapsrc="${ENGINE_SWAPNET_SRC:-$(dirname "$btxd")/../../contrib/swapnet/swapnet}"
  if [[ ! -d "$swapsrc" ]]; then
    _engine_err "no SwapNet sources at $swapsrc. They decide which modules the runtime"
    _engine_err "must be able to import. Set ENGINE_SWAPNET_SRC to contrib/swapnet/swapnet"
    _engine_err "of the btx checkout this btxd was built from."
    return 1
  fi
  echo "==> Python $ver runtime: bundling from $python"
  # LD_LIBRARY_PATH only so the interpreter binary itself starts: it links the
  # same libpython, and not every build of it carries a run path to it.
  LD_LIBRARY_PATH="$(dirname "$resolved")${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
    "$python" -I "$_engine_lib_dir/engine-python-bundle.py" \
    "$(realpath "$swapsrc")" "$libdir" "$soname" "$resolved"
}

# ── A finished package ──────────────────────────────────────────────────────

# An engine artifact from btxd-linux.yml as it is since 0.34.16 work began: its
# SHA256SUMS.txt lists every file, lib/ included. (The older artifact lists only
# the two binaries and carries only cuBLASLt; that one still has to be completed
# on the staging machine.)
engine_is_finished_package() {
  [[ -f "$1/SHA256SUMS.txt" ]] && grep -qE '^[0-9a-f]{64}  lib/' "$1/SHA256SUMS.txt"
}

# engine_verify_sums BINDIR SUMS
# The staged tree against the sums the engine build wrote AFTER it had checked
# and started the package. Both directions: no file missing, none extra, none
# changed. Anything else means what is about to ship is not what was tested.
engine_verify_sums() {
  local bindir=$1 sums=$2 want have
  want="$(grep -E '^[0-9a-f]{64}  (btxd|btx-cli|lib/.+)$' "$sums" | LC_ALL=C sort -k2)"
  if [[ -z "$want" ]]; then
    _engine_err "$sums lists no package file"
    return 1
  fi
  have="$(cd "$bindir" && find . -type f -printf '%P\0' | LC_ALL=C sort -z | xargs -0 sha256sum | LC_ALL=C sort -k2)"
  if [[ "$want" != "$have" ]]; then
    _engine_err "the staged tree is not the package the engine build tested."
    _engine_err "Lines with < are in the artifact's SHA256SUMS.txt only, lines with > in the staged tree only:"
    diff <(printf '%s\n' "$want") <(printf '%s\n' "$have") | grep -E '^[<>]' | head -20 >&2
    return 1
  fi
  echo "==> $(printf '%s\n' "$have" | wc -l | tr -d ' ') staged file(s) are byte for byte the ones in the artifact's SHA256SUMS.txt"
}

# ── Staging ─────────────────────────────────────────────────────────────────

# engine_stage_tree SRC BINDIR
# SRC holds btxd and btx-cli: a btx build's bin/, or a downloaded engine
# artifact (which also has lib/, the closure its build already tested).
# BINDIR is replaced.
engine_stage_tree() {
  local src=$1 bindir=$2 libdir="$2/lib" f soname
  local -a roots
  for f in patchelf readelf ldd realpath; do
    command -v "$f" >/dev/null || { _engine_err "$f is required (apt install patchelf binutils)."; return 1; }
  done
  for f in btxd btx-cli; do
    [[ -f "$src/$f" ]] || { _engine_err "$src/$f not found"; return 1; }
  done
  rm -rf "$bindir"
  mkdir -p "$libdir" || return 1
  cp "$src/btxd" "$src/btx-cli" "$bindir/" || return 1
  chmod 755 "$bindir/btxd" "$bindir/btx-cli"
  # An engine artifact: take its lib/ whole, BEFORE asking the loader for
  # anything. Otherwise a library this machine happens to have (the installer
  # runner has most of them) could be picked up in place of the copy the engine
  # build started in a clean container.
  if [[ -d "$src/lib" ]]; then
    cp -R "$src/lib/." "$libdir/" || return 1
    echo "==> took $(find "$libdir" -type f | wc -l | tr -d ' ') file(s) from $src/lib"
  fi
  # A finished package is COPIED and COMPARED, nothing else. No library is
  # looked up on this machine and no run path is touched: if the artifact were
  # short of a library or had a wrong run path, completing it here would ship
  # bytes the engine build never started, and would do it silently. The check
  # the caller runs next (engine_check_tree) only reads.
  if engine_is_finished_package "$src"; then
    engine_verify_sums "$bindir" "$src/SHA256SUMS.txt" || return 1
    return 0
  fi
  soname="$(engine_python_soname "$bindir/btxd")"
  if [[ -n "$soname" ]]; then
    engine_stage_python "$src/btxd" "$soname" "$libdir" || return 1
  fi
  # The two binaries are resolved at their SOURCE path, where their run path
  # still means something; the copies have not been given theirs yet.
  roots=("$src/btxd" "$src/btx-cli")
  while IFS= read -r f; do roots+=("$f"); done < <(engine_elfs "$libdir")
  engine_vendor_closure "$libdir" "${roots[@]}" || return 1
  engine_set_runpaths "$bindir" || return 1
  # The manifest names the libpython that SHIPS. The bundler saw it before its
  # run path was set, and recorded that as libpython_source_sha256.
  if [[ -n "$soname" ]]; then
    for f in "$libdir"/python3.*/"$ENGINE_PY_MANIFEST"; do
      [[ -f "$f" ]] || continue
      sed -i "s/\"libpython_sha256\": \"[^\"]*\"/\"libpython_sha256\": \"$(sha256sum "$libdir/$soname" | cut -d' ' -f1)\"/" "$f" || return 1
      grep -q '"libpython_sha256": "[0-9a-f]\{64\}"' "$f" || { _engine_err "could not record the shipped libpython's sha256 in $f"; return 1; }
    done
  fi
  rmdir "$libdir" 2>/dev/null || true
}

# ── Checks ──────────────────────────────────────────────────────────────────

# The versions an ELF REQUIRES from other libraries (.gnu.version_r), one name
# per line. Not `objdump -T | grep GLIBC_`, which also matches the versions a
# library defines.
engine_version_needs() {
  LC_ALL=C readelf --version-info --wide "$1" 2>/dev/null | awk '
    /^Version needs section/ { n = 1; next }
    /^Version (symbols|definition) section/ { n = 0 }
    n { for (i = 1; i < NF; i++) if ($i == "Name:") print $(i + 1) }'
}

# Highest PREFIX_x.y[.z] among the names on stdin; empty when there is none.
engine_max_version() {
  { grep -E "^$1_[0-9]+(\.[0-9]+)*$" || true; } | sed "s/^$1_//" | sort -uV | tail -1
}

engine_version_le() {
  [[ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | head -1)" == "$1" ]]
}

# engine_check_tree BINDIR [REPORT]
# Every ELF in the package, not only the two binaries:
#   1. its run path has nothing but $ORIGIN entries;
#   2. with no LD_LIBRARY_PATH the loader resolves all it needs, and takes
#      nothing but glibc and the C++ runtime from outside the package;
#   3. it needs no glibc newer than ENGINE_GLIBC_MAX and no libstdc++ newer
#      than ENGINE_GLIBCXX_MAX / ENGINE_CXXABI_MAX.
# Prints one line per ELF with the highest version of each it needs, and writes
# the same lines to REPORT. Returns non-zero when any file fails any check.
engine_check_tree() {
  local bindir libdir report=${2:-} f rel rp part out dep path needs g x a line
  local rc=0 count=0 gmax=0 xmax=0 amax=0
  local -a files parts
  bindir="$(realpath "$1")"; libdir="$bindir/lib"
  files=("$bindir/btxd" "$bindir/btx-cli")
  if [[ -d "$libdir" ]]; then
    while IFS= read -r f; do files+=("$f"); done < <(engine_elfs "$libdir")
  fi
  line="$(printf '%-7s %-9s %-8s %s' GLIBC GLIBCXX CXXABI file)"
  echo "$line"
  [[ -n "$report" ]] && echo "$line" > "$report"
  for f in "${files[@]}"; do
    [[ -f "$f" ]] || { _engine_err "$f is missing from the package"; rc=1; continue; }
    rel="${f#"$bindir"/}"
    count=$((count + 1))

    # readelf must ANSWER. Its errors are discarded everywhere else in this
    # file, and an ELF it cannot read would otherwise look like one that needs
    # nothing: no libraries, no symbol versions, a clean row in the table.
    # Its exit code is not enough: on a truncated file readelf 2.38 prints
    # errors and still exits 0 (tried). So: no "Error" on stderr, and a dynamic
    # section, which every file in this package has.
    if LC_ALL=C readelf -d --version-info "$f" 2>&1 >/dev/null | grep -q 'Error'        || ! LC_ALL=C readelf -d "$f" 2>/dev/null | grep -q '^Dynamic section'; then
      _engine_err "readelf cannot read $rel, so nothing about it is known"
      rc=1
      continue
    fi

    while IFS= read -r rp; do
      IFS=: read -ra parts <<< "$rp"
      for part in "${parts[@]}"; do
        case "$part" in
          '$ORIGIN'|'$ORIGIN/'*) ;;
          *) _engine_err "$rel has the run path entry '$part': a path of the build machine, not of the package"; rc=1 ;;
        esac
      done
    done < <(engine_runpath_entries "$f")

    out="$(env -u LD_LIBRARY_PATH -u LD_PRELOAD ldd "$f" 2>&1 || true)"
    if grep -q "not found" <<< "$out"; then
      _engine_err "$rel has unresolved libraries:"
      grep "not found" <<< "$out" >&2
      rc=1
    fi
    while read -r dep path; do
      [[ -n "$dep" ]] || continue
      if engine_is_system_lib "$dep"; then continue; fi
      path="$(realpath -m "$path")"
      case "$path" in
        "$libdir"/*) ;;
        *) _engine_err "$rel takes $dep from $path, outside the package"; rc=1 ;;
      esac
    done < <(awk '{ for (i = 1; i < NF; i++) if ($i == "=>" && $(i + 1) ~ /^\//) print $1, $(i + 1) }' <<< "$out")

    needs="$(engine_version_needs "$f")"
    g="$(engine_max_version GLIBC <<< "$needs")"
    x="$(engine_max_version GLIBCXX <<< "$needs")"
    a="$(engine_max_version CXXABI <<< "$needs")"
    if [[ -n "$g" ]]; then
      engine_version_le "$g" "$ENGINE_GLIBC_MAX" || { _engine_err "$rel needs GLIBC_$g, newer than $ENGINE_GLIBC_MAX"; rc=1; }
      engine_version_le "$g" "$gmax" || gmax=$g
    fi
    if [[ -n "$x" ]]; then
      engine_version_le "$x" "$ENGINE_GLIBCXX_MAX" || { _engine_err "$rel needs GLIBCXX_$x, newer than $ENGINE_GLIBCXX_MAX"; rc=1; }
      engine_version_le "$x" "$xmax" || xmax=$x
    fi
    if [[ -n "$a" ]]; then
      engine_version_le "$a" "$ENGINE_CXXABI_MAX" || { _engine_err "$rel needs CXXABI_$a, newer than $ENGINE_CXXABI_MAX"; rc=1; }
      engine_version_le "$a" "$amax" || amax=$a
    fi
    line="$(printf '%-7s %-9s %-8s %s' "${g:--}" "${x:--}" "${a:--}" "$rel")"
    echo "$line"
    [[ -n "$report" ]] && echo "$line" >> "$report"
  done
  # btxd and btx-cli are always there. Fewer than two means the walk looked at
  # nothing and would otherwise report a cheerful OK.
  if [[ $count -lt 2 ]]; then
    _engine_err "the check looked at only $count ELF file(s) under $bindir"
    rc=1
  fi
  line="highest needed by $count ELF file(s): GLIBC_$gmax (limit $ENGINE_GLIBC_MAX), GLIBCXX_$xmax (limit $ENGINE_GLIBCXX_MAX), CXXABI_$amax (limit $ENGINE_CXXABI_MAX)"
  echo "$line"
  [[ -n "$report" ]] && echo "$line" >> "$report"
  return $rc
}

# ── The clean start ─────────────────────────────────────────────────────────

# What runs INSIDE the clean image, under `env -i`. POSIX sh: the image has no
# reason to carry anything else. Each failing step prints why and exits with
# its own code.
IFS= read -r -d '' ENGINE_CLEAN_START_SCRIPT <<'CLEAN_START' || true
set -eu
mode=$1
e=/opt/engine/bin
d=/tmp/engine-clean-start
log="$d/regtest/debug.log"
mkdir -p "$d" "$HOME"
say() { printf '%s\n' "$*"; }
cli() { "$e/btx-cli" -regtest -datadir="$d" "$@"; }
swaplines() { grep 'SwapNet' "$log" 2>/dev/null | sed 's/^/  debug.log: /' || true; }
logtail() { tail -n 40 "$log" 2>/dev/null | sed 's/^/  debug.log: /' || true; }

# The image must really be clean, or passing here proves nothing.
if command -v python3 >/dev/null 2>&1 || command -v python >/dev/null 2>&1; then
  say "NOT A CLEAN IMAGE: a Python interpreter is on PATH"; exit 40
fi
if ldconfig -p 2>/dev/null | grep -q 'libpython'; then
  say "NOT A CLEAN IMAGE: the loader cache already has a libpython"; exit 40
fi
say "image: $(. /etc/os-release && printf '%s' "$PRETTY_NAME"), $(getconf GNU_LIBC_VERSION), no Python on the machine"

v=$("$e/btxd" --version 2>&1) || { say "btxd --version failed:"; say "$v"; exit 41; }
say "$v" | head -n 1
v=$("$e/btx-cli" --version 2>&1) || { say "btx-cli --version failed:"; say "$v"; exit 41; }
say "$v" | head -n 1

case "$mode" in
  plain) flag="" ;;
  swapnet0) flag="-swapnet=0" ;;
  swapnet1|masked) flag="-swapnet=1" ;;
  *) say "unknown mode: $mode"; exit 2 ;;
esac
say "starting: btxd -regtest $flag"
started=1
# $flag is empty or one word.
# shellcheck disable=SC2086
"$e/btxd" -regtest -datadir="$d" -fallbackfee=0.0001 -daemonwait $flag > "$d/start.out" 2>&1 || started=0

if [ "$mode" = masked ]; then
  # The negative control: the same start with the standard library hidden.
  if [ "$started" = 1 ]; then
    cli stop > /dev/null 2>&1 || true
    say "NEGATIVE CONTROL FAILED: btxd -swapnet=1 started although the Python standard"
    say "library was hidden. This test cannot tell a working runtime from a missing one."
    swaplines
    exit 42
  fi
  sed 's/^/  start: /' "$d/start.out"
  swaplines
  # It must refuse FOR THIS REASON. A start that fails for any other one (a
  # port, a library, the datadir) would pass a looser control and prove nothing
  # about the runtime.
  if ! grep -q 'SwapNet: startup failed' "$log" 2>/dev/null; then
    say "NEGATIVE CONTROL FAILED: btxd refused to start, but never logged 'SwapNet: startup failed'."
    logtail
    exit 43
  fi
  say "CLEAN START OK (masked): without the standard library btxd -swapnet=1 refuses to start, and says why"
  exit 0
fi

if [ "$started" != 1 ]; then
  say "START FAILED ($mode): btxd -regtest $flag did not come up in the clean image"
  sed 's/^/  start: /' "$d/start.out"
  logtail
  exit 44
fi
trap 'cli stop > /dev/null 2>&1 || true' EXIT

info=$(cli getblockchaininfo) || { say "getblockchaininfo failed"; logtail; exit 45; }
case "$info" in
  *'"chain": "regtest"'*) ;;
  *) say "getblockchaininfo did not report regtest:"; say "$info"; exit 45 ;;
esac
cli createwallet smoke > /dev/null || { say "createwallet failed"; logtail; exit 46; }
addr=$(cli -rpcwallet=smoke getnewaddress) || { say "getnewaddress failed"; logtail; exit 46; }
[ -n "$addr" ] || { say "getnewaddress returned nothing"; exit 46; }
cli -rpcwallet=smoke generatetoaddress 1 "$addr" > /dev/null || { say "generatetoaddress failed"; logtail; exit 47; }
count=$(cli getblockcount) || { say "getblockcount failed"; exit 47; }
[ "$count" = 1 ] || { say "unexpected block count: $count"; exit 47; }
say "regtest: wallet created, 1 block mined"

case "$mode" in
  swapnet1)
    if ! grep -q 'SwapNet: embedded runtime started' "$log"; then
      say "btxd -swapnet=1 is running but never logged 'SwapNet: embedded runtime started':"
      swaplines
      exit 48
    fi
    # The answer itself, not the absence of an error: exit 0, and the two
    # fields that only the in-process Python worker of this protocol sets.
    out=$(cli getswapnetworkinfo 2>&1) && rc=0 || rc=$?
    say "getswapnetworkinfo (exit $rc):"
    printf '%s\n' "$out" | head -n 12 | sed 's/^/  /'
    [ "$rc" = 0 ] || { say "getswapnetworkinfo failed with -swapnet=1"; exit 49; }
    case "$out" in
      *'"in_process": true'*) ;;
      *) say "getswapnetworkinfo does not report \"in_process\": true"; exit 49 ;;
    esac
    case "$out" in
      *'"protocol": 2,'*) ;;
      *) say "getswapnetworkinfo does not report \"protocol\": 2"; exit 49 ;;
    esac
    ;;
  swapnet0)
    if grep -q 'SwapNet: embedded runtime started' "$log"; then
      say "the SwapNet runtime started although -swapnet=0 was given"; swaplines; exit 50
    fi
    if out=$(cli getswapnetworkinfo 2>&1); then
      say "getswapnetworkinfo answered although -swapnet=0 was given:"; say "$out"; exit 51
    fi
    say "getswapnetworkinfo with -swapnet=0: $(printf '%s' "$out" | tr '\n' ' ' | cut -c1-200)"
    ;;
esac
swaplines

cli stop > /dev/null 2>&1 || true
trap - EXIT
n=0
while [ "$n" -lt 60 ] && cli getblockcount > /dev/null 2>&1; do n=$((n + 1)); sleep 1; done
say "CLEAN START OK ($mode)"
CLEAN_START

# engine_clean_start BINDIR MODE
# Start the package in ENGINE_CLEAN_IMAGE: mounted read-only, an empty
# environment (`env -i`), no network, an unprivileged user.
#
#   plain     an engine without Python (before 0.34.16): boot, wallet, one block
#   swapnet0  the same with -swapnet=0. libpython must load (btxd links it)
#             although Python is never started, and the SwapNet RPCs must refuse.
#   swapnet1  the same with -swapnet=1. btxd must log that the embedded runtime
#             started, which takes the standard library, sqlite3 and
#             cryptography, all from the package, and getswapnetworkinfo must
#             exit 0 and report "in_process": true and "protocol": 2.
#   masked    swapnet1 with bin/lib/python3.X hidden behind an empty directory.
#             btxd must REFUSE to start and log "SwapNet: startup failed". This
#             is the proof that swapnet1 passing
#             means the bundled runtime was used: if a start without it also
#             succeeded, the test would be measuring nothing.
engine_clean_start() {
  local bindir mode=$2 pydir empty
  local -a mask=()
  bindir="$(realpath "$1")"
  command -v docker >/dev/null || { _engine_err "docker is required for the clean start"; return 1; }
  if [[ "$mode" == masked ]]; then
    pydir="$(find "$bindir/lib" -maxdepth 1 -type d -name 'python3.*' | head -1)"
    [[ -n "$pydir" ]] || { _engine_err "no bin/lib/python3.X to hide in $bindir"; return 1; }
    empty="$(mktemp -d)"; chmod 755 "$empty"
    mask=(-v "$empty:/opt/engine/bin/lib/$(basename "$pydir"):ro")
  fi
  timeout 600 docker run --rm --network none --user "$(id -u):$(id -g)" \
    -v "$bindir:/opt/engine/bin:ro" "${mask[@]}" \
    "$ENGINE_CLEAN_IMAGE" \
    /usr/bin/env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin HOME=/tmp/home \
    /bin/sh -c "$ENGINE_CLEAN_START_SCRIPT" engine-clean-start "$mode"
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  set -euo pipefail
  cmd="${1:-}"
  [[ $# -gt 0 ]] && shift
  case "$cmd" in
    stage) engine_stage_tree "$@" ;;
    check) engine_check_tree "$@" ;;
    clean-start) engine_clean_start "$@" ;;
    *)
      echo "usage: $(basename "$0") stage <src> <bin dir> | check <bin dir> [report] | clean-start <bin dir> <mode>" >&2
      exit 2
      ;;
  esac
fi
