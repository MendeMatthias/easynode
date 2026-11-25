#!/usr/bin/env bash
# The ModelNet engine's pin, checked rather than remembered.
#
# WHY THIS FILE EXISTS. The default engine is pinned in the installer workflows
# (PROVEN_BTXD_RUN_ID and friends) and travels inside every installer. The
# ModelNet engine does not: an installed Linux app downloads it when its user
# switches the swarm on, and checks the download against a pin. That pin is
# apps/node/engine-modelnet-linux.pin, and this script is the only thing that
# compares it with the rest of the repository.
#
# What it refuses:
#
#   1. a pin with a key missing, misspelt or of the wrong shape;
#   2. a ModelNet engine built from another commit, or reporting another
#      version, than the engine the app pins (NODE_RELEASE_COMMIT and
#      NODE_RELEASE_TAG in commands.rs). The two engines must be the same BTX:
#      a node that switches the swarm on must not change consensus code;
#   3. artifact and archive names that are not the ones btxd-linux.yml mints
#      for that commit;
#   4. the two pins pointing at each other: the installer's PROVEN_BTXD pin
#      naming a ModelNet artifact, or this pin naming the installer's run.
#
# It reads files and runs nothing else. Usage:
#
#   check-modelnet-engine-pin.sh                 # the repository's own pin
#   check-modelnet-engine-pin.sh <pin file>      # another file (the self-test)
#   check-modelnet-engine-pin.sh --self-test     # prove each refusal fires
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$(cd "$HERE/.." && pwd)"
ROOT="$(cd "$APP_DIR/../.." && pwd)"
PIN_DEFAULT="$APP_DIR/engine-modelnet-linux.pin"
INSTALLER_WF="$ROOT/.github/workflows/node-linux-installer.yml"
ARCHIVE_PREFIX="easynode-engine-linux-x86_64-modelnet"

# shellcheck source=/dev/null
. "$HERE/lib/engine-pin.sh"

KEYS="MODELNET_ENGINE_RUN_ID MODELNET_ENGINE_ARTIFACT MODELNET_ENGINE_COMMIT
MODELNET_ENGINE_VERSION MODELNET_ENGINE_BTXD_BUILD_ID MODELNET_ENGINE_BTXD_SHA256
MODELNET_ENGINE_ARCHIVE MODELNET_ENGINE_ARCHIVE_SIZE MODELNET_ENGINE_ARCHIVE_SHA256
MODELNET_ENGINE_UNPACKED_SIZE MODELNET_ENGINE_FILE_COUNT MODELNET_ENGINE_FILES_SHA256"

# pin_get FILE KEY: the value of `KEY="value"`. The file is PARSED, never
# sourced: a pin file is data.
pin_get() {
  sed -n "s/^$2=\"\\([^\"]*\\)\"\$/\\1/p" "$1"
}

check_pin() {
  local pin=$1 rc=0 k v n want
  local run art commit version build_id btxd_sha archive size archive_sha files_sha unpacked count
  fail() { echo "error: $*" >&2; rc=1; }

  [[ -f "$pin" ]] || { echo "error: no pin file at $pin" >&2; return 1; }
  for k in $KEYS; do
    n="$(grep -c "^$k=" "$pin" || true)"
    [[ "$n" == 1 ]] || fail "$k must be set exactly once in $(basename "$pin") (found $n)"
  done
  # Nothing but comments, blank lines and the known keys.
  while IFS= read -r v; do
    k="${v%%=*}"
    case " $(echo $KEYS) " in
      *" $k "*) ;;
      *) fail "unknown line in $(basename "$pin"): $v" ;;
    esac
  done < <(grep -vE '^(#|$)' "$pin" || true)
  [[ $rc -eq 0 ]] || return 1

  run="$(pin_get "$pin" MODELNET_ENGINE_RUN_ID)"
  art="$(pin_get "$pin" MODELNET_ENGINE_ARTIFACT)"
  commit="$(pin_get "$pin" MODELNET_ENGINE_COMMIT)"
  version="$(pin_get "$pin" MODELNET_ENGINE_VERSION)"
  build_id="$(pin_get "$pin" MODELNET_ENGINE_BTXD_BUILD_ID)"
  btxd_sha="$(pin_get "$pin" MODELNET_ENGINE_BTXD_SHA256)"
  archive="$(pin_get "$pin" MODELNET_ENGINE_ARCHIVE)"
  size="$(pin_get "$pin" MODELNET_ENGINE_ARCHIVE_SIZE)"
  archive_sha="$(pin_get "$pin" MODELNET_ENGINE_ARCHIVE_SHA256)"
  files_sha="$(pin_get "$pin" MODELNET_ENGINE_FILES_SHA256)"
  unpacked="$(pin_get "$pin" MODELNET_ENGINE_UNPACKED_SIZE)"
  count="$(pin_get "$pin" MODELNET_ENGINE_FILE_COUNT)"

  [[ "$run" =~ ^[0-9]+$ ]] || fail "MODELNET_ENGINE_RUN_ID is not a run id: '$run'"
  [[ "$commit" =~ ^[0-9a-f]{40}$ ]] || fail "MODELNET_ENGINE_COMMIT is not a full commit: '$commit'"
  [[ "$version" =~ ^v[0-9]+(\.[0-9]+)+$ ]] || fail "MODELNET_ENGINE_VERSION is not a version: '$version'"
  [[ "$build_id" =~ ^[0-9a-f]{40}$ ]] || fail "MODELNET_ENGINE_BTXD_BUILD_ID is not a BuildID: '$build_id'"
  [[ "$btxd_sha" =~ ^[0-9a-f]{64}$ ]] || fail "MODELNET_ENGINE_BTXD_SHA256 is not a sha256: '$btxd_sha'"
  [[ "$archive_sha" =~ ^[0-9a-f]{64}$ ]] || fail "MODELNET_ENGINE_ARCHIVE_SHA256 is not a sha256: '$archive_sha'"
  [[ "$files_sha" =~ ^[0-9a-f]{64}$ ]] || fail "MODELNET_ENGINE_FILES_SHA256 is not a sha256: '$files_sha'"
  [[ "$size" =~ ^[1-9][0-9]*$ ]] || fail "MODELNET_ENGINE_ARCHIVE_SIZE is not a size: '$size'"
  [[ "$unpacked" =~ ^[1-9][0-9]*$ ]] || fail "MODELNET_ENGINE_UNPACKED_SIZE is not a size: '$unpacked'"
  [[ "$count" =~ ^[1-9][0-9]*$ ]] || fail "MODELNET_ENGINE_FILE_COUNT is not a count: '$count'"
  [[ $rc -eq 0 ]] || return 1

  # The same BTX as the engine the app pins.
  want="$(engine_pin_commit "$APP_DIR")"
  if [[ -z "$want" ]]; then
    fail "commands.rs has no NODE_RELEASE_COMMIT to compare the ModelNet engine's commit with"
  elif [[ "$commit" != "$want" ]]; then
    fail "the ModelNet engine is built from $commit, the app pins $want."
    fail "Build it again from the app's commit (btxd-linux.yml, modelnet: on) and move every line of the pin."
  fi
  want="$(engine_pin_version "$APP_DIR")"
  [[ "$version" == "$want" ]] || fail "the ModelNet engine reports $version, the app's engine $want"

  # The names btxd-linux.yml mints for that commit.
  [[ "$art" == "btxd-linux-modelnet-$commit" ]] \
    || fail "MODELNET_ENGINE_ARTIFACT is '$art', a ModelNet build of $commit is named btxd-linux-modelnet-$commit"
  [[ "$archive" == "$ARCHIVE_PREFIX-$version-${commit:0:8}.tar.gz" ]] \
    || fail "MODELNET_ENGINE_ARCHIVE is '$archive', the archive of $version at $commit is named $ARCHIVE_PREFIX-$version-${commit:0:8}.tar.gz"

  # The two pins must never point at each other.
  if [[ -f "$INSTALLER_WF" ]]; then
    v="$(sed -n 's/^  PROVEN_BTXD_ARTIFACT: "\([A-Za-z0-9._-]*\)"$/\1/p' "$INSTALLER_WF")"
    case "$v" in
      *modelnet*) fail "node-linux-installer.yml pins the ModelNet artifact '$v' as the DEFAULT engine" ;;
    esac
    v="$(sed -n 's/^  PROVEN_BTXD_RUN_ID: "\([0-9]*\)"$/\1/p' "$INSTALLER_WF")"
    [[ -z "$v" || "$v" != "$run" ]] \
      || fail "MODELNET_ENGINE_RUN_ID $run is the installer's PROVEN_BTXD_RUN_ID: that run built the default engine"
  else
    fail "no $INSTALLER_WF to compare with"
  fi
  [[ $rc -eq 0 ]] || return 1
  echo "ModelNet engine pin OK: run $run, $archive ($size bytes), btxd BuildID $build_id, same commit and version as the app's engine"
}

self_test() {
  local t bad n=0
  t="$(mktemp -d)"
  check_pin "$PIN_DEFAULT" >/dev/null || { echo "self-test: the repository's own pin does not pass" >&2; return 1; }
  # Each line: what is done to a copy of the real pin. Every one must be refused.
  while IFS='|' read -r what expr; do
    bad="$t/pin"
    sed -E "$expr" "$PIN_DEFAULT" > "$bad"
    if cmp -s "$bad" "$PIN_DEFAULT"; then
      echo "self-test: '$what' changed nothing, the test is dead" >&2; return 1
    fi
    if check_pin "$bad" >/dev/null 2>&1; then
      echo "self-test: a pin with $what was ACCEPTED" >&2; return 1
    fi
    n=$((n + 1))
  done <<'CASES'
another commit|s/^(MODELNET_ENGINE_COMMIT=")[0-9a-f]{4}/\1ffff/
another version|s/^(MODELNET_ENGINE_VERSION="v)[0-9]+/\199/
the default engine's artifact name|s/^(MODELNET_ENGINE_ARTIFACT="btxd-linux-)modelnet-/\1/
a short archive sha256|s/^(MODELNET_ENGINE_ARCHIVE_SHA256="[0-9a-f]{60})[0-9a-f]{4}/\1/
no BuildID|/^MODELNET_ENGINE_BTXD_BUILD_ID=/d
a run id that is not a number|s/^(MODELNET_ENGINE_RUN_ID=")[0-9]+/\1latest/
an archive named for another commit|s/^(MODELNET_ENGINE_ARCHIVE=".*-)[0-9a-f]{8}(\.tar\.gz")/\100000000\2/
a file count of zero|s/^(MODELNET_ENGINE_FILE_COUNT=")[0-9]+/\10/
a line that is no key|$ a PROVEN_BTXD_RUN_ID="1"
CASES
  # ...and the installer's own run id as the ModelNet run.
  bad="$t/pin"
  sed -E "s/^(MODELNET_ENGINE_RUN_ID=\")[0-9]+/\\1$(sed -n 's/^  PROVEN_BTXD_RUN_ID: "\([0-9]*\)"$/\1/p' "$INSTALLER_WF")/" "$PIN_DEFAULT" > "$bad"
  if check_pin "$bad" >/dev/null 2>&1; then
    echo "self-test: the installer's run id was ACCEPTED as the ModelNet run" >&2; return 1
  fi
  n=$((n + 1))
  rm -rf "$t"
  echo "self-test OK: the real pin passes and $n broken copies of it are each refused"
}

case "${1:-}" in
  --self-test) self_test ;;
  "") check_pin "$PIN_DEFAULT" ;;
  *) check_pin "$1" ;;
esac
