#!/usr/bin/env bash
#
# Does the engine tag we pin actually carry the withdrawn stall-recovery rule?
#
# WHAT THIS PROTECTS. The BTX Node app pins one upstream engine tag in
# apps/node/src-tauri/src/commands.rs (NODE_RELEASE_TAG). Some upstream tags
# carry a mainnet consensus rule, nMatMulStallRecoveryHeight = 199'299, that was
# introduced in v0.33.4 (commit 1c87fcd6, PR #119) and withdrawn again in
# v0.34.2 (commit 1a58e07a). pow.cpp applies that height with no version gate
# and MatMulAsert compares next_height with '==', so a node built from a tag
# that carries the number diverges from the majority chain at exactly block
# 199299. Shipping such a tag to users puts them on a dead branch that needs a
# manual rollback, not just an upgrade. This script is the machine check for
# that. Nothing else in the repo can see it.
#
# WHY IT IS NOT JUST A GREP. doc/node-release-recipe.md used to tell a human to
# run `git grep -h "consensus.nMatMulStallRecoveryHeight = " <tag>` and accept
# the tag if the output says std::numeric_limits<int32_t>::max(). That criterion
# passes a FORKED tag. chainparams.cpp assigns the field five times, once per
# network, and testnet, testnet4, signet and regtest always carry the sentinel.
# So v0.34.1 prints 199'299 first and then four max() lines, and a human
# skimming for "max()" finds it and ships the fork. The rule has to be "EVERY
# assignment is the sentinel", never "some assignment is".
#
# FAIL CLOSED, ON PURPOSE. Any assignment that is not exactly the disabled
# sentinel is treated as the red flag, wherever it appears and whatever network
# it belongs to. That can only ever raise a false alarm on a tag that moved the
# sentinel's spelling. It can never wave a forked tag through, which is the
# property a safety guard needs. Finding ZERO assignments is also a failure: a
# failed fetch, a renamed symbol or a moved file must be loud, never a silent
# pass. This repo has twice shipped breakage behind a guard that quietly stopped
# matching anything and kept exiting 0 (see the REL_CURRENT comment block in
# scripts/check-node-links.py). A guard that cannot fail is not a guard.
#
# WHAT THIS DOES **NOT** COVER, so nobody mistakes its green for full coverage.
#
#   * ONLY the mac engine pin. It reads NODE_RELEASE_TAG, which is the release
#     TAG the mac node app bundles. Windows and Linux node builds do not use a
#     tag at all: node-win-installer.yml and node-linux-installer.yml build btxd
#     from commit pin 1e51f0d1, which is v0.33.3 era and therefore predates the
#     constant entirely (v0.33.3 has no nMatMulStallRecoveryHeight assignment).
#     Those two platforms are out of scope because they are not exposed, not
#     because they were checked. If either ever moves to a tag or to a commit at
#     or after v0.33.4, bring it into this guard rather than assuming.
#   * ONLY this one constant. It says nothing about the withdrawn assumeutxo
#     bases 199299 (f12a27d0) and 199300 (ff80e629), which v0.34.2, v0.34.3 and
#     v0.34.4 all still compile, and nothing about the trusted-mirror M>=2
#     mainnet refusal that 0.34 added. Both are in docs/node-release-recipe.md
#     and both are still manual. A tag can pass this script and still be
#     unshippable for us.
#   * The mac node release is cut by hand, so nothing forces this to run on that
#     path. `npm run check:engine` is the one-liner; the recipe's checklist is
#     what makes it non-optional. CI runs it when the pin or this script changes.
#
# USAGE
#   scripts/check-engine-tag.sh              check the pin in commands.rs
#   scripts/check-engine-tag.sh v0.34.4      check any tag, ignores the
#                                            acknowledgement below
#   ENGINE_TAG_GUARD_STRICT=1 scripts/check-engine-tag.sh
#                                            check the pin and ignore the
#                                            acknowledgement too
#
# ---------------------------------------------------------------------------
# THE ACKNOWLEDGED PIN. Read this before changing it.
#
# The tag we ship TODAY, v0.33.4.1, is forked. That is a real, known,
# unresolved problem and this guard reports it on every run. It does not fail
# the build for it, because failing would turn CI red on main and block every
# unrelated PR for a state that no unrelated PR caused or can fix.
#
# The tradeoff, stated plainly. The honest alternative is to let the guard go
# red and leave it red. We chose not to, and the cost of that choice is that a
# forked pin can sit here indefinitely while CI stays green. The line below is
# the whole mitigation: it names exactly ONE tag, it is checked against the
# actual pin, and it makes the fork impossible to ship silently. Move the pin to
# any other still-forked tag and this guard fails, because the new tag will not
# match this line. Clearing this line makes the guard fail on the current pin.
# Editing it is a deliberate act a human has to perform and a reviewer will see
# in the diff.
#
# When the pin moves to a clean engine, empty this to "" and the guard becomes
# an unconditional gate again. Do not add tags to it. It holds one.
ACKNOWLEDGED_FORKED_TAG=""
# ---------------------------------------------------------------------------

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMMANDS_RS="$ROOT/apps/node/src-tauri/src/commands.rs"
CHAINPARAMS="src/kernel/chainparams.cpp"
BTX_CLONE="${BTX_CLONE:-/Users/bonuz/repos/btx}"
RAW_BASE="https://raw.githubusercontent.com/btxchain/btx"

# An UNTAGGED pin. When commands.rs also declares NODE_RELEASE_COMMIT, the tag
# string may not exist upstream yet (0.34.6 shipped from release/0.34.6 before
# upstream tagged it). The guard then fetches source at that SHA instead, so an
# untagged pin gets exactly the check a tagged one gets. It never substitutes a
# branch name: a branch moves, a SHA does not. Empty when the tag is real.
pin_commit() {
  sed -n 's/^pub const NODE_RELEASE_COMMIT: &str = "\([0-9a-f]\{40\}\)";.*$/\1/p' "$1" | head -1
}
SENTINEL='std::numeric_limits<int32_t>::max()'
# One assignment per network: mainnet, testnet, testnet4, signet, regtest.
# Measured as 5 on v0.33.4, v0.33.4.1, v0.33.4.2, v0.34, v0.34.1, v0.34.2,
# v0.34.3 and v0.34.4. Move this only after reading the file by hand.
EXPECTED_ASSIGNMENTS=5

# GitHub Actions renders ::error:: as an annotation. Locally it is just noise,
# so only emit it in CI.
annotate() {
  if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::$1::$2"; fi
}
die() {
  echo "FAIL: $1" >&2
  annotate error "engine tag guard: $1"
  shift
  for line in "$@"; do echo "  $line" >&2; done
  exit 1
}

# --- 1. which tag ----------------------------------------------------------
OVERRIDE_TAG="${1:-}"
if [ -n "$OVERRIDE_TAG" ]; then
  TAG="$OVERRIDE_TAG"
  echo "checking tag $TAG (explicit argument, the pin in commands.rs is ignored)"
else
  [ -f "$COMMANDS_RS" ] || die "cannot find $COMMANDS_RS" \
    "The app moved. Point this guard at the new path rather than skipping it."

  # Deliberately strict. If the const is reshaped, we want a loud failure here,
  # not a lenient pattern that keeps matching something almost right.
  TAG="$(sed -n 's/^pub const NODE_RELEASE_TAG: &str = "\([^"]*\)";.*$/\1/p' "$COMMANDS_RS" | head -1)"
  if [ -z "$TAG" ]; then
    die "could not read NODE_RELEASE_TAG from $COMMANDS_RS" \
      "Expected a line of the exact shape:" \
      '  pub const NODE_RELEASE_TAG: &str = "v0.33.4.1";' \
      "Its shape changed. FIX THIS GUARD, do not delete it and do not skip it." \
      "A guard that stops matching and exits 0 is how this repo has shipped" \
      "broken pins before (scripts/check-node-links.py, REL_CURRENT, 3 blind days)."
  fi
  echo "pinned engine tag: $TAG (from apps/node/src-tauri/src/commands.rs)"
fi

# --- 2. fetch chainparams.cpp for that tag ---------------------------------
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FILE="$WORK/chainparams.cpp"
SOURCE=""

# The ref we fetch. Normally the tag itself; for an untagged pin (see
# pin_commit above) the commit the tag name stands for. An explicit override
# argument is always taken literally, so `check-engine-tag.sh v0.34.7` still
# means "that upstream tag" and nothing else.
REF="$TAG"
if [ -z "$OVERRIDE_TAG" ]; then
  PIN_COMMIT="$(pin_commit "$COMMANDS_RS")"
  if [ -n "$PIN_COMMIT" ]; then
    REF="$PIN_COMMIT"
    echo "untagged pin: verifying $TAG at commit $REF"
  fi
fi

if [ -d "$BTX_CLONE/.git" ] \
   && git -C "$BTX_CLONE" rev-parse --verify --quiet "$REF^{commit}" >/dev/null 2>&1 \
   && git -C "$BTX_CLONE" show "$REF:$CHAINPARAMS" > "$FILE" 2>/dev/null; then
  SOURCE="local clone $BTX_CLONE"
else
  # No clone, or the ref is not in it (a CI runner, or a tag fetched after the
  # clone was last updated). curl --max-time, NOT timeout: timeout does not
  # exist on macOS and this script runs on both.
  URL="$RAW_BASE/$REF/$CHAINPARAMS"
  if curl -fsSL --max-time 60 "$URL" -o "$FILE" 2>/dev/null; then
    SOURCE="$URL"
  else
    die "could not read $CHAINPARAMS for tag $TAG" \
      "Tried the local clone at $BTX_CLONE and $URL." \
      "Either the tag does not exist upstream or the fetch failed." \
      "This is a hard failure by design: an unverifiable tag is not a safe tag."
  fi
fi
[ -s "$FILE" ] || die "fetched an empty $CHAINPARAMS for tag $TAG" \
  "Source was: $SOURCE"
echo "source: $SOURCE"

# --- 3. every assignment, comment TEXT removed -----------------------------
# chainparams.cpp also has explanatory comment lines that name the symbol, so
# comments have to go before matching. Strip the comment text, never the whole
# line. Dropping a line because it happens to START with a comment marker is a
# hole big enough to ship a fork through: this line
#
#     /* mainnet */ consensus.nMatMulStallRecoveryHeight = 199'299;
#
# is a real forked assignment, and dropping it takes the assignment count
# silently from five to four and reports OK. Removing "//" to end of line and
# single-line "/* ... */" leaves the code on such a line intact.
#
# A comment that SPANS lines is deliberately not tracked. A forked-looking line
# inside one is therefore read as code and flagged. That is a false alarm, and
# a false alarm is the only direction this guard is allowed to be wrong in.
CODE="$(sed -e 's:/\*[^*]*\*/: :g' -e 's://.*$::' "$FILE")"

ASSIGNMENTS="$(
  printf '%s\n' "$CODE" \
    | grep -E 'nMatMulStallRecoveryHeight[[:space:]]*=' \
    | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' \
    || true
)"

# Zero is also what you get from a tag that PREDATES the field entirely, such as
# v0.33.3. Failing there is correct: the guard cannot certify what it cannot
# read, and "the number is absent" and "the file is absent" look identical from
# here. Verify such a tag by hand and say so in the commit.
if [ -z "$ASSIGNMENTS" ]; then
  die "found ZERO assignments of nMatMulStallRecoveryHeight in $CHAINPARAMS at $TAG" \
    "Source was: $SOURCE" \
    "A tag that carries the fork assigns this five times, once per network." \
    "Zero means the fetch returned the wrong thing, the symbol was renamed, or" \
    "the file moved. It does NOT mean the tag is clean, so this is a failure." \
    "Fix the guard to match the new shape before shipping anything."
fi

TOTAL="$(printf '%s\n' "$ASSIGNMENTS" | wc -l | tr -d ' ')"

# Every tag that has this field at all assigns it once per network, and that has
# been five on every tag from v0.33.4 to v0.34.4. A count that is not five means
# the file changed shape under us. Refuse to render a verdict on a shape this
# guard was not written against, because a shape change is exactly how an
# assignment goes missing and a fork reads as clean.
if [ "$TOTAL" != "$EXPECTED_ASSIGNMENTS" ]; then
  die "$TAG assigns nMatMulStallRecoveryHeight $TOTAL times, expected $EXPECTED_ASSIGNMENTS" \
    "Source was: $SOURCE" \
    "One assignment per network, and that has been $EXPECTED_ASSIGNMENTS on every tag this guard" \
    "was written against. A different count means upstream added or removed a" \
    "network, or a line stopped matching. Read chainparams.cpp for this tag by" \
    "hand, confirm what mainnet assigns, then update EXPECTED_ASSIGNMENTS at the" \
    "top of this script in the same commit and say why."
fi

# Compare the assigned VALUE, not the line. A substring test on the whole line
# passes this, because the sentinel is right there in the comment:
#
#     consensus.nMatMulStallRecoveryHeight = 199'299;  // was ...::max()
#
# So take what sits between the first '=' and the terminating ';' and require it
# to be the sentinel exactly. An assignment whose line carries no ';' has its
# value wrapped onto the next line, where this cannot read it, so it counts as
# bad rather than as fine.
BAD=""
while IFS= read -r line; do
  [ -n "$line" ] || continue
  case "$line" in
    *\;*) value="${line#*=}"; value="${value%%;*}" ;;
    *)    value="(no ; on this line, the value is unreadable here)" ;;
  esac
  value="$(printf '%s' "$value" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"
  if [ "$value" != "$SENTINEL" ]; then
    BAD="${BAD}${line}"$'\n'
  fi
done <<EOF
$ASSIGNMENTS
EOF
BAD="${BAD%$'\n'}"

# --- 4. verdict ------------------------------------------------------------
if [ -z "$BAD" ]; then
  echo "OK: $TAG assigns nMatMulStallRecoveryHeight $TOTAL times, every one of them"
  echo "    the disabled sentinel $SENTINEL."
  echo "    No mainnet stall-recovery height. This engine follows the majority chain."
  if [ -z "$OVERRIDE_TAG" ] && [ -n "$ACKNOWLEDGED_FORKED_TAG" ] && [ "$TAG" = "$ACKNOWLEDGED_FORKED_TAG" ]; then
    echo
    echo "note: ACKNOWLEDGED_FORKED_TAG still names $TAG, which now checks out clean."
    echo "      Empty that line in this script so the guard is unconditional again."
  fi
  exit 0
fi

BAD_COUNT="$(printf '%s\n' "$BAD" | wc -l | tr -d ' ')"

echo "$TAG assigns nMatMulStallRecoveryHeight $TOTAL times."
echo "These are NOT the disabled sentinel ($BAD_COUNT of $TOTAL):"
printf '%s\n' "$BAD" | sed 's/^/    /'
echo
echo "A node built from this tag activates the withdrawn stall-recovery rule."
echo "pow.cpp applies it with no version gate and MatMulAsert compares with '==',"
echo "so it diverges from the majority chain at exactly block 199299."
echo "The rule was introduced in v0.33.4 (1c87fcd6, PR #119) and withdrawn in"
echo "v0.34.2 (1a58e07a). Tags v0.33.4 through v0.34.1 carry it."
echo

if [ -z "$OVERRIDE_TAG" ] && [ -n "$ACKNOWLEDGED_FORKED_TAG" ] && [ "$TAG" = "$ACKNOWLEDGED_FORKED_TAG" ] && [ -z "${ENGINE_TAG_GUARD_STRICT:-}" ]; then
  echo "NOT FAILING: $TAG is the tag named in ACKNOWLEDGED_FORKED_TAG at the top of"
  echo "this script. The pin is knowingly forked and this build is knowingly shipping"
  echo "it. Moving the pin to any other forked tag WILL fail this guard."
  echo "Run with ENGINE_TAG_GUARD_STRICT=1 to fail on the acknowledged pin too."
  annotate warning "engine tag guard: pinned engine $TAG is forked (acknowledged). It diverges from the majority chain at block 199299."
  exit 0
fi

if [ -n "$ACKNOWLEDGED_FORKED_TAG" ] && [ "$TAG" != "$ACKNOWLEDGED_FORKED_TAG" ] && [ -z "$OVERRIDE_TAG" ]; then
  echo "The acknowledgement in this script names $ACKNOWLEDGED_FORKED_TAG, not $TAG."
  echo "Pin a clean engine instead. If you truly mean to ship this forked tag,"
  echo "change ACKNOWLEDGED_FORKED_TAG by hand and say why in the commit message."
  echo
fi

die "engine tag $TAG carries the withdrawn mainnet stall-recovery height"
