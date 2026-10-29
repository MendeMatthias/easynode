#!/usr/bin/env bash
# Publish the snapshot this node's keeper offers, so a new easyNode that
# follows signatures starts from it instead of the one compiled into its engine.
#
# ── WHY ─────────────────────────────────────────────────────────────────────
# "Serve a chain snapshot" (docs/snapshot-serve.md) exports the chain state at
# the tip every 500 blocks, signs it with this node's key and offers it over
# P2P. A new node rarely reaches this machine over P2P: it is at home, behind a
# router, at an address that moves. So from 0.6.31 easyNode fetches the pair
# over HTTPS instead, from MendeMatthias/EasyBTX-releases, and loads it with
# `loadtxoutsetattested` (crates/btx-core/src/attested_snapshot.rs). The
# engine refuses a pair not signed by a key the node pins, so the host and the
# pointer never have to be trusted. This script is the one thing that puts
# the pair there.
#
# ── WHAT IT DOES ────────────────────────────────────────────────────────────
#   1. Reads the keeper's record, <datadir>/snapshots/current-offer.json.
#   2. Checks the pair on disk against it (size, both SHA-256s) and that the
#      manifest carries the key every easyNode mirror pins (02d5efca…). A
#      pair signed by any other key would be refused by every node that
#      downloads it, so it is not published.
#   3. Creates the pre-release utxo-snapshot-<height> with the two files, the
#      tag and names the first published pair (225927) used. Skipped when it
#      exists.
#   4. Replaces utxo-snapshot-latest/attested-snapshot.json with the record,
#      and reads it back over the same URL the app uses.
#   5. Deletes utxo-snapshot-<n> pre-releases beyond the newest three, never
#      225927: the app pins that one as its fallback, and it must stay
#      published for as long as a release that pins it is installed.
# Idempotent: running it with nothing new published does nothing but check.
#
# ── WHAT IT NEEDS ───────────────────────────────────────────────────────────
#   * gh, logged in with write access to MendeMatthias/EasyBTX-releases
#   * python3 and curl
#   * the keeper running on this node ("Serve a chain snapshot" on)
#
# ── RUN IT ──────────────────────────────────────────────────────────────────
# Once by hand, then from cron every six hours (the keeper refreshes about
# every eleven):
#   17 */6 * * * /path/to/publish-attested-snapshot.sh >> ~/publish-attested-snapshot.log 2>&1
#
# Options:
#   --datadir DIR     the node's data folder (default: $EASYBTX_NODE_DATADIR,
#                     else the path in ~/.easybtx-location, else ~/.easybtx)
#   --repo OWNER/NAME where to publish (default MendeMatthias/EasyBTX-releases)
#   --signer PUBKEY   the key the manifest must carry (default 02d5efca…)
#   --dry-run         check everything and print what would be published

set -euo pipefail

REPO="MendeMatthias/EasyBTX-releases"
SIGNER="02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675"
POINTER_TAG="utxo-snapshot-latest"
POINTER_ASSET="attested-snapshot.json"
KEEP=3
# attested_snapshot::pinned_pair() in the app. Never deleted here.
PINNED_HEIGHT=225927
DRY=0
DATADIR="${EASYBTX_NODE_DATADIR:-}"

die() { echo "publish-attested-snapshot: $*" >&2; exit 1; }
say() { echo "[$(date -u +%Y-%m-%dT%H:%M:%SZ)] $*"; }
run() {
  if [ "$DRY" = 1 ]; then echo "  (dry run) $*"; else "$@"; fi
}

while [ $# -gt 0 ]; do
  case "$1" in
    --datadir) DATADIR="$2"; shift 2 ;;
    --repo) REPO="$2"; shift 2 ;;
    --signer) SIGNER="$2"; shift 2 ;;
    --dry-run) DRY=1; shift ;;
    -h|--help) sed -n '2,49p' "$0"; exit 0 ;;
    *) die "unknown option $1 (see --help)" ;;
  esac
done

if [ -z "$DATADIR" ] && [ -s "$HOME/.easybtx-location" ]; then
  # The app trims the file's contents the same way (datadir.rs).
  DATADIR="$(python3 -c 'import sys; print(open(sys.argv[1]).read().strip())' "$HOME/.easybtx-location")"
fi
DATADIR="${DATADIR:-$HOME/.easybtx}"
DIR="$DATADIR/snapshots"

command -v python3 >/dev/null || die "python3 is needed"
command -v curl >/dev/null || die "curl is needed"
command -v gh >/dev/null || die "gh is needed, logged in with write access to $REPO"
[ -f "$DIR/current-offer.json" ] ||
  die "no $DIR/current-offer.json. Is \"Serve a chain snapshot\" on, and has it offered a pair yet?"

# Every check in one place. Prints the height when the pair is publishable.
HEIGHT="$(python3 - "$DIR" "$SIGNER" <<'PY'
import hashlib, json, pathlib, sys

d = pathlib.Path(sys.argv[1])
signer = bytes.fromhex(sys.argv[2])
r = json.loads((d / "current-offer.json").read_text())
h = int(r["height"])
dat = d / f"utxo-btx-main-{h}.dat"
man = d / f"snapshot-manifest-{h}.json"

def sha256(p):
    x = hashlib.sha256()
    with open(p, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            x.update(block)
    return x.hexdigest()

problems = []
for p in (dat, man):
    if not p.is_file():
        problems.append(f"{p.name} is missing")
if not problems:
    if dat.stat().st_size != int(r["file_size"]):
        problems.append("the snapshot's size is not the record's")
    if sha256(dat) != r["sha256"].lower():
        problems.append("the snapshot's SHA-256 is not the record's")
    if sha256(man) != r["manifest_sha256"].lower():
        problems.append("the manifest's SHA-256 is not the record's")
    if signer not in man.read_bytes():
        problems.append(f"the manifest is not signed by {sys.argv[2][:8]}, the key easyNode mirrors pin")
if problems:
    print("; ".join(problems), file=sys.stderr)
    sys.exit(1)
print(h)
PY
)" || die "the pair on disk does not match the keeper's record; nothing published"

TAG="utxo-snapshot-$HEIGHT"
DAT="$DIR/utxo-btx-main-$HEIGHT.dat"
MAN="$DIR/snapshot-manifest-$HEIGHT.json"
BLOCK="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["block_hash"])' "$DIR/current-offer.json")"
say "the keeper offers $HEIGHT ($BLOCK); publishing to $REPO"

# 3. The pair.
if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
  say "$TAG is already published"
else
  run gh release create "$TAG" --repo "$REPO" --prerelease \
    --title "Signed UTXO snapshot, block $HEIGHT" \
    --notes "The chain state at block $HEIGHT ($BLOCK), exported and signed by easyNode's snapshot keeper with ${SIGNER:0:16}…, the key easyNode mirrors pin. A node that follows signatures loads it with loadtxoutsetattested; the engine checks the signature, so these files do not need to be trusted. Published by scripts/publish-attested-snapshot.sh." \
    "$DAT" "$MAN"
fi

# 4. The pointer, under the name the app asks for.
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
cp "$DIR/current-offer.json" "$STAGE/$POINTER_ASSET"
if ! gh release view "$POINTER_TAG" --repo "$REPO" >/dev/null 2>&1; then
  run gh release create "$POINTER_TAG" --repo "$REPO" --prerelease \
    --title "Newest signed UTXO snapshot (pointer)" \
    --notes "attested-snapshot.json names the newest utxo-snapshot-<height> pre-release. easyNode reads it; scripts/publish-attested-snapshot.sh replaces it."
fi
run gh release upload "$POINTER_TAG" "$STAGE/$POINTER_ASSET" --repo "$REPO" --clobber

if [ "$DRY" = 0 ]; then
  URL="https://github.com/$REPO/releases/download/$POINTER_TAG/$POINTER_ASSET"
  seen=""
  for _ in 1 2 3 4 5; do
    seen="$(curl -fsSL "$URL" 2>/dev/null | python3 -c 'import json,sys; print(json.load(sys.stdin)["height"])' 2>/dev/null || true)"
    [ "$seen" = "$HEIGHT" ] && break
    sleep 5
  done
  [ "$seen" = "$HEIGHT" ] || die "the pointer reads back as '${seen:-nothing}', not $HEIGHT"
  say "the pointer reads back as $HEIGHT"
fi

# 5. Older pairs.
gh release list --repo "$REPO" --limit 300 --json tagName --jq '.[].tagName' |
  sed -n 's/^utxo-snapshot-\([0-9][0-9]*\)$/\1/p' | sort -rn | tail -n +$((KEEP + 1)) |
  while read -r old; do
    [ "$old" = "$PINNED_HEIGHT" ] && continue
    [ "$old" = "$HEIGHT" ] && continue
    say "removing utxo-snapshot-$old"
    run gh release delete "utxo-snapshot-$old" --repo "$REPO" --yes --cleanup-tag
  done

say "done"
