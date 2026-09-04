#!/usr/bin/env bash
# Publish a BTX Node release to the releases repo — the last manual mile.
#
# Every release up to 0.6.17 ended with a human driving the GitHub API by hand,
# and `docs/node-release-recipe.md` step 7 has specified this script by shape
# for three releases without anyone writing it. This is that script.
#
# WHAT IT REFUSES TO DO, which is the whole point:
#
#   1. It will not publish bytes the gates never saw. Every artifact must appear
#      in the SHA256SUMS passed with `--sums`, with a matching hash, AND must be
#      no newer than that file. A rebuild after the gate run bumps the mtime and
#      this script stops. That is the concrete meaning of "refuses untested
#      bytes": not a promise in a checklist, a comparison it performs.
#
#   2. It will not publish an artifact whose updater signature does not verify
#      against the embedded public key of the app. Verification runs before the
#      first network call, via verify-updater-sig.py next door — no minisign
#      binary needed, because the publishing box does not reliably have one.
#
#   3. It will never set this release as "Latest". `make_latest` is sent as the
#      STRING "false" (the API ignores a boolean here), and after publishing the
#      script re-reads /releases/latest and FAILS if a node tag captured it. A
#      node release that steals the repo-global Latest pointer breaks the
#      updater of the *miner*, which is a different product.
#
#   4. It will not leave a window where the tag resolves but the files 404. The
#      release is created as a DRAFT, assets are attached, and only then is it
#      flipped live.
#
#   5. It will not trust the bytes it just uploaded. After going live it
#      re-downloads every asset from its public URL, compares sha256 against the
#      local file, and re-verifies the downloaded .sig. A signature that only
#      ever verified locally proves nothing about what users will fetch.
#
# TOOLCHAIN. curl and python3 only. Measured on the Windows/WSL publishing box
# on 2026-09-04: `gh` is not installed and neither is `jq`, on either side of
# the WSL boundary. A publish script that needs them is a publish script that
# does not run where releases are actually cut, which is how we got here.
#
# Usage:
#   publish-node-release.sh --version 0.6.18 --sums SHA256SUMS \
#       [--linux <.AppImage>] [--mac <.app.tar.gz>] [--win <-setup.exe>] \
#       [--deb <.deb>] [--notes-file NOTES.md] [--repo owner/repo] [--dry-run]
#
#   Platform subsets are normal — see build-node-feed.sh for why. Pass only the
#   platforms this release actually ships.
#
# Token: $GITHUB_TOKEN or $GH_TOKEN, else prompted (never echoed, never logged).
# It needs `contents: write` on the releases repo and nothing else.
#
# ORDER. This runs AFTER the gates and BEFORE the feed is deployed:
#   gates -> sign -> publish-node-release.sh -> build-node-feed.sh -> site PR
# Deploying the feed first points users at assets that do not exist yet.
set -euo pipefail

VERSION="" ; SUMS="" ; NOTES_FILE="" ; DRY_RUN=0
REPO="MendeMatthias/EasyBTX-releases"
declare -a ASSETS=()

die()  { echo "error: $*" >&2 ; exit 1 ; }
note() { echo "  $*" ; }

while [ $# -gt 0 ]; do
  case "$1" in
    --version)    VERSION="${2:?}" ; shift 2 ;;
    --sums)       SUMS="${2:?}" ; shift 2 ;;
    --linux|--mac|--win|--deb|--asset)
                  ASSETS+=("${2:?}") ; shift 2 ;;
    --notes-file) NOTES_FILE="${2:?}" ; shift 2 ;;
    --repo)       REPO="${2:?}" ; shift 2 ;;
    --dry-run)    DRY_RUN=1 ; shift ;;
    -h|--help)    sed -n '1,54p' "$0" ; exit 0 ;;
    *) die "unknown argument: $1 (see --help)" ;;
  esac
done

[ -n "$VERSION" ] || die "--version is required"
[ -n "$SUMS" ]    || die "--sums is required; it is what makes untested bytes checkable"
[ -f "$SUMS" ]    || die "no such sums file: $SUMS"
[ "${#ASSETS[@]}" -gt 0 ] || die "pass at least one artifact (--linux/--mac/--win/--deb/--asset)"

TAG="node-v${VERSION}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VERIFY_SIG="$HERE/verify-updater-sig.py"
[ -f "$VERIFY_SIG" ] || die "verify-updater-sig.py not found beside this script"

# verify-updater-sig.py reads the pubkey the APP EMBEDS, so it wants the config
# rather than a pubkey on the command line — that is what makes it impossible to
# verify against the wrong key by passing the wrong argument. Its contract is
# `verify-updater-sig.py <tauri.conf.json> <artifact>`, and it derives the .sig
# path from the artifact itself.
TAURI_CONF="$HERE/../src-tauri/tauri.conf.json"
[ -f "$TAURI_CONF" ] || die "tauri.conf.json not found at $TAURI_CONF"

command -v curl    >/dev/null || die "curl not found"
command -v python3 >/dev/null || die "python3 not found"

echo "==> preflight: $TAG -> $REPO"

# ---------------------------------------------------------------------------
# 1. Local checks. Everything that can fail offline fails here, before we have
#    created anything on GitHub that would need cleaning up.
# ---------------------------------------------------------------------------
# Freshness is compared with `-nt`, not with `stat -c %Y`. stat prints whole
# seconds, so an artifact rebuilt in the same second as the sums file compares
# EQUAL and slips through — which the test suite caught. `-nt` uses the full
# timestamp the filesystem records.

for a in "${ASSETS[@]}"; do
  [ -f "$a" ] || die "no such artifact: $a"
  base="$(basename "$a")"

  case "$base" in
    *"$VERSION"*) : ;;
    *) die "artifact does not carry version $VERSION in its name: $base
       staging the wrong build is the mistake this catches." ;;
  esac

  [ -f "$a.sig" ] || die "missing updater signature: $a.sig"

  # (a) the hash must match what the gate run recorded
  want="$(awk -v f="$base" '$2 == f || $2 == "*" f {print $1}' "$SUMS" | head -1)"
  [ -n "$want" ] || die "$base is not listed in $SUMS — the gates never saw these bytes"
  got="$(sha256sum "$a" | awk '{print $1}')"
  [ "$want" = "$got" ] || die "sha256 mismatch for $base
       gates recorded: $want
       file on disk  : $got"

  # (b) the artifact must not be NEWER than the sums file. A rebuild after the
  #     gate run is exactly the untested-bytes case, and it is invisible to (a)
  #     if somebody regenerates the sums from the new build out of habit.
  if [ "$a" -nt "$SUMS" ]; then
    die "$base is newer than $SUMS — it was rebuilt after the gates ran.
       Re-run the gates and regenerate the sums, or publish the tested bytes."
  fi

  # (c) the updater signature must verify against the embedded pubkey
  python3 "$VERIFY_SIG" "$TAURI_CONF" "$a" >/dev/null \
    || die "updater signature does not verify for $base"

  note "ok  $base  ($(du -h "$a" | cut -f1))  hash + mtime + signature"
done

BODY_FILE="${NOTES_FILE:-}"
if [ -n "$BODY_FILE" ] && [ ! -f "$BODY_FILE" ]; then
  die "no such notes file: $BODY_FILE"
fi

if [ "$DRY_RUN" -eq 1 ]; then
  echo "==> dry run: every local check passed; nothing was published."
  exit 0
fi

# ---------------------------------------------------------------------------
# 2. Token. Prompted if absent, never echoed.
# ---------------------------------------------------------------------------
TOKEN="${GITHUB_TOKEN:-${GH_TOKEN:-}}"
if [ -z "$TOKEN" ]; then
  printf 'GitHub token (contents:write on %s), input hidden: ' "$REPO" >&2
  read -rs TOKEN
  printf '\n' >&2
fi
[ -n "$TOKEN" ] || die "no token"

API="https://api.github.com"
UPLOADS="https://uploads.github.com"
AUTH=(-H "Authorization: Bearer $TOKEN"
      -H "Accept: application/vnd.github+json"
      -H "X-GitHub-Api-Version: 2022-11-28")

# jq is not on the publishing box; python3 is. Reads one top-level key from a
# JSON object on stdin and prints nothing at all if the body is not an object,
# is not JSON, or lacks the key — so callers test for emptiness, not for luck.
jget() {
  python3 -c '
import sys, json
try:
    d = json.load(sys.stdin)
except Exception:
    sys.exit(0)
if not isinstance(d, dict):
    sys.exit(0)
v = d.get(sys.argv[1])
if v is None:
    sys.exit(0)
print(str(v).lower() if isinstance(v, bool) else v)
' "$1"
}

# ---------------------------------------------------------------------------
# 3. Refuse to clobber an existing tag.
# ---------------------------------------------------------------------------
existing="$(curl -sS "${AUTH[@]}" "$API/repos/$REPO/releases/tags/$TAG" || true)"
if [ -n "$(printf '%s' "$existing" | jget id)" ]; then
  die "$TAG already exists on $REPO. Delete it deliberately or bump the version;
       this script will not overwrite a published release."
fi

# ---------------------------------------------------------------------------
# 4. Create it as a DRAFT so the tag never resolves without its files.
# ---------------------------------------------------------------------------
echo "==> creating draft release $TAG"
payload="$(python3 -c '
import json, sys
tag, name, notes_path = sys.argv[1], sys.argv[2], sys.argv[3]
body = open(notes_path, encoding="utf-8").read() if notes_path else ""
print(json.dumps({"tag_name": tag, "name": name, "body": body,
                  "draft": True, "prerelease": False, "make_latest": "false"}))
' "$TAG" "easyBTX Node $VERSION" "${BODY_FILE:-}")"

created="$(printf '%s' "$payload" | curl -sS "${AUTH[@]}" -X POST \
  -H "Content-Type: application/json" --data-binary @- \
  "$API/repos/$REPO/releases")"
REL_ID="$(printf '%s' "$created" | jget id)"
[ -n "$REL_ID" ] || die "could not create draft release. API said:
$(printf '%s' "$created" | head -20)"
note "draft id $REL_ID"

# From here a failure leaves a DRAFT behind — harmless and invisible to users,
# but say so rather than letting the next person wonder.
cleanup_hint() {
  [ -n "${REL_ID:-}" ] && \
    echo "note: draft release $REL_ID left on $REPO; delete it or re-run." >&2
}
trap cleanup_hint ERR

# ---------------------------------------------------------------------------
# 5. Attach every artifact and its signature, plus the sums file itself.
# ---------------------------------------------------------------------------
upload_one() {
  local f="$1" base ctype resp
  base="$(basename "$f")"
  case "$base" in
    *.sig|*.json|SHA256SUMS*) ctype="text/plain" ;;
    *)                        ctype="application/octet-stream" ;;
  esac
  echo "==> uploading $base"
  resp="$(curl -sS "${AUTH[@]}" -X POST \
    -H "Content-Type: $ctype" \
    --data-binary @"$f" \
    "$UPLOADS/repos/$REPO/releases/$REL_ID/assets?name=$base")"
  [ -n "$(printf '%s' "$resp" | jget id)" ] || die "upload failed for $base. API said:
$(printf '%s' "$resp" | head -20)"
}

for a in "${ASSETS[@]}"; do
  upload_one "$a"
  upload_one "$a.sig"
done
upload_one "$SUMS"

# ---------------------------------------------------------------------------
# 6. Flip it live — still never Latest.
# ---------------------------------------------------------------------------
echo "==> publishing (draft=false, make_latest=false)"
live="$(printf '%s' '{"draft":false,"make_latest":"false"}' | curl -sS "${AUTH[@]}" -X PATCH \
  -H "Content-Type: application/json" --data-binary @- \
  "$API/repos/$REPO/releases/$REL_ID")"
[ "$(printf '%s' "$live" | jget draft)" = "false" ] || die "release did not flip live. API said:
$(printf '%s' "$live" | head -20)"
trap - ERR

# ---------------------------------------------------------------------------
# 7. Verify the RELEASED bytes, never the local ones.
# ---------------------------------------------------------------------------
echo "==> verifying published assets by re-download"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

for a in "${ASSETS[@]}"; do
  base="$(basename "$a")"
  url="https://github.com/$REPO/releases/download/$TAG/$base"
  curl -sSL -o "$TMP/$base"     "$url"     || die "could not re-download $base"
  curl -sSL -o "$TMP/$base.sig" "$url.sig" || die "could not re-download $base.sig"

  local_hash="$(sha256sum "$a"       | awk '{print $1}')"
  rel_hash="$(sha256sum "$TMP/$base" | awk '{print $1}')"
  [ "$local_hash" = "$rel_hash" ] || die "PUBLISHED BYTES DIFFER from the tested build for $base
       local:     $local_hash
       published: $rel_hash"

  python3 "$VERIFY_SIG" "$TAURI_CONF" "$TMP/$base" >/dev/null \
    || die "published signature does not verify for $base"
  note "ok  $base  round-tripped and verified"
done

# ---------------------------------------------------------------------------
# 8. The Latest pointer of the miner must be untouched.
# ---------------------------------------------------------------------------
echo "==> checking /releases/latest still belongs to the miner"
latest_tag="$(curl -sS "${AUTH[@]}" "$API/repos/$REPO/releases/latest" | jget tag_name)"
case "$latest_tag" in
  node-v*) die "a node release captured the repo-global Latest pointer ($latest_tag).
       This breaks the updater of the miner. Clear it in the GitHub UI now." ;;
  "")      echo "warning: could not read /releases/latest — check it by hand." >&2 ;;
  *)       note "Latest is still $latest_tag" ;;
esac

echo
echo "published: https://github.com/$REPO/releases/tag/$TAG"
echo "next: build-node-feed.sh --version $VERSION ... then the site PR."
