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

# ── The bytes must be the bytes the gates ran against ───────────────────────
# docs/node-release-recipe.md has promised this since it was written and nothing
# performed it. It is the one check the signature CANNOT stand in for: a
# signature proves "these bytes, signed by our key", which is equally true of a
# rebuild made ten minutes after the gates passed. Byte-identity to the gate
# run's SHA256SUMS is what connects "the gates went green" to "this is what you
# are shipping".
#
# The workflow writes SHA256SUMS beside the artifacts it produced, so it travels
# in the asset dir with them.
SUMS="$ASSET_DIR/SHA256SUMS"
if [[ ! -f "$SUMS" ]]; then
  echo "error: no SHA256SUMS in $ASSET_DIR." >&2
  echo "       It comes from the gate run that built these artifacts, and without" >&2
  echo "       it there is nothing tying these bytes to a run that passed. Fetch" >&2
  echo "       it from the run, or rebuild through the workflow." >&2
  exit 1
fi

# Portable sha256: the release Mac has shasum, Linux boxes have sha256sum.
sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

# The hash recorded for NAME in a sums file, or nothing. Structural parse:
# 64 hex, one separator (two spaces, or space-star for binary mode), the rest
# of the line is the name. Names with spaces are therefore fine, and a CR at
# the end of the line is ignored. Pure bash 3.2, so it runs on the release Mac.
sums_lookup() {
  local name="$1" sums="$2" line hash rest
  while IFS= read -r line || [ -n "$line" ]; do
    line="${line%$'\r'}"
    [ "${#line}" -gt 66 ] || continue
    hash="${line:0:64}"
    case "$hash" in *[!0-9a-fA-F]*) continue ;; esac
    rest="${line:64}"
    case "$rest" in
      "  "*) rest="${rest:2}" ;;
      " *"*) rest="${rest:2}" ;;
      *) continue ;;
    esac
    if [ "$rest" = "$name" ]; then
      printf '%s\n' "$hash" | tr 'A-F' 'a-f'
      return 0
    fi
  done < "$sums"
  return 0
}

bad=0
for f in "${ALL[@]}"; do
  case "$f" in
    *.sig|SHA256SUMS*|*.json) continue ;;
  esac
  # A sums line is 64 hex, a separator (two spaces, or space-star for binary
  # mode), then the NAME, which may contain spaces: it is everything after the
  # separator, never awk's $2. Compare the whole remainder so a substring like
  # BTX-Node_0.6.1_amd64.AppImage cannot satisfy 0.6.18, and drop a trailing CR
  # so a sums file that crossed a Windows box still reads.
  want="$(sums_lookup "$f" "$SUMS")"
  if [ -z "$want" ]; then
    echo "error: $f is not listed in SHA256SUMS." >&2
    echo "       The gates never saw this file. If it was rebuilt after the run," >&2
    echo "       that rebuild is what has to be published, gates and all." >&2
    bad=1
    continue
  fi
  got="$(sha256_of "$ASSET_DIR/$f")"
  if [ "$want" != "$got" ]; then
    echo "error: $f does not match the gate run." >&2
    echo "       SHA256SUMS: $want" >&2
    echo "       this file : $got" >&2
    bad=1
    continue
  fi
  # A file newer than the sums it matches is possible (touch, a copy) and is not
  # itself proof of anything, but it means the provenance story is not what it
  # looks like. Say so rather than pass silently.
  if [ "$ASSET_DIR/$f" -nt "$SUMS" ]; then
    echo "warning: $f is newer than SHA256SUMS but its hash matches." >&2
    echo "         Same bytes, later timestamp - a copy rather than a rebuild." >&2
  fi
  echo "      $f matches the gate run"
done
[ "$bad" -eq 0 ] || {
  echo >&2
  echo "Refusing to publish artifacts the gates did not produce." >&2
  exit 1
}
echo

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

# ── Everything above here is OFFLINE ────────────────────────────────────────
# The gh requirement lives HERE, not at the top, because the recipe promises a
# dry run "does every offline check and stops before the first API call" - and
# with the check at the top, a box without gh got `error: gh is required` and
# performed no checks at all. That is the box this script is for:
# node-release-recipe.md records, measured, that the publishing machine has
# neither gh nor jq. So the offline half now runs everywhere, and the tool is
# only demanded at the point it is actually used.
# ── Remember what Latest points at, so we can prove we did not steal it ─────
# One read-only GET. On a dry run it is a courtesy, not a requirement: a box
# without gh still gets every offline check and a green exit, which is what the
# recipe promises and what the publishing box, which has no gh, needs.
if command -v gh >/dev/null 2>&1; then
  PREV_LATEST="$(gh api "repos/$REPO/releases/latest" --jq '.tag_name' 2>/dev/null || echo "<none>")"
  echo "==> repo-global Latest currently: $PREV_LATEST"
  if [[ "$PREV_LATEST" == node-v* ]]; then
    echo "    ⚠ Latest is already a NODE release. It should be the miner's."
    echo "      Someone published a node release without make_latest=false."
  fi
  echo
else
  PREV_LATEST="<unknown: no gh>"
fi

if [ "$DO_PUBLISH" -eq 0 ]; then
  echo "==> DRY RUN complete. Every offline check passes."
  echo "    Would create draft $TAG on $REPO, upload ${#ALL[@]} asset(s),"
  echo "    re-download and verify each against the released bytes, then flip it live."
  exit 0
fi

command -v gh >/dev/null || {
  echo "error: gh is required from here on (creating the draft, uploading," >&2
  echo "       re-downloading and flipping it live all go through it)." >&2
  echo "       Every offline check above has passed. Install gh, or run the" >&2
  echo "       REST sequence in docs/node-release-recipe.md by hand." >&2
  exit 1
}

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
