#!/usr/bin/env bash
# Shared download/verify/extract for upstream btxchain/btx release archives.
# Sourced by stage-node-pkg.sh (macOS) and stage-node-pkg-linux.sh.
#
# WHY THIS EXISTS. Upstream changed its archive format between 0.34 and 0.35:
#
#                 0.34.x                                  0.35.0-rc1
#   name          btx-<ver>-<triple>.tar.gz               btx-0XY-<platform>.tar.xz
#   checksums     SHA256SUMS (missing on .13/.14/.15)     SHA384SUMS
#   top dir       btx-<ver>/                              none (./bin/ at the root)
#   btxd          bin/btxd wrapper -> libexec/btxd.real   bin/btxd (no libexec/)
#   metal         libexec/metal/                          bin/metal/
#
# The staging scripts were written for the 0.34 column: `tar -xzf`, a
# `btx-$VERSION/` top dir and libexec/*.real. Against a 0.35 archive they would
# either fail with a confusing tar error or, worse, stage half a package. These
# helpers take both formats for the mechanical part (hash, unpack, find the
# root) and then REFUSE a layout the script has not been adapted to, loudly,
# instead of guessing at it. Adapting the copy/rpath/codesign steps to the 0.35
# layout is a deliberate change to make once 0.35 final's layout is known.

# release_archive_verify FILE HEX
# HEX is the archive's pinned digest: 64 hex = SHA-256 (GitHub's asset digest,
# "sha256:" prefix allowed), 96 hex = SHA-384 (0.35's SHA384SUMS).
release_archive_verify() {
  local file="$1" want="${2#sha256:}" bits got
  want="$(printf '%s' "$want" | tr 'A-F' 'a-f')"
  case "${#want}" in
    64) bits=256 ;;
    96) bits=384 ;;
    *) echo "error: pinned digest for $(basename "$file") is ${#want} hex chars;" \
            "expected 64 (sha256) or 96 (sha384)" >&2; return 1 ;;
  esac
  [[ "$want" =~ ^[0-9a-f]+$ ]] || { echo "error: pinned digest is not hex: $want" >&2; return 1; }
  if command -v shasum >/dev/null 2>&1; then
    got="$(shasum -a "$bits" "$file" | awk '{print $1}')"
  elif command -v "sha${bits}sum" >/dev/null 2>&1; then
    got="$("sha${bits}sum" "$file" | awk '{print $1}')"
  else
    echo "error: neither shasum nor sha${bits}sum is installed" >&2; return 1
  fi
  if [[ "$got" != "$want" ]]; then
    echo "error: $(basename "$file") sha$bits is $got, pinned $want" >&2
    echo "       Upstream has regenerated assets in place before (v0.32.11). Re-check" >&2
    echo "       the GitHub digest and SHA*SUMS before changing the pin." >&2
    return 1
  fi
  echo "==> sha$bits OK: $(basename "$file")"
}

# release_archive_extract FILE DIR -> prints the package root (the dir holding bin/btxd)
# `tar -xf` detects gzip and xz on both GNU tar and macOS bsdtar.
release_archive_extract() {
  local file="$1" dir="$2" roots
  mkdir -p "$dir"
  tar -xf "$file" -C "$dir" || { echo "error: could not unpack $(basename "$file")" >&2; return 1; }
  if [[ -x "$dir/bin/btxd" ]]; then
    printf '%s\n' "$dir"; return 0
  fi
  roots="$(find "$dir" -mindepth 2 -maxdepth 3 -path '*/bin/btxd' -type f 2>/dev/null)"
  if [[ "$(printf '%s\n' "$roots" | grep -c .)" != 1 ]]; then
    echo "error: expected exactly one bin/btxd in $(basename "$file"), found:" >&2
    printf '  %s\n' ${roots:-"(none)"} >&2
    return 1
  fi
  dirname "$(dirname "$roots")"
}

# release_layout_is_034 ROOT : the staging below copies libexec/*.real.
release_layout_is_034() {
  [[ -x "$1/bin/btxd" && -f "$1/libexec/btxd.real" ]]
}

# require_034_layout ROOT : fail closed on any other layout.
require_034_layout() {
  local root="$1"
  release_layout_is_034 "$root" && return 0
  cat >&2 <<EOF
error: $root is not the 0.34 package layout this script stages
       (bin/btxd wrapper + libexec/btxd.real). It looks like the 0.35 layout
       (bin/btxd itself, bin/metal/, no libexec/). Adapt this staging script's
       copy, rpath and codesign steps to that layout in a reviewed change
       before shipping it. Do not work around this check.
EOF
  return 1
}

# btxd_reports_exact_version BTXD VERSION
# Passes only when the first --version line names exactly v<VERSION>: "v0.35.0"
# must not accept "v0.35.0rc1" (a substring grep would), and "v0.35.0rc1" must
# be pinned on purpose to stage an rc.
btxd_reports_exact_version() {
  local btxd="$1" version="$2" line
  line="$("$btxd" --version 2>/dev/null | head -1)" || true
  btxd_version_line_matches "$line" "$version"
}

# btxd_version_line_matches LINE VERSION : the pure check, for tests.
btxd_version_line_matches() {
  local line="$1" version="$2" esc
  esc="$(printf '%s' "$version" | sed 's/[.]/\\./g')"
  printf '%s\n' "$line" | grep -qE "(^|[^0-9A-Za-z.])v${esc}([^0-9A-Za-z.]|$)"
}
