#!/usr/bin/env bash
#
# Would our USERS' MACHINES actually start on the engine tag we pin?
#
# WHAT THIS PROTECTS, and why check-engine-tag.sh is not enough.
#
# scripts/check-engine-tag.sh answers exactly one question: does this tag carry
# the withdrawn mainnet stall-recovery height. That is a real question with a
# real answer, but a tag can pass it and still be unshippable.
#
# v0.34.4 is the proof. It passes the fork guard cleanly, five assignments out
# of five on the disabled sentinel, and it cannot start on most of the fleet.
#
# THE THING THIS MODELS is the pairing of ENGINE and APP MODE, because neither
# alone decides whether a node starts.
#
#   btxd refuses a CONSENSUS start when the host's device class is not a row in
#   the sealed golden manifest. That manifest has shipped two rows since 0.34:
#   cuda/sm_120 and metal/m4_class. Measured on an RTX 3060 (cuda/sm_86) on
#   2026-08-29: v0.34.4 would not start. 0.34.5 changed this and allows a
#   DEGRADED start instead, logging "MatMul RC DEGRADED START" and withholding
#   NODE_MATMUL_CONSENSUS. Measured on the same 3060 against PR #128: it starts.
#
#   btxd from 0.34 onward also refuses a 1-of-1 TRUSTED MIRROR on mainnet.
#   crates/btx-core/src/node.rs sends every non-Metal host down exactly that
#   path with -matmultrustedthreshold=1.
#
# So the matrix, all four cells measured from the tags themselves:
#
#   engine        consensus start        1-of-1 mirror     our app works?
#   < 0.34        refused off-manifest   ALLOWED           yes, via the mirror
#   0.34 .. .4    refused off-manifest   refused           NO. no startable mode
#   0.34.5+       ALLOWED (degraded)     refused           yes, but only if the
#                                                          app stops using the
#                                                          mirror on that engine
#
# A bump into the middle row is a fleet-wide outage. A bump into the bottom row
# without the matching app change is the same outage. This guard catches both.
#
# FAIL CLOSED. A file it cannot fetch, a marker it cannot find, a manifest it
# cannot parse: all failures. This repo has twice shipped breakage behind a
# guard that quietly stopped matching and kept exiting 0. A guard that cannot
# fail is not a guard, so --self-test deliberately trips each check.
#
# WHAT THIS DOES NOT COVER, so nobody mistakes its green for full coverage.
#   * It does not run the engine. It reads that tag's own source.
#   * "Starts" is not "validates". A host outside the manifest starts on 0.34.5
#     and then stalls below the Epoch-A height. That is the honest outcome and
#     the release notes must say so.
#   * It says nothing about the fork constant (that is check-engine-tag.sh), the
#     withdrawn assumeutxo bases, or whether a machine is fast enough.
#   * Being IN the manifest is necessary, not sufficient. BTX's cuBLASLt request
#     gets no IMMA kernel on any pre-Hopper NVIDIA card, so 30 and 40 series
#     owners are excluded even before the manifest is consulted. See
#     docs/2026-08-29-ampere-imma-layout.md.
#
# USAGE
#   scripts/check-engine-fleet-ready.sh              check the pin in commands.rs
#   scripts/check-engine-fleet-ready.sh v0.34.5      check any tag
#   scripts/check-engine-fleet-ready.sh --self-test  prove each check can fail

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMMANDS_RS="$ROOT/apps/node/src-tauri/src/commands.rs"
NODE_RS="$ROOT/crates/btx-core/src/node.rs"
INIT_CPP="src/init.cpp"
MANIFEST="src/matmul/matmul_v4_rc_production_golden_manifest.data"
BTX_CLONE="${BTX_CLONE:-/Users/bonuz/repos/btx}"
RAW_BASE="https://raw.githubusercontent.com/btxchain/btx"

# 0.34.5 introduced this line. Its presence means an off-manifest host starts.
# Measured across every tag on 2026-08-29: absent on v0.33.3 through v0.34.4,
# present on PR #128. Do NOT use the refusal message as the discriminator: that
# string exists as far back as v0.33.3, because the InitError call site is old
# and 0.34.5 neutered the predicate under it rather than deleting the message.
DEGRADED_START_MARKER="MatMul RC DEGRADED START"
# Added in 0.34. Absent on every v0.33.x tag.
SINGLE_KEY_REFUSAL="Mainnet trusted MatMul mirrors require at least 2"
# The app-side marker that says node.rs knows to stay in consensus mode on an
# engine that allows a degraded start, instead of taking the refused mirror.
APP_DEGRADED_GATE="node_allows_degraded_matmul_start"

annotate() {
  if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::$1::$2"; fi
}
die() {
  echo "FAIL: $1" >&2
  annotate error "engine fleet guard: $1"
  shift
  for line in "$@"; do echo "  $line" >&2; done
  exit 1
}

fetch_at_tag() {
  tag="$1"; path="$2"; dest="$3"
  if [ -d "$BTX_CLONE/.git" ] \
     && git -C "$BTX_CLONE" rev-parse --verify --quiet "$tag^{commit}" >/dev/null 2>&1 \
     && git -C "$BTX_CLONE" show "$tag:$path" > "$dest" 2>/dev/null; then
    echo "local clone $BTX_CLONE"
    return 0
  fi
  # curl --max-time, NOT timeout: timeout does not exist on macOS and this
  # script runs on both.
  if curl -fsSL --max-time 60 "$RAW_BASE/$tag/$path" -o "$dest" 2>/dev/null; then
    echo "$RAW_BASE/$tag/$path"
    return 0
  fi
  return 1
}

# =============================== self test ==================================
if [ "${1:-}" = "--self-test" ]; then
  fails=0
  t="$(mktemp -d)"; trap 'rm -rf "$t"' EXIT

  printf 'int main() { return 0; }\n' > "$t/plain.cpp"
  printf 'LogPrintf("%s: ...");\n' "$DEGRADED_START_MARKER" > "$t/degraded.cpp"
  printf 'InitError(_("%s independent signers"));\n' "$SINGLE_KEY_REFUSAL" > "$t/mirror.cpp"
  printf 'fn %s(p: &Path) -> bool { true }\n' "$APP_DEGRADED_GATE" > "$t/node_gated.rs"
  printf 'fn something_else() {}\n' > "$t/node_ungated.rs"

  grep -q "$DEGRADED_START_MARKER" "$t/plain.cpp"    && { echo "self-test: degraded check matched a file without the marker"; fails=1; }
  grep -q "$DEGRADED_START_MARKER" "$t/degraded.cpp" || { echo "self-test: degraded check MISSED the marker"; fails=1; }
  grep -q "$SINGLE_KEY_REFUSAL"    "$t/plain.cpp"    && { echo "self-test: mirror check matched a file without the refusal"; fails=1; }
  grep -q "$SINGLE_KEY_REFUSAL"    "$t/mirror.cpp"   || { echo "self-test: mirror check MISSED the refusal"; fails=1; }
  grep -q "$APP_DEGRADED_GATE"     "$t/node_ungated.rs" && { echo "self-test: app-gate check matched an ungated node.rs"; fails=1; }
  grep -q "$APP_DEGRADED_GATE"     "$t/node_gated.rs"   || { echo "self-test: app-gate check MISSED the gate"; fails=1; }

  printf 'BTX_RC_PRODUCTION_GOLDEN_V1\n' > "$t/m0.data"
  printf 'BTX_RC_PRODUCTION_GOLDEN_V1\nid|cuda|sm_120|1|d|1|doc/x|r|f|h\n' > "$t/m1.data"
  n0="$(awk -F'|' 'NR>1 && NF>3 {print $2"/"$3}' "$t/m0.data" | grep -c . || true)"
  n1="$(awk -F'|' 'NR>1 && NF>3 {print $2"/"$3}' "$t/m1.data" | grep -c . || true)"
  [ "$n0" = "0" ] || { echo "self-test: header-only manifest read as $n0 rows"; fails=1; }
  [ "$n1" = "1" ] || { echo "self-test: one-row manifest read as $n1 rows"; fails=1; }

  [ "$fails" -eq 0 ] || die "self-test failed; the checks above are not doing what they claim"
  echo "OK: self-test passed. Every check matches what it should and misses what it should."
  exit 0
fi

# --- 1. which tag ----------------------------------------------------------
OVERRIDE_TAG="${1:-}"
if [ -n "$OVERRIDE_TAG" ]; then
  TAG="$OVERRIDE_TAG"
  echo "checking tag $TAG (explicit argument, the pin in commands.rs is ignored)"
else
  [ -f "$COMMANDS_RS" ] || die "cannot find $COMMANDS_RS" \
    "The app moved. Point this guard at the new path rather than skipping it."
  TAG="$(sed -n 's/^pub const NODE_RELEASE_TAG: &str = "\([^"]*\)";.*$/\1/p' "$COMMANDS_RS" | head -1)"
  [ -n "$TAG" ] || die "could not read NODE_RELEASE_TAG from $COMMANDS_RS" \
    "Its shape changed. FIX THIS GUARD, do not delete it and do not skip it."
  echo "pinned engine tag: $TAG (from apps/node/src-tauri/src/commands.rs)"
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

INIT_FILE="$WORK/init.cpp"
SRC_INIT="$(fetch_at_tag "$TAG" "$INIT_CPP" "$INIT_FILE")" || die \
  "could not read $INIT_CPP for tag $TAG" \
  "Tried $BTX_CLONE and $RAW_BASE/$TAG/$INIT_CPP." \
  "An unverifiable tag is not a safe tag."
[ -s "$INIT_FILE" ] || die "fetched an empty $INIT_CPP for tag $TAG" "Source was: $SRC_INIT"

MANIFEST_FILE="$WORK/manifest.data"
SRC_MANIFEST="$(fetch_at_tag "$TAG" "$MANIFEST" "$MANIFEST_FILE")" || die \
  "could not read $MANIFEST for tag $TAG" \
  "Tried $BTX_CLONE and $RAW_BASE/$TAG/$MANIFEST."
[ -s "$MANIFEST_FILE" ] || die "fetched an empty golden manifest for tag $TAG"
echo "source: $SRC_INIT"
echo

# --- 2. what does the ENGINE allow? ----------------------------------------
CONSENSUS_STARTS=0
if grep -q "$DEGRADED_START_MARKER" "$INIT_FILE"; then CONSENSUS_STARTS=1; fi
MIRROR_STARTS=1
if grep -q "$SINGLE_KEY_REFUSAL" "$INIT_FILE"; then MIRROR_STARTS=0; fi

if [ "$CONSENSUS_STARTS" -eq 1 ]; then
  echo "consensus mode ......... starts off-manifest (degraded, no consensus service bit)"
else
  echo "consensus mode ......... REFUSES to start off-manifest"
fi
if [ "$MIRROR_STARTS" -eq 1 ]; then
  echo "1-of-1 trusted mirror .. accepted on mainnet"
else
  echo "1-of-1 trusted mirror .. REFUSED on mainnet"
fi

# --- 3. what does the APP choose? ------------------------------------------
[ -f "$NODE_RS" ] || die "cannot find $NODE_RS" \
  "This guard has to know which mode the app selects. Point it at the new path."
APP_PINS_SINGLE_KEY=0
if grep -q -- "-matmultrustedthreshold=1" "$NODE_RS"; then APP_PINS_SINGLE_KEY=1; fi
APP_HAS_DEGRADED_GATE=0
if grep -q "$APP_DEGRADED_GATE" "$NODE_RS"; then APP_HAS_DEGRADED_GATE=1; fi

if [ "$APP_PINS_SINGLE_KEY" -eq 1 ]; then
  echo "app off-manifest path .. trusted mirror, threshold 1"
else
  echo "app off-manifest path .. no 1-of-1 mirror pin found"
fi
if [ "$APP_HAS_DEGRADED_GATE" -eq 1 ]; then
  echo "app degraded-start gate  present ($APP_DEGRADED_GATE)"
else
  echo "app degraded-start gate  ABSENT"
fi
echo

# --- 4. who can validate independently on this tag? ------------------------
ROWS="$(awk -F'|' 'NR>1 && NF>3 {print $2"/"$3}' "$MANIFEST_FILE" || true)"
ROW_COUNT="$(printf '%s' "$ROWS" | grep -c . || true)"
PROBLEMS=""
note() { PROBLEMS="${PROBLEMS}$1"$'\n'; }

if [ "$ROW_COUNT" -eq 0 ]; then
  note "the golden manifest at this tag has zero device rows, so nobody can validate above the Epoch-A height (or its shape changed and this guard can no longer read it, which is equally a failure)"
else
  echo "golden manifest ........ $ROW_COUNT device class(es) validate independently:"
  printf '%s\n' "$ROWS" | sed 's/^/      /'
  echo "    Everything else starts, follows, and stalls below the fork."
  echo "    Say that plainly in the release notes. Do not imply otherwise."
fi
echo

# --- 5. is there a startable configuration? --------------------------------
if [ "$CONSENSUS_STARTS" -eq 0 ] && [ "$MIRROR_STARTS" -eq 0 ]; then
  note "no startable mode exists on this tag for a host outside the golden manifest: consensus exits at init and a 1-of-1 trusted mirror is refused"
elif [ "$CONSENSUS_STARTS" -eq 1 ] && [ "$MIRROR_STARTS" -eq 0 ] \
     && [ "$APP_PINS_SINGLE_KEY" -eq 1 ] && [ "$APP_HAS_DEGRADED_GATE" -eq 0 ]; then
  note "this tag allows a degraded consensus start but refuses the 1-of-1 mirror the app still selects; crates/btx-core/src/node.rs needs a $APP_DEGRADED_GATE gate so off-manifest hosts stay in consensus mode instead"
fi

if [ -z "$PROBLEMS" ]; then
  if [ "$CONSENSUS_STARTS" -eq 1 ]; then
    echo "OK: $TAG is fleet-startable, off-manifest hosts via a degraded consensus start."
  else
    echo "OK: $TAG is fleet-startable, off-manifest hosts via the trusted mirror."
  fi
  echo "    This says nothing about the fork constant. Run scripts/check-engine-tag.sh too."
  exit 0
fi

echo "$TAG is NOT fleet-ready. Shipping it would break users at startup:"
printf '%s' "$PROBLEMS" | sed 's/^/    - /'
echo
echo "A node that refuses to start is worse for a user than a stale node that runs."
die "engine tag $TAG is not fleet-startable with the app as written"
