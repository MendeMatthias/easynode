#!/usr/bin/env bash
# Build a Caddy that understands this directory's Caddyfile.
#
# Caddyfile.template uses `rate_limit`, which is NOT in stock Caddy: it is the
# github.com/mholt/caddy-ratelimit plugin, and the deployment this was ported
# from was built with it (its runbook mentions the fact once, under
# "Hardening, 2026-07-17"). A stock binary refuses the whole configuration:
#
#     $ caddy validate --config Caddyfile --adapter caddyfile
#     Error: adapting config using caddyfile: ... unrecognized directive: rate_limit
#
# Usage:
#   deploy/esplora/build-caddy.sh                      # -> /usr/local/bin/caddy (sudo if needed)
#   PREFIX=$HOME/.local deploy/esplora/build-caddy.sh  # no root
#
# Needs Go (https://go.dev/dl) and xcaddy:
#   go install github.com/caddyserver/xcaddy/cmd/xcaddy@latest
# The official download page (https://caddyserver.com/download) builds the same
# binary if you tick "mholt/caddy-ratelimit". Either way, the check at the end
# is what matters: the module list must carry http.handlers.rate_limit.
set -euo pipefail
PREFIX="${PREFIX:-/usr/local}"
export PATH="$HOME/go/bin:$PATH"

command -v xcaddy >/dev/null 2>&1 || {
  echo "xcaddy not found. Install Go, then: go install github.com/caddyserver/xcaddy/cmd/xcaddy@latest" >&2
  exit 1
}

out="$(mktemp -d)/caddy"
xcaddy build --with github.com/mholt/caddy-ratelimit --output "$out"

"$out" list-modules 2>/dev/null | grep -q '^http.handlers.rate_limit$' || {
  echo "the built caddy does not list http.handlers.rate_limit; refusing to install it" >&2
  exit 1
}

mkdir -p "$PREFIX/bin" 2>/dev/null || true
if [ -w "$PREFIX/bin" ]; then
  install -m755 "$out" "$PREFIX/bin/caddy"
else
  echo "installing to $PREFIX/bin needs root"
  sudo install -m755 "$out" "$PREFIX/bin/caddy"
fi
echo "installed $PREFIX/bin/caddy ($("$PREFIX/bin/caddy" version))"

HERE="$(cd "$(dirname "$0")" && pwd)"
echo "validate the template with:"
echo "  BTX_ESPLORA_HOST=http://127.0.0.1:3080 $PREFIX/bin/caddy validate --config $HERE/Caddyfile.template --adapter caddyfile"
