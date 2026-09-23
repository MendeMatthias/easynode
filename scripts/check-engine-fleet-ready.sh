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
#   btxd from 0.34 onward also refuses a 1-of-1 TRUSTED MIRROR on mainnet,
#   unless -allowsinglekeytrustedmirror=1 is passed (0.34.5 added the flag as
#   a transition override). crates/btx-core/src/node.rs hands that mirror, at
#   -matmultrustedthreshold=1, to a refused Mac and, since 2026-09-15, to every
#   PC with no NVIDIA driver; a PC with the driver stays in consensus mode
#   (docs/decisions/2026-09-15-keyless-cpu-hosts-are-trusted-mirrors.md).
#
# So the matrix, all four cells measured from the tags themselves:
#
#   engine        consensus start        1-of-1 mirror     our app works?
#   < 0.34        refused off-manifest   ALLOWED           yes, via the mirror
#   0.34 .. .4    refused off-manifest   refused           NO. no startable mode
#   0.34.5+       ALLOWED (degraded)     refused without   yes: driver hosts via
#                                        the override      consensus, the rest
#                                                          via the mirror plus
#                                                          the override
#
# A bump into the middle row is a fleet-wide outage. A bump into the bottom row
# without the degraded-start gate, or with the gate but without the override on
# the mirror arm, is the same outage. This guard catches all three.
#
# A FOURTH PAIRING, added 2026-09-23 after it broke a smoke test rather than
# the fleet. Since 0.6.26 every host that validates also SIGNS: the app writes
# matmulattestationsignerkeyfile= into its conf and pins no key. 0.34.8 and
# 0.34.9 refuse exactly that at init ("-matmulattestationblocklist leaves 0
# unblocked pin member(s), below -matmultrustedthreshold=1", with an empty
# blocklist): upstream 235d39be made the unblocked-pin check unconditional and
# does not count the node's own secp256k1 key toward it, where 0.34.6 ran the
# check only when pins existed. On such a tag node.rs must pin the signer's own
# key (signing_key_self_pin), or every validating node fails to start.
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
#   * Being IN the manifest is necessary, not sufficient: the host must also
#     reproduce the row's golden digest. An earlier version of this note said no
#     pre-Hopper NVIDIA card could, so 30 and 40 series owners were excluded
#     before the manifest was consulted. That was wrong, and it must not become
#     recruitment copy: the mainnet signer since September 2026 is an RTX 3060
#     (cuda/sm_86), and the cohort check accepts a cuda row of any sm_*
#     architecture (matmul_v4_rc_production_canary.cpp,
#     GoldenArchitectureMatchesFamily). What binds is the digest and the
#     fingerprint, not the GPU generation.
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

# A pin that names its COMMIT. When commands.rs also declares
# NODE_RELEASE_COMMIT, the key in NODE_RELEASE_TAG may not be an upstream ref at
# all: 0.34.6 shipped from release/0.34.6 before upstream tagged it, and since
# 2026-09-15 the key carries a `-<short sha>` qualifier (`v0.34.6-3013c2c2`) so
# the app re-provisions onto the tag build. The guard then fetches source at
# that SHA instead, so such a pin gets exactly the check a bare tag gets. It
# never substitutes a branch name: a branch moves, a SHA does not. Empty when
# the key is itself a real upstream tag.
pin_commit() {
  sed -n 's/^pub const NODE_RELEASE_COMMIT: &str = "\([0-9a-f]\{40\}\)";.*$/\1/p' "$1" | head -1
}

# 0.34.5 introduced this line. Its presence means an off-manifest host starts.
# Measured across every tag on 2026-08-29: absent on v0.33.3 through v0.34.4,
# present on PR #128. Do NOT use the refusal message as the discriminator: that
# string exists as far back as v0.33.3, because the InitError call site is old
# and 0.34.5 neutered the predicate under it rather than deleting the message.
DEGRADED_START_MARKER="MatMul RC DEGRADED START"
# Added in 0.34. Absent on every v0.33.x tag.
SINGLE_KEY_REFUSAL="Mainnet trusted MatMul mirrors require at least 2"
# The app-side marker that says node.rs knows which engines allow a degraded
# start: on those a driver host takes consensus mode and a driver-less host
# takes the mirror WITH the override below, instead of the refused bare mirror.
APP_DEGRADED_GATE="node_allows_degraded_matmul_start"
# The override that makes a 1-of-1 mainnet mirror start on 0.34.5+. If node.rs
# pins a single key on such an engine and this literal is gone, every PC with
# no NVIDIA driver fails at init. Since 2026-09-15 that is the mirror arm.
APP_SINGLE_KEY_OVERRIDE="-allowsinglekeytrustedmirror=1"
# The unguarded unblocked-pin check (see the header). Measured on 2026-09-23:
# absent on v0.34.4, v0.34.5 and v0.34.6, where the same comparison sits behind
# `if (!trusted_signers.empty() &&`; present on v0.34.8-rc4 and v0.34.9. Its
# presence means a node holding a local signing key and no pin is refused.
SIGNER_UNPINNED_REFUSAL="if (unblocked_pin_members <"
# The app-side answer: node.rs pins a validating signer's own key.
APP_SIGNER_SELF_PIN="signing_key_self_pin"

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
  printf 'args.push("%s".to_string());\n' "$APP_SINGLE_KEY_OVERRIDE" > "$t/node_override.rs"
  grep -q -- "$APP_SINGLE_KEY_OVERRIDE" "$t/node_ungated.rs"  && { echo "self-test: override check matched a node.rs without the override"; fails=1; }
  grep -q -- "$APP_SINGLE_KEY_OVERRIDE" "$t/node_override.rs" || { echo "self-test: override check MISSED the override"; fails=1; }
  # The 0.34.6 shape (guarded) must NOT read as a refusal; the 0.34.9 shape must.
  printf '        if (!trusted_signers.empty() &&\n            unblocked_pin_members < static_cast<size_t>(trusted_threshold)) {\n' > "$t/pin_guarded.cpp"
  printf '        if (unblocked_pin_members < static_cast<size_t>(trusted_threshold)) {\n' > "$t/pin_unguarded.cpp"
  grep -qF -- "$SIGNER_UNPINNED_REFUSAL" "$t/pin_guarded.cpp"   && { echo "self-test: signer-pin check matched the 0.34.6 guarded form"; fails=1; }
  grep -qF -- "$SIGNER_UNPINNED_REFUSAL" "$t/pin_unguarded.cpp" || { echo "self-test: signer-pin check MISSED the unguarded form"; fails=1; }
  printf 'pub fn %s(conf: &Path, datadir: &Path) -> Option<String> { None }\n' "$APP_SIGNER_SELF_PIN" > "$t/node_selfpin.rs"
  grep -qF -- "$APP_SIGNER_SELF_PIN" "$t/node_ungated.rs" && { echo "self-test: self-pin check matched a node.rs without it"; fails=1; }
  grep -qF -- "$APP_SIGNER_SELF_PIN" "$t/node_selfpin.rs" || { echo "self-test: self-pin check MISSED it"; fails=1; }

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
# An explicit argument that spells the install KEY (suffixed or not) is routed
# to NODE_RELEASE_COMMIT like the bare run is, because the key may not be an
# upstream ref. Any other argument is taken literally as an upstream tag, so
# `check-engine-fleet-ready.sh v0.34.6` checks upstream's tag object, which is
# not necessarily the commit the app pins; read the "verifying ... at commit"
# line to know which one you got.
REF="$TAG"
PIN_COMMIT="$(pin_commit "$COMMANDS_RS")"
if [ -n "$PIN_COMMIT" ] && [ "$TAG" = "$(sed -n 's/^pub const NODE_RELEASE_TAG: &str = "\([^"]*\)";.*$/\1/p' "$COMMANDS_RS" | head -1)" ]; then
  REF="$PIN_COMMIT"
  echo "pin names its commit: verifying $TAG at commit $REF (NODE_RELEASE_COMMIT)"
fi
SRC_INIT="$(fetch_at_tag "$REF" "$INIT_CPP" "$INIT_FILE")" || die \
  "could not read $INIT_CPP for $TAG (ref $REF)" \
  "Tried $BTX_CLONE and $RAW_BASE/$REF/$INIT_CPP." \
  "An unverifiable tag is not a safe tag."
[ -s "$INIT_FILE" ] || die "fetched an empty $INIT_CPP for $TAG (ref $REF)" "Source was: $SRC_INIT"

MANIFEST_FILE="$WORK/manifest.data"
SRC_MANIFEST="$(fetch_at_tag "$REF" "$MANIFEST" "$MANIFEST_FILE")" || die \
  "could not read $MANIFEST for $TAG (ref $REF)" \
  "Tried $BTX_CLONE and $RAW_BASE/$REF/$MANIFEST."
[ -s "$MANIFEST_FILE" ] || die "fetched an empty golden manifest for $TAG (ref $REF)"
echo "source: $SRC_INIT"
echo

# --- 2. what does the ENGINE allow? ----------------------------------------
CONSENSUS_STARTS=0
if grep -q "$DEGRADED_START_MARKER" "$INIT_FILE"; then CONSENSUS_STARTS=1; fi
MIRROR_STARTS=1
if grep -q "$SINGLE_KEY_REFUSAL" "$INIT_FILE"; then MIRROR_STARTS=0; fi
UNPINNED_SIGNER_STARTS=1
if grep -qF -- "$SIGNER_UNPINNED_REFUSAL" "$INIT_FILE"; then UNPINNED_SIGNER_STARTS=0; fi

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
if [ "$UNPINNED_SIGNER_STARTS" -eq 1 ]; then
  echo "signer, no pin ......... accepted (its own key seeds the pin)"
else
  echo "signer, no pin ......... REFUSED at init (unblocked-pin check counts no local secp key)"
fi

# --- 3. what does the APP choose? ------------------------------------------
[ -f "$NODE_RS" ] || die "cannot find $NODE_RS" \
  "This guard has to know which mode the app selects. Point it at the new path."
APP_PINS_SINGLE_KEY=0
if grep -q -- "-matmultrustedthreshold=1" "$NODE_RS"; then APP_PINS_SINGLE_KEY=1; fi
APP_HAS_DEGRADED_GATE=0
if grep -q "$APP_DEGRADED_GATE" "$NODE_RS"; then APP_HAS_DEGRADED_GATE=1; fi
APP_PASSES_SINGLE_KEY_OVERRIDE=0
if grep -q -- "$APP_SINGLE_KEY_OVERRIDE" "$NODE_RS"; then APP_PASSES_SINGLE_KEY_OVERRIDE=1; fi
APP_SELF_PINS_SIGNER=0
if grep -qF -- "$APP_SIGNER_SELF_PIN" "$NODE_RS"; then APP_SELF_PINS_SIGNER=1; fi

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
if [ "$APP_PASSES_SINGLE_KEY_OVERRIDE" -eq 1 ]; then
  echo "app single-key override  present ($APP_SINGLE_KEY_OVERRIDE)"
else
  echo "app single-key override  ABSENT"
fi
if [ "$APP_SELF_PINS_SIGNER" -eq 1 ]; then
  echo "app signer self-pin .... present ($APP_SIGNER_SELF_PIN)"
else
  echo "app signer self-pin .... ABSENT"
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
  note "this tag allows a degraded consensus start but refuses the 1-of-1 mirror the app still selects; crates/btx-core/src/node.rs needs a $APP_DEGRADED_GATE gate so driver hosts take consensus mode and driver-less hosts take the mirror with $APP_SINGLE_KEY_OVERRIDE"
elif [ "$CONSENSUS_STARTS" -eq 1 ] && [ "$MIRROR_STARTS" -eq 0 ] \
     && [ "$APP_PINS_SINGLE_KEY" -eq 1 ] && [ "$APP_HAS_DEGRADED_GATE" -eq 1 ] \
     && [ "$APP_PASSES_SINGLE_KEY_OVERRIDE" -eq 0 ]; then
  note "this tag refuses the 1-of-1 mirror the app hands a PC with no NVIDIA driver unless $APP_SINGLE_KEY_OVERRIDE is passed, and crates/btx-core/src/node.rs no longer passes it"
fi
# Separate from the mode question above: every host that validates also signs
# (0.6.26), so a tag that refuses a key with no pin needs the app's self-pin.
if [ "$UNPINNED_SIGNER_STARTS" -eq 0 ] && [ "$APP_SELF_PINS_SIGNER" -eq 0 ]; then
  note "this tag refuses to start a node that holds a local signing key and pins no key, which is every validating host since 0.6.26, and crates/btx-core/src/node.rs does not pin the signer's own key ($APP_SIGNER_SELF_PIN)"
fi

if [ -z "$PROBLEMS" ]; then
  if [ "$CONSENSUS_STARTS" -eq 1 ]; then
    echo "OK: $TAG is fleet-startable: off-manifest hosts with an NVIDIA driver via a degraded consensus start, hosts without one via the 1-of-1 mirror behind the override."
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
