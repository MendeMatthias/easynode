#!/usr/bin/env bash
# Refresh the vendored electrs, rust-btx and test-vectors trees from a
# btx-esplora checkout. Bytes come from `git archive`, so the copy matches the
# repository objects rather than a working tree with local edits or CRLF.
#
# Usage: deploy/esplora/sync-from-btx-esplora.sh /path/to/btx-esplora
#
# Only the three vendored trees are replaced. The hand-ported files beside
# them (Caddyfile.template, the units, the guardian, the scripts) are not
# touched; PROVENANCE.md says what each of them changed and why.
set -euo pipefail
SRC="${1:?usage: sync-from-btx-esplora.sh <path-to-btx-esplora-checkout>}"
HERE="$(cd "$(dirname "$0")" && pwd)"

commit="$(git -C "$SRC" rev-parse HEAD)"
for d in electrs rust-btx test-vectors; do
  rm -rf "${HERE:?}/$d"
done
git -C "$SRC" archive --format=tar HEAD electrs rust-btx test-vectors \
  | tar -x -C "$HERE" --exclude='electrs/contrib/popular-scripts.txt'

# Record the commit in PROVENANCE.md. Portable on purpose: no `sed -i`, whose
# in-place flag differs between GNU and BSD.
prov="$HERE/PROVENANCE.md"
tmp="$(mktemp)"
awk -v c="$commit" -v d="$(date -u +%F)" '
  /^Vendored commit: / { print "Vendored commit: " c; next }
  /^Vendored on: /     { print "Vendored on: " d ", via `git archive` (bytes from the repository objects, not a checkout)"; next }
  { print }' "$prov" > "$tmp" && mv "$tmp" "$prov"

echo "vendored btx-esplora@$commit into $HERE"
echo "review with: git status --short $HERE"
