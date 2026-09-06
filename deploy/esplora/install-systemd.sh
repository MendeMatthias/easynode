#!/usr/bin/env bash
# Install the Esplora stack as systemd units, with the precondition checked
# FIRST. This is the server path: a machine that serves Esplora without the
# easyNode desktop app, which is the shape esplora-1.easybtx.com will run.
#
# The README used to say "fill in the templates and install them", which is
# four files, two placeholder substitutions, a binary path, a marker directory
# and an enable. Every one of them is a chance to install a unit that starts
# and then fails in a way nobody reads.
#
# WHAT IT REFUSES, AND WHY IT REFUSES IT FIRST. electrs indexes from btxd's
# block files and a pruned datadir deletes them, so the index can never be
# built and cannot be completed later without a full resync. btxd's own answer
# is authoritative and this asks it before touching anything: a datadir's
# btx_rw.conf outranks its conf file, so a conf that says prune=0 proves
# nothing (measured on the release box 2026-09-04, where the conf said 0 and
# the node had been pruning at 4096 for weeks). electrs would otherwise
# discover this hours into an index it was never going to finish.
#
#   deploy/esplora/install-systemd.sh --host esplora-1.example.com [options]
#
# Options:
#   --host <name>        the public hostname Caddy serves and gets a certificate for
#   --user <name>        the service user (default: the invoking user)
#   --datadir <path>     btxd's datadir, holding blocks/ and .cookie (default /var/lib/btx)
#   --db-dir <path>      where electrs keeps its index (default /var/lib/electrs-db)
#   --rpc <addr:port>    btxd's JSON-RPC (default 127.0.0.1:19334)
#   --yes                actually do it; without this the plan is printed and nothing changes
#
# It prints exactly what it would write and changes nothing until --yes.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
HOST=""
SVC_USER="${SUDO_USER:-$(id -un)}"
DATADIR=/var/lib/btx
DB_DIR=/var/lib/electrs-db
RPC=127.0.0.1:19334
APPLY=0

while [ $# -gt 0 ]; do
  case "$1" in
    --host)    HOST="${2:?--host needs a value}"; shift 2 ;;
    --user)    SVC_USER="${2:?--user needs a value}"; shift 2 ;;
    --datadir) DATADIR="${2:?--datadir needs a value}"; shift 2 ;;
    --db-dir)  DB_DIR="${2:?--db-dir needs a value}"; shift 2 ;;
    --rpc)     RPC="${2:?--rpc needs a value}"; shift 2 ;;
    --yes)     APPLY=1; shift ;;
    -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

[ -n "$HOST" ] || { echo "--host is required (the name Caddy serves and gets a certificate for)" >&2; exit 2; }
case "$HOST" in
  *[!a-zA-Z0-9.-]*) echo "--host must be a hostname, not a URL or an expression: $HOST" >&2; exit 2 ;;
  *.*) : ;;
  *) echo "--host needs a dot: a full hostname, not '$HOST'" >&2; exit 2 ;;
esac

say() { printf '  %s\n' "$*"; }
run() { if [ "$APPLY" -eq 1 ]; then sudo "$@"; else printf '    would run: sudo %s\n' "$*"; fi; }

echo "── the precondition, asked of the node itself ──"
# btx-cli is not necessarily on PATH for a service user; try the usual places.
BCLI="${BTX_CLI:-}"
if [ -z "$BCLI" ]; then
  for c in btx-cli /usr/local/bin/btx-cli "$HOME/.local/bin/btx-cli"; do
    command -v "$c" >/dev/null 2>&1 && { BCLI="$c"; break; }
  done
fi
if [ -z "$BCLI" ]; then
  echo "ABORT: no btx-cli found. Set BTX_CLI=/path/to/btx-cli." >&2
  exit 3
fi
info="$("$BCLI" -datadir="$DATADIR" getblockchaininfo 2>/dev/null || true)"
if [ -z "$info" ]; then
  echo "ABORT: btxd at $DATADIR did not answer getblockchaininfo." >&2
  echo "       Start it and try again. This script refuses to guess a datadir's" >&2
  echo "       prune posture from its config file: the datadir's own btx_rw.conf" >&2
  echo "       outranks that file, and a node has run pruned against a conf that" >&2
  echo "       said prune=0 for weeks." >&2
  exit 3
fi
pruned="$(printf '%s' "$info" | python3 -c 'import json,sys; d=json.load(sys.stdin); print("yes" if d.get("pruned") else "no", d.get("pruneheight") or 0, d.get("blocks") or 0)')"
set -- $pruned
if [ "$1" = "yes" ]; then
  echo "ABORT: this datadir is pruned (history below block $2 is gone)." >&2
  echo "       electrs indexes from the block files on disk and the ones it needs" >&2
  echo "       are not there. Setting prune=0 now is not enough, because nothing" >&2
  echo "       re-downloads history that was discarded: serving Esplora from this" >&2
  echo "       datadir requires a full resync with prune=0 from the start." >&2
  exit 3
fi
say "btxd is unpruned and at block $3. electrs can index it."

echo
echo "── the binaries ──"
missing=0
for b in electrs caddy; do
  path="$(command -v "$b" || true)"
  [ -z "$path" ] && [ -x "/usr/local/bin/$b" ] && path="/usr/local/bin/$b"
  [ -z "$path" ] && [ -x "$HOME/.local/bin/$b" ] && path="$HOME/.local/bin/$b"
  if [ -z "$path" ]; then
    say "$b: NOT FOUND — build it with deploy/esplora/build-$b.sh"
    missing=1
  else
    say "$b: $path"
  fi
done
if [ "$missing" -eq 1 ]; then
  echo "ABORT: build the missing binaries first; this script installs units, it does not fetch code." >&2
  exit 3
fi
if ! caddy list-modules 2>/dev/null | grep -qx 'http.handlers.rate_limit'; then
  echo "ABORT: this caddy has no rate_limit module, and the Caddyfile needs it." >&2
  echo "       deploy/esplora/build-caddy.sh builds one that does." >&2
  exit 3
fi
say "caddy carries the rate-limit module"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

sed -e "s|^User=USER$|User=$SVC_USER|" \
    -e "s|--daemon-dir /var/lib/btx|--daemon-dir $DATADIR|" \
    -e "s|--daemon-rpc-addr 127.0.0.1:19334|--daemon-rpc-addr $RPC|" \
    -e "s|--db-dir /var/lib/electrs-db|--db-dir $DB_DIR|" \
    "$HERE/electrs.service.template" > "$WORK/electrs.service"

# The front runs as its own unit reading this directory's Caddyfile, with the
# two placeholders supplied as environment. The Caddyfile is NOT copied: it is
# read from the repository so an update to it is a git pull, not a re-install.
cat > "$WORK/btx-esplora-front.service" <<UNIT
[Unit]
Description=easyNode Esplora front (Caddy: TLS, CORS, freshness headers)
After=network-online.target electrs.service
Wants=network-online.target

[Service]
User=$SVC_USER
Type=simple
Environment=BTX_ESPLORA_HOST=$HOST
Environment=BTX_ESPLORA_RUN=/run
Environment=BTX_ESPLORA_ELECTRS=127.0.0.1:3000
Environment=BTX_ESPLORA_BTXD_RPC=$RPC
# Ports 80 and 443 must reach this machine or Caddy cannot get a certificate.
AmbientCapabilities=CAP_NET_BIND_SERVICE
ExecStart=$(command -v caddy) run --config $HERE/Caddyfile.template --adapter caddyfile
ExecReload=$(command -v caddy) reload --config $HERE/Caddyfile.template --adapter caddyfile
Restart=on-failure
RestartSec=10

[Install]
WantedBy=multi-user.target
UNIT

echo
echo "── what will be installed ──"
say "electrs.service              user $SVC_USER, datadir $DATADIR, index $DB_DIR"
say "btx-esplora-front.service    host $HOST, config $HERE/Caddyfile.template"
say "btx-staleness.service/.timer the freshness guardian, every 30s"
say "/usr/local/bin/btx-staleness-check.sh"
say "markers in /run (tmpfs; the guardian recreates them)"

echo
echo "── install ──"
run install -m 0755 "$HERE/btx-staleness-check.sh" /usr/local/bin/btx-staleness-check.sh
run install -m 0644 "$WORK/electrs.service" /etc/systemd/system/electrs.service
run install -m 0644 "$WORK/btx-esplora-front.service" /etc/systemd/system/btx-esplora-front.service
run install -m 0644 "$HERE/btx-staleness.service" /etc/systemd/system/btx-staleness.service
run install -m 0644 "$HERE/btx-staleness.timer" /etc/systemd/system/btx-staleness.timer
run install -d -o "$SVC_USER" -g "$SVC_USER" "$DB_DIR"
run systemctl daemon-reload
# The guardian starts FIRST and on its own: until it has written a marker the
# front answers `unverified`, which is the honest state and the one this
# deployment wants on the way up rather than on the way to a wrong claim.
run systemctl enable --now btx-staleness.timer
run systemctl enable --now electrs.service
run systemctl enable --now btx-esplora-front.service

echo
if [ "$APPLY" -ne 1 ]; then
  echo "Nothing was changed. Re-run with --yes to apply."
  exit 0
fi
cat <<NEXT
Installed. Then, in this order:

  1. journalctl -fu electrs        — the first index takes hours on a full chain
  2. curl -sI https://$HOST/blocks/tip/height | grep -i x-btx-freshness
     It says 'unverified' until the guardian has judged it against the census.
  3. scripts/verify-esplora.sh https://$HOST <mainnet-address-with-spend-history>
     Do not advertise this endpoint to anyone until that passes, against a
     reference that is NOT this host.
NEXT
