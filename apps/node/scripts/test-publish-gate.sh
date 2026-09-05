#!/usr/bin/env bash
# Prove the publish script's SHA256SUMS gate against a CI-SHAPED sums file.
#
# The gate shipped once against a hand-written sums file that happened to match
# its author's assumptions, and could not pass for any real release: CI hashed
# the bundler's "easyBTX Node_<ver>_..." names (with a space) and the recipe
# renamed the assets afterwards. This fixture is what CI now produces, so the
# gate is tested against the input it will actually get. Runs in ci.yml.
#
# Only the OFFLINE half is exercised: the run stops at signature verification,
# which needs a real key. What is asserted is the sums gate's verdict per asset,
# read off the script's own output.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PUB="$HERE/publish-node-release.sh"
VER="$(python3 -c "import json;print(json.load(open('$HERE/../src-tauri/tauri.conf.json'))['version'])")"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
A="$T/assets"; mkdir -p "$A"

printf 'appimage bytes' > "$A/BTX-Node_${VER}_amd64.AppImage"
printf 'sig'            > "$A/BTX-Node_${VER}_amd64.AppImage.sig"
printf 'setup bytes'    > "$A/BTX-Node_${VER}_x64-setup.exe"
printf 'sig'            > "$A/BTX-Node_${VER}_x64-setup.exe.sig"
# Exactly what the two workflows emit: a Linux sums file (sha256sum *) and a
# Windows one (single line, may have crossed a Windows box: binary-mode star,
# CRLF). Concatenated the way an operator collects them into one asset dir.
( cd "$A" && sha256sum "BTX-Node_${VER}_amd64.AppImage" ) > "$A/SHA256SUMS"
printf '%s *%s\r\n' "$(sha256sum "$A/BTX-Node_${VER}_x64-setup.exe" | cut -d' ' -f1)" "BTX-Node_${VER}_x64-setup.exe" >> "$A/SHA256SUMS"

run() { bash "$PUB" "$VER" "$A" >"$T/out" 2>&1 || true; }

echo "== every asset matches: the gate must pass both, then stop at signatures =="
run
grep -q "BTX-Node_${VER}_amd64.AppImage matches the gate run" "$T/out" || { echo "FAIL: AppImage not matched"; cat "$T/out"; exit 1; }
grep -q "BTX-Node_${VER}_x64-setup.exe matches the gate run" "$T/out"  || { echo "FAIL: setup.exe not matched (star/CRLF line)"; cat "$T/out"; exit 1; }
grep -q "is not listed in SHA256SUMS" "$T/out" && { echo "FAIL: a listed asset was reported missing"; cat "$T/out"; exit 1; }
grep -q "does not match the gate run" "$T/out" && { echo "FAIL: a matching asset was reported tampered"; cat "$T/out"; exit 1; }
echo "   pass"

echo "== tampered bytes: refused =="
printf 'tampered' > "$A/BTX-Node_${VER}_amd64.AppImage"
run
grep -q "BTX-Node_${VER}_amd64.AppImage does not match the gate run" "$T/out" || { echo "FAIL: tampering not caught"; cat "$T/out"; exit 1; }
grep -q "Refusing to publish" "$T/out" || { echo "FAIL: no refusal"; exit 1; }
echo "   pass"

echo "== an asset the gates never saw: refused =="
printf 'orphan' > "$A/BTX-Node_${VER}_aarch64.app.tar.gz"; printf 'sig' > "$A/BTX-Node_${VER}_aarch64.app.tar.gz.sig"
run
grep -q "BTX-Node_${VER}_aarch64.app.tar.gz is not listed in SHA256SUMS" "$T/out" || { echo "FAIL: unlisted asset not caught"; cat "$T/out"; exit 1; }
echo "   pass"
echo "publish gate: all fixtures behave"
