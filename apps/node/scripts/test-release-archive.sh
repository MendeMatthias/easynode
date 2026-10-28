#!/usr/bin/env bash
# Offline test for lib/release-archive.sh. Builds tiny fake archives in both the
# 0.34 and the 0.35 layout and checks every helper, including that each check
# can actually fail. No network, no real btxd.
#   bash apps/node/scripts/test-release-archive.sh
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/lib/release-archive.sh"

T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
fails=0
ok()   { echo "ok   $1"; }
bad()  { echo "FAIL $1"; fails=$((fails + 1)); }
want_ok()   { local name="$1"; shift; if "$@" >/dev/null 2>&1; then ok "$name"; else bad "$name"; fi; }
want_fail() { local name="$1"; shift; if "$@" >/dev/null 2>&1; then bad "$name (should fail)"; else ok "$name"; fi; }

fake_btxd() { printf '#!/bin/sh\necho "BTX Core daemon version %s"\n' "$2" > "$1"; chmod +x "$1"; }
sha() { shasum -a "$1" "$2" 2>/dev/null | awk '{print $1}' || "sha$1sum" "$2" | awk '{print $1}'; }

# 0.34 layout: btx-0.34.15/{bin/btxd, libexec/btxd.real}, .tar.gz
mkdir -p "$T/a/btx-0.34.15/bin" "$T/a/btx-0.34.15/libexec"
fake_btxd "$T/a/btx-0.34.15/bin/btxd" v0.34.15
echo real > "$T/a/btx-0.34.15/libexec/btxd.real"
tar -czf "$T/old.tar.gz" -C "$T/a" btx-0.34.15
# 0.35 layout: ./bin/btxd, ./bin/metal/, .tar.xz
mkdir -p "$T/b/bin/metal"
fake_btxd "$T/b/bin/btxd" v0.35.0rc1
echo k > "$T/b/bin/metal/k.metallib"
( cd "$T/b" && tar -cJf "$T/new.tar.xz" . )

# verify: sha256, sha256: prefix, sha384, wrong hash, wrong length
want_ok   "verify sha256"             release_archive_verify "$T/old.tar.gz" "$(sha 256 "$T/old.tar.gz")"
want_ok   "verify sha256: prefix"     release_archive_verify "$T/old.tar.gz" "sha256:$(sha 256 "$T/old.tar.gz")"
want_ok   "verify sha384"             release_archive_verify "$T/new.tar.xz" "$(sha 384 "$T/new.tar.xz")"
want_fail "verify wrong sha256"       release_archive_verify "$T/old.tar.gz" "$(sha 256 "$T/new.tar.xz")"
want_fail "verify wrong sha384"       release_archive_verify "$T/new.tar.xz" "$(sha 384 "$T/old.tar.gz")"
want_fail "verify bad digest length"  release_archive_verify "$T/old.tar.gz" "abc123"

# extract: both compressions, root found for both layouts
r_old="$(release_archive_extract "$T/old.tar.gz" "$T/x1" 2>/dev/null)"
[[ "$r_old" == "$T/x1/btx-0.34.15" ]] && ok "extract .tar.gz finds btx-<ver>/ root" || bad "extract .tar.gz root: $r_old"
r_new="$(release_archive_extract "$T/new.tar.xz" "$T/x2" 2>/dev/null)"
[[ "$r_new" == "$T/x2" ]] && ok "extract .tar.xz finds ./ root" || bad "extract .tar.xz root: $r_new"

# layout gate
want_ok   "0.34 layout accepted"      require_034_layout "$r_old"
want_fail "0.35 layout refused"       require_034_layout "$r_new"

# exact version
want_ok   "v0.34.15 == 0.34.15"       btxd_version_line_matches "BTX Core daemon version v0.34.15" 0.34.15
want_fail "v0.35.0rc1 != 0.35.0"      btxd_version_line_matches "BTX Core daemon version v0.35.0rc1" 0.35.0
want_ok   "v0.35.0rc1 == 0.35.0rc1"   btxd_version_line_matches "BTX Core daemon version v0.35.0rc1" 0.35.0rc1
want_fail "v0.34.150 != 0.34.15"      btxd_version_line_matches "BTX Core daemon version v0.34.150" 0.34.15
want_fail "v0.34.15.1 != 0.34.15"     btxd_version_line_matches "BTX Core daemon version v0.34.15.1" 0.34.15
want_ok   "fake btxd reports exact"   btxd_reports_exact_version "$r_old/bin/btxd" 0.34.15

echo
if [[ "$fails" == 0 ]]; then echo "release-archive: all checks passed"; else echo "release-archive: $fails FAILED"; exit 1; fi
