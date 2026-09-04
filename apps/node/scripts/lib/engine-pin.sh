#!/usr/bin/env bash
# The engine pin, in one place, checked rather than remembered.
#
# WHY THIS FILE EXISTS. There are five stage-node-pkg*.sh scripts and each one
# carried its own hardcoded VERSION. `NODE_RELEASE_TAG` in commands.rs is what
# decides the install directory, and node.rs derives the version-gated btxd
# flags from that directory name. So the two are load-bearing on each other and
# nothing compared them.
#
# They drifted. The mac and linux scripts sat on 0.33.2 while the pin moved to
# v0.34.5. The result on mac was that the documented contributor build could not
# complete first-run setup: `provision_node_package` derives the expected
# version from the install directory, sees v0.34.5, asks the staged binary, gets
# v0.33.2, and refuses the package. Nobody noticed because nothing checked.
#
# If you fork this, keep this guard. The failure it prevents is silent until it
# is expensive: a build that looks fine and installs an engine the app then
# refuses, or worse, one it accepts and drives with flags that engine does not
# have.

# Read NODE_RELEASE_TAG out of commands.rs. Prints e.g. `v0.34.5`.
engine_pin_tag() {
  local app_dir="$1"
  local src="$app_dir/src-tauri/src/commands.rs"
  [[ -f "$src" ]] || { echo "error: no commands.rs at $src" >&2; return 1; }
  local tag
  tag="$(grep -oE 'NODE_RELEASE_TAG: &str = "v[0-9]+(\.[0-9]+)+"' "$src" \
         | grep -oE 'v[0-9]+(\.[0-9]+)+' | head -1)"
  [[ -n "$tag" ]] || { echo "error: could not read NODE_RELEASE_TAG from $src" >&2; return 1; }
  printf '%s\n' "$tag"
}

# Fail unless this script's VERSION matches the pin. $1 = app dir, $2 = VERSION
# (without the leading v).
assert_matches_engine_pin() {
  local app_dir="$1" version="$2" tag
  tag="$(engine_pin_tag "$app_dir")" || return 1
  if [[ "$tag" != "v$version" ]]; then
    cat >&2 <<EOF
error: this staging script and the engine pin disagree.

    this script stages     v$version
    NODE_RELEASE_TAG is    $tag

  The app installs the staged package into a directory named after
  NODE_RELEASE_TAG and then verifies the binary reports that version, so
  staging v$version here produces a build that fails first-run setup with
  "staged node package is not $tag".

  Fix ONE of these, deliberately:
    - bump VERSION (and TARBALL_SHA256, from the release's signed SHA256SUMS)
      in this script to match $tag, or
    - change NODE_RELEASE_TAG in apps/node/src-tauri/src/commands.rs, which is
      CODEOWNERS protected and decides which btxd every user runs.
EOF
    return 1
  fi
}
