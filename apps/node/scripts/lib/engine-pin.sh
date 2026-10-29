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

# Read NODE_RELEASE_TAG out of commands.rs: the app's INSTALL KEY. Prints e.g.
# `v0.34.5`, or a suffixed key like `v0.34.6-3013c2c2`.
#
# The key is `<version btxd reports>[-<qualifier>]`. The qualifier exists so the
# key can move when the engine moves but the reported version does not
# (2026-09-15: upstream tagged v0.34.6 one commit past the 9eb4e005 build every
# install already held under the key `v0.34.6`, and the app re-provisions ONLY
# when this key changes). The pattern is deliberately no looser than that shape:
# a reshaped constant must fail here loudly, not match something almost right.
engine_pin_tag() {
  local app_dir="$1"
  local src="$app_dir/src-tauri/src/commands.rs"
  [[ -f "$src" ]] || { echo "error: no commands.rs at $src" >&2; return 1; }
  local tag
  tag="$(grep -oE 'NODE_RELEASE_TAG: &str = "v[0-9]+(\.[0-9]+)+(-[0-9A-Za-z._]+)?"' "$src" \
         | grep -oE 'v[0-9]+(\.[0-9]+)+(-[0-9A-Za-z._]+)?' | head -1)"
  [[ -n "$tag" ]] || { echo "error: could not read NODE_RELEASE_TAG from $src" >&2; return 1; }
  printf '%s\n' "$tag"
}

# The version btxd is expected to REPORT for the pinned engine: the install key
# with any `-<qualifier>` removed. `v0.34.6-3013c2c2` -> `v0.34.6`; a bare key
# prints unchanged. This is what the staging scripts compare against
# `btxd --version` and what ends up in the package's `.btxd-version` marker,
# which is what provisioning verifies the binary against (installer.rs,
# BTXD_VERSION_MARKER). The install key itself never reaches the binary check:
# btxd knows nothing about our qualifier.
#
# Stripping the suffix is a convention, not a proof, and the -source staging
# scripts close the gap by asking the real binary before writing the marker.
# It was violated exactly once, before this file existed: `v0.33.3-pr105b`
# carried a binary that reported v0.33.2. Do not do that again; if the version
# part of the key is not what btxd prints, the scripts refuse the build.
engine_pin_version() {
  local tag
  tag="$(engine_pin_tag "$1")" || return 1
  printf '%s\n' "${tag%%-*}"
}

# Read NODE_RELEASE_COMMIT out of commands.rs: the exact upstream commit the
# install key stands for. Set while upstream had not tagged it (0.6.18 shipped
# 0.34.6 from release/0.34.6 this way) and kept set once it had, because a
# suffixed key is not an upstream ref and nothing can fetch it by name. Prints
# the 40-hex SHA, or nothing when the key is itself a real upstream tag.
engine_pin_commit() {
  local app_dir="$1"
  local src="$app_dir/src-tauri/src/commands.rs"
  [[ -f "$src" ]] || { echo "error: no commands.rs at $src" >&2; return 1; }
  grep -oE 'NODE_RELEASE_COMMIT: &str = "[0-9a-f]{40}"' "$src" \
    | grep -oE '[0-9a-f]{40}' | head -1
  return 0
}
# The ref to CHECK OUT to build the pinned engine: the commit when there is
# one, else the tag. A branch name is never substituted; a branch moves and a
# SHA does not. This is what CI's engine build and any worktree should use;
# engine_pin_tag stays the NAME (install directory, version string, guards).
engine_pin_ref() {
  local app_dir="$1" commit
  commit="$(engine_pin_commit "$app_dir")" || return 1
  if [[ -n "$commit" ]]; then printf '%s\n' "$commit"; else engine_pin_tag "$app_dir"; fi
}
# Fail unless this script's VERSION matches the pin. $1 = app dir, $2 = VERSION
# (without the leading v).
#
# Compared against engine_pin_version, the key minus its qualifier, NOT against
# the key itself. The version a script stages is the version the binary prints,
# and the binary never prints our qualifier; comparing against the full key
# would make every -source script refuse the exact engine the pin names. This
# does not weaken the guard: the key's version part is pinned in the same
# constant, the -source scripts then confirm it against the real `btxd
# --version` before writing `.btxd-version`, and provisioning verifies the
# installed binary against that marker on the user's machine.
assert_matches_engine_pin() {
  local app_dir="$1" version="$2" tag expect
  tag="$(engine_pin_tag "$app_dir")" || return 1
  expect="${tag%%-*}"
  if [[ "$expect" != "v$version" ]]; then
    cat >&2 <<EOF
error: this staging script and the engine pin disagree.

    this script stages     v$version
    NODE_RELEASE_TAG is    $tag  (btxd must report $expect)

  The app installs the staged package into a directory named after
  NODE_RELEASE_TAG and then verifies the binary reports the version the
  package declares in .btxd-version, so staging v$version here produces a
  build that fails first-run setup with "staged node package is not $expect".

  Fix ONE of these, deliberately:
    - bump VERSION (and TARBALL_SHA256, from the release's signed SHA256SUMS)
      in this script to match $expect, or
    - change NODE_RELEASE_TAG in apps/node/src-tauri/src/commands.rs, which is
      CODEOWNERS protected and decides which btxd every user runs.
EOF
    return 1
  fi
}
