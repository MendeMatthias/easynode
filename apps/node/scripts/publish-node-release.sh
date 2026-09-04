#!/usr/bin/env bash
# Publish a node release, the last mile the recipe describes and nothing
# implemented until now.
#
# WHAT THIS IS FOR. Every step before this one is scripted. This one was a
# paragraph of instructions, which is how three earlier releases went out with
# their signatures never verified against the bytes users would actually fetch.
#
# THE TWO WAYS THIS GOES WRONG, both guarded here:
#
#   1. Stealing the miner's "Latest" pointer. The node and the miner share the
#      EasyBTX-releases repository. GitHub keeps ONE repo-global Latest pointer,
#      and a node release that claims it breaks the MINER's updater for
#      everybody. `make_latest` must be the STRING "false", not a boolean, and
#      this script checks afterwards that /releases/latest still resolves to
#      whatever it did before.
#
#   2. A window where the tag resolves but the files 404. So: create as a
#      DRAFT, attach everything, verify, and only then flip it live.
#
# WHAT IT WILL NOT DO. It does not sign. The signing key lives on the
# maintainer's machine and in no repository, so this script only ever handles
# artifacts that are ALREADY signed, and refuses any it cannot verify.
#
# VERIFICATION IS AGAINST THE RELEASED BYTES, never the local copies. Every
# asset is re-downloaded from the draft and byte-compared, and every .sig is
# checked against the re-downloaded artifact under the pubkey embedded in
# tauri.conf.json. A signature that only ever verified locally proves nothing
# about what a user will fetch.
#
# Usage:
#   publish-node-release.sh <version> <asset-dir>            # dry run, checks only
#   publish-node-release.sh <version> <asset-dir> --publish  # actually do it
#
# Example:
#   publish-node-release.sh 0.6.18 ~/Desktop/BTX-Node-0.6.18-release
set -euo pipefail

APP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONF="$APP_DIR/src-tauri/tauri.conf.json"
VERIFY="$APP_DIR/scripts/verify-updater-sig.py"
REPO="MendeMatthias/EasyBTX-releases"

VERSION="${1:?usage: publish-node-release.sh <version> <asset-dir> [--publish]}"
ASSET_DIR="${2:?usage: publish-node-release.sh <version> <asset-dir> [--publish]}"
MODE="${3:-}"
TAG="node-v$VERSION"

DO_PUBLISH=0
[[ "$MODE" == "--publish" ]] && DO_PUBLISH=1
if [ "$DO_PUBLISH" -eq 0 ]; then
  echo "==> DRY RUN. Every check below runs; nothing is created or uploaded."
  echo "    Re-run with --publish once it is all green."
  echo
fi

command -v gh >/dev/null || { echo "error: gh is required" >&2; exit 1; }
[[ -f "$CONF" ]]      || { echo "error: no tauri.conf.json at $CONF" >&2; exit 1; }
[[ -x "$VERIFY" ]] || [[ -f "$VERIFY" ]] || { echo "error: no verify-updater-sig.py" >&2; exit 1; }
[[ -d "$ASSET_DIR" ]] || { echo "error: no asset dir at $ASSET_DIR" >&2; exit 1; }

# ── The version in the tree must be the version being published ─────────────
# Publishing 0.6.18 from a tree that says 0.6.17 produces an app whose About box
# disagrees with its own release, and an updater feed that points at the wrong
# thing. Cheap to check, confusing to debug.
TREE_VERSION="$(python3 -c "import json;print(json.load(open('$CONF'))['version'])")"
if [[ "$TREE_VERSION" != "$VERSION" ]]; then
  echo "error: tauri.conf.json says $TREE_VERSION but you asked to publish $VERSION." >&2
  echo "       Bump the version in the tree first, or publish $TREE_VERSION." >&2
  exit 1
fi

# ── Refuse untested bytes ───────────────────────────────────────────────────
# Every artifact that the updater will hand to a running app must arrive with a
# signature. An asset with no .sig is either a hand-built file or a signing step
# somebody skipped, and both are exactly what this script exists to stop.
shopt -s nullglob
# ⚠ No `mapfile`. It is a bash 4 builtin and macOS ships bash 3.2, which is the
# shell a release actually runs under on the release Mac. This read loop is the
# portable equivalent and was written after `mapfile: command not found` on the
# first run here.
SIGNED=(); ALL=()
while IFS= read -r line; do [ -n "$line" ] && SIGNED+=("$line"); done < <(cd "$ASSET_DIR" && ls -1 *.sig 2>/dev/null || true)
while IFS= read -r line; do [ -n "$line" ] && ALL+=("$line"); done < <(cd "$ASSET_DIR" && ls -1 2>/dev/null || true)
[ ${#ALL[@]} -gt 0 ] || { echo "error: $ASSET_DIR is empty" >&2; exit 1; }

echo "==> assets in $ASSET_DIR"
for f in "${ALL[@]}"; do echo "      $f"; done
echo

# Anything the updater serves (.tar.gz, .AppImage, .exe) needs a sibling .sig.
missing=0
for f in "${ALL[@]}"; do
  case "$f" in
    *.sig|SHA256SUMS*|*.json) continue ;;
    *.tar.gz|*.AppImage|*.exe)
      if [[ ! -f "$ASSET_DIR/$f.sig" ]]; then
        echo "error: $f has no $f.sig. The updater cannot serve an unsigned artifact." >&2
        missing=1
      fi ;;
  esac
done
[ "$missing" -eq 0 ] || exit 1

# ── Verify the LOCAL signatures before uploading anything ───────────────────
# Not the real check, which happens against the released bytes below, but there
# is no point uploading 450 MB to discover the signature was wrong.
echo "==> verifying local signatures against the pubkey in tauri.conf.json"
for s in "${SIGNED[@]}"; do
  artifact="${s%.sig}"
  [[ -f "$ASSET_DIR/$artifact" ]] || { echo "error: $s has no artifact $artifact" >&2; exit 1; }
  if python3 "$VERIFY" "$CONF" "$ASSET_DIR/$artifact" >/dev/null 2>&1; then
    echo "      ok   $artifact"
  else
    echo "error: signature does NOT verify for $artifact" >&2
    echo "       It was signed with a different key than the one this app trusts." >&2
    exit 1
  fi
done
echo

# ── Remember what Latest points at, so we can prove we did not steal it ─────
PREV_LATEST="$(gh api "repos/$REPO/releases/latest" --jq '.tag_name' 2>/dev/null || echo "<none>")"
echo "==> repo-global Latest currently: $PREV_LATEST"
if [[ "$PREV_LATEST" == node-v* ]]; then
  echo "    ⚠ Latest is already a NODE release. It should be the miner's."
  echo "      Someone published a node release without make_latest=false."
fi
echo

if [ "$DO_PUBLISH" -eq 0 ]; then
  echo "==> DRY RUN complete. Everything checked passes."
  echo "    Would create draft $TAG on $REPO, upload ${#ALL[@]} asset(s),"
  echo "    re-download and verify each against the released bytes, then flip it live."
  exit 0
fi

# ── Draft, upload, verify, and only then publish ────────────────────────────
if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
  echo "error: $TAG already exists on $REPO. Refusing to touch an existing release." >&2
  exit 1
fi

echo "==> creating DRAFT $TAG"
gh release create "$TAG" --repo "$REPO" --draft --title "easyNode $VERSION" \
  --notes "easyNode $VERSION. See https://easybtx.com/node/changelog" >/dev/null

echo "==> uploading ${#ALL[@]} asset(s)"
for f in "${ALL[@]}"; do
  gh release upload "$TAG" "$ASSET_DIR/$f" --repo "$REPO" >/dev/null
  echo "      uploaded $f"
done

# THE CHECK THAT MATTERS. Re-download from the release and compare bytes, then
# verify every signature against what was actually served.
echo
echo "==> re-downloading from the release and comparing against local bytes"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
gh release download "$TAG" --repo "$REPO" --dir "$tmp" >/dev/null
for f in "${ALL[@]}"; do
  if cmp -s "$ASSET_DIR/$f" "$tmp/$f"; then
    echo "      identical  $f"
  else
    echo "error: $f differs after upload. NOT publishing." >&2
    echo "       The draft is left in place so you can inspect it." >&2
    exit 1
  fi
done

echo
echo "==> verifying signatures against the RELEASED bytes"
for s in "${SIGNED[@]}"; do
  artifact="${s%.sig}"
  if python3 "$VERIFY" "$CONF" "$tmp/$artifact" >/dev/null 2>&1; then
    echo "      ok   $artifact"
  else
    echo "error: released $artifact does NOT verify. NOT publishing." >&2
    exit 1
  fi
done

# make_latest is the STRING "false". A boolean here silently does the wrong
# thing and hands the repo-global Latest pointer to the node, which breaks the
# miner's updater for every user.
echo
echo "==> flipping the draft live, make_latest=\"false\""
RELEASE_ID="$(gh api "repos/$REPO/releases/tags/$TAG" --jq '.id')"
gh api -X PATCH "repos/$REPO/releases/$RELEASE_ID" \
  -f draft=false -f make_latest=false >/dev/null

NOW_LATEST="$(gh api "repos/$REPO/releases/latest" --jq '.tag_name' 2>/dev/null || echo "<none>")"
echo "==> repo-global Latest is now: $NOW_LATEST"
if [[ "$NOW_LATEST" != "$PREV_LATEST" ]]; then
  echo >&2
  echo "⚠ LATEST MOVED, from $PREV_LATEST to $NOW_LATEST." >&2
  echo "  If it now names this node release, the miner's updater is pointing at" >&2
  echo "  the wrong thing. Fix it before doing anything else:" >&2
  echo "    gh api -X PATCH repos/$REPO/releases/<miner-release-id> -f make_latest=true" >&2
  exit 1
fi

echo
echo "==> published $TAG, and Latest is unchanged at $PREV_LATEST"
echo "    Next: regenerate the feed with build-node-feed.sh and deploy it."
echo "    The feed must not go out before these assets exist, and they now do."
