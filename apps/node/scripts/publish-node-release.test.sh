#!/usr/bin/env bash
# Offline tests for publish-node-release.sh.
#
# WHY THIS EXISTS. The publish script is the one script in this repo that is
# hard to rehearse: its happy path needs the release key and write access to the
# releases repo, so it would otherwise be first exercised on the day it matters.
# Its VALUE, though, is entirely in what it refuses, and every refusal is
# offline. So the refusals are what get tested, on every pull request.
#
# Nothing here touches the network. Every case is expected to fail before the
# first API call, and the one case that would proceed is stopped by --dry-run.
#
# Run it directly: apps/node/scripts/publish-node-release.test.sh
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
P="$HERE/publish-node-release.sh"
[ -f "$P" ] || { echo "publish-node-release.sh not found beside this test"; exit 1; }

bash -n "$P" || { echo "SYNTAX ERROR in publish-node-release.sh"; exit 1; }
echo "bash -n: OK"

T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
cd "$T"

V=0.6.18
A="BTX-Node_${V}_amd64.AppImage"
pass=0; fail=0

# expect_fail <label> <substring the error must contain> -- <args to the script>
expect_fail() {
  local label="$1" want="$2"; shift 3
  local out rc
  out="$(bash "$P" "$@" 2>&1)"; rc=$?
  if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -qF -- "$want"; then
    echo "  PASS  $label"; pass=$((pass+1))
  else
    echo "  FAIL  $label (rc=$rc, wanted: $want)"
    printf '%s\n' "$out" | head -4 | sed 's/^/        /'
    fail=$((fail+1))
  fi
}

echo
echo "=== argument validation ==="
expect_fail "no --version"          "--version is required"        -- --sums x
expect_fail "no --sums"             "--sums is required"           -- --version "$V"
expect_fail "no artifacts"          "pass at least one"            -- --version "$V" --sums /etc/hostname

echo
echo "=== artifact identity ==="
head -c 1048576 /dev/urandom > "$A"
sha256sum "$A" > SHA256SUMS
touch SHA256SUMS                      # sums newer than the artifact: the good case

expect_fail "missing artifact"      "no such artifact" \
  -- --version "$V" --sums SHA256SUMS --linux missing.AppImage

cp "$A" "BTX-Node_9.9.9_amd64.AppImage"
expect_fail "wrong version in name" "does not carry version" \
  -- --version "$V" --sums SHA256SUMS --linux "BTX-Node_9.9.9_amd64.AppImage"

expect_fail "missing .sig"          "missing updater signature" \
  -- --version "$V" --sums SHA256SUMS --linux "$A"

echo "not-a-real-signature" > "$A.sig"

echo
echo "=== the bytes must be the bytes the gates hashed ==="
echo "deadbeef  someone-elses-file" > OTHER_SUMS
expect_fail "not listed in sums"    "the gates never saw these bytes" \
  -- --version "$V" --sums OTHER_SUMS --linux "$A"

sed 's/^./0/' SHA256SUMS > BAD_SUMS
expect_fail "hash mismatch"         "sha256 mismatch" \
  -- --version "$V" --sums BAD_SUMS --linux "$A"

echo
echo "=== rebuilt after the gates ran ==="
# The regression this pins: an earlier draft compared `stat -c %Y`, whole
# seconds, so an artifact rebuilt in the same second as the sums file compared
# EQUAL and was accepted. The script uses `-nt` now.
touch "$A"
expect_fail "rebuilt after gates"   "it was rebuilt after the gates ran" \
  -- --version "$V" --sums SHA256SUMS --linux "$A"

echo
echo "=== signature (reached only once everything above passes) ==="
sha256sum "$A" > SHA256SUMS           # rehash and make the sums newest again
expect_fail "bad signature"         "updater signature does not verify" \
  -- --version "$V" --sums SHA256SUMS --linux "$A"

echo
echo "--------------------------------------------"
echo "  passed $pass, failed $fail"
[ "$fail" -eq 0 ] || exit 1
