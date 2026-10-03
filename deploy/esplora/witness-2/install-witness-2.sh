#!/usr/bin/env bash
# Install witness-2 on the api.btxscan.io box: btx-witness reading btxd2,
# served by the box's Caddy at https://api.btxscan.io/witness/.
#
# Run ON THE BOX, as root, once, with two arguments:
#
#   install-witness-2.sh <binary-url> <expected-sha256>
#
# <binary-url> is https:// (or file:// for a binary already on the box).
# What it does, in order, and it stops at the first thing that fails:
#
#   1. downloads the binary and refuses it unless its sha256 is the one given
#   2. installs it as /usr/local/bin/btx-witness
#   3. writes /etc/systemd/system/btx-witness-2.service, enables and starts it
#   4. waits for 127.0.0.1:3081/blocks/tip/height to answer a height
#   5. edits /etc/caddy/Caddyfile in place (timestamped backup first): one
#      `handle /witness/*` block inside the api.btxscan.io site. Skipped when
#      the route is already there.
#   6. caddy validate, systemctl reload caddy, and asks the public name
#      through the local Caddy for /witness/blocks/tip/height
#
# On ANY failure after something changed, it puts things back: the Caddyfile
# from the backup and a reload, the previous binary, and the unit stopped,
# disabled and removed if this run created it. It prints a short verdict either
# way. Running it again after success changes nothing but the binary.
#
# Undo by hand: see README.md next to this file.

# az vm run-command may hand this file to sh. Everything below is bash.
if [ -z "${BASH_VERSION:-}" ]; then exec bash "$0" "$@"; fi

set -uo pipefail

BIN=/usr/local/bin/btx-witness
UNIT_NAME=btx-witness-2.service
UNIT_PATH=/etc/systemd/system/$UNIT_NAME
CADDYFILE=/etc/caddy/Caddyfile
WITNESS_LOCAL=127.0.0.1:3081
SITE=api.btxscan.io

# The unit, exactly as deploy/esplora/witness-2/btx-witness-2.service. It is
# written out here because the box gets this one file and nothing else;
# test-install-witness-2.sh fails if the two ever differ.
unit_text() {
  cat <<'UNIT'
[Unit]
Description=BTX fork witness 2 (serves /blocks/tip/height, /block-height/<h> and /signers/recent from btxd2)
After=network-online.target btxd2.service
Wants=network-online.target

[Service]
# deploy/esplora/btx-witness.service.template, filled in for the api.btxscan.io
# box: the node it reads is btxd2 (datadir /data/btx2, JSON-RPC on
# 127.0.0.1:8434, cookie auth). install-witness-2.sh writes this exact text;
# a test keeps the two identical.
User=azureuser
Type=simple
# Loopback only. Caddy on the same box serves it as
# https://api.btxscan.io/witness/ with TLS and the per-IP rate limit.
ExecStart=/usr/local/bin/btx-witness --datadir /data/btx2 --rpc 127.0.0.1:8434 --listen 127.0.0.1:3081
# always, not on-failure: a witness that has exited for any reason has left
# the census without its second source. It also exits at start when btxd2 does
# not answer, and five seconds brings it back once btxd2 is up.
Restart=always
RestartSec=5

# It reads btxd2's .cookie and answers three GETs. It needs nothing else.
# The cookie is /data/btx2/.cookie, 0600 azureuser: readable by User= above.
# ProtectSystem=strict makes /data read-only for this process, which is all it
# needs, and ProtectHome does not reach /data at all. The witness re-reads the
# cookie after a 401, so a btxd2 restart (new cookie) needs no restart here.
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=read-only
ReadOnlyPaths=/data/btx2
ProtectKernelTunables=true
ProtectControlGroups=true
RestrictAddressFamilies=AF_INET AF_INET6
MemoryDenyWriteExecute=true

[Install]
WantedBy=multi-user.target
UNIT
}

# The block that goes inside the api.btxscan.io site.
#
# A plain `handle` with ONE path matcher, not `handle_path`. Caddy sorts
# blocks of the same directive by their matcher: a single path matcher first,
# then other matchers in the order written, then none. Every handle imported
# from (esplora_api) uses a named matcher (@options, @rawblock, @stale,
# @unverified, @fresh) or none (the terminal one), and the freshness ones
# match every path whenever their marker file exists. So this block has to be
# a `handle` to be sorted with them, and its path matcher puts it first.
# `handle_path` is its own directive with its own place in the order, which
# is exactly the question this avoids.
witness_block() {
  printf '\t# witness-2: btx-witness reading btxd2 on %s (easyNode deploy/esplora/witness-2).\n' "$WITNESS_LOCAL"
  # shellcheck disable=SC2016  # the backticks are Caddyfile comment text
  printf '\t# A `handle` with one path matcher sorts ahead of every handle in (esplora_api).\n'
  printf '\thandle /witness/* {\n'
  printf '\t\turi strip_prefix /witness\n'
  # Its own per-IP zone, written inline like the live box's @rawblock route:
  # the live file has no (perip_limit) snippet (seen 2026-10-03, run refused).
  printf '\t\trate_limit {\n'
  printf '\t\t\tzone witness {\n'
  printf '\t\t\t\tkey {remote_host}\n'
  printf '\t\t\t\tevents 120\n'
  printf '\t\t\t\twindow 10s\n'
  printf '\t\t\t}\n'
  printf '\t\t}\n'
  printf '\t\treverse_proxy %s\n' "$WITNESS_LOCAL"
  printf '\t}\n'
}

say() { printf '[witness-2] %s\n' "$*"; }

# <url> <sha256>: https:// or file://, and 64 hex characters.
valid_args() {
  local url="${1:-}" sha="${2:-}"
  case "$url" in
    https://?*|file:///?*) ;;
    *) echo "the binary URL must start with https:// (or file:// for a file on this box)" >&2; return 1 ;;
  esac
  if ! printf '%s' "$sha" | grep -Eq '^[0-9a-fA-F]{64}$'; then
    echo "the sha256 must be 64 hex characters" >&2
    return 1
  fi
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# <file> <expected>: case-insensitive.
sha_matches() {
  local got want
  got="$(sha256_of "$1")" || return 1
  want="$(printf '%s' "$2" | tr 'A-F' 'a-f')"
  [ -n "$want" ] && [ "$got" = "$want" ]
}

caddy_has_witness() {
  grep -Eq '^[[:space:]]*handle /witness/\*' "$1"
}

# <in> <out>: write <in> plus the witness block, placed on the line after the
# api.btxscan.io site's opening line. Refuses (non-zero, <out> not written)
# when the route is already there, when there is not exactly one such site,
# or when the rate_limit plugin's global order line is missing.
caddy_insert_witness() {
  local in="$1" out="$2" sites
  if caddy_has_witness "$in"; then
    echo "$in already has a /witness/ route" >&2
    return 1
  fi
  if ! grep -Eq '^[[:space:]]*order rate_limit ' "$in"; then
    echo "$in has no 'order rate_limit' (the rate_limit plugin is not wired); not editing" >&2
    return 1
  fi
  sites="$(grep -c "^${SITE//./\\.} {[[:space:]]*\$" "$in")"
  if [ "$sites" != "1" ]; then
    echo "expected exactly one '$SITE {' line in $in, found $sites; not editing" >&2
    grep -n '^[^[:space:]#(].*{[[:space:]]*$' "$in" >&2
    return 1
  fi
  local at
  at="$(grep -n "^${SITE//./\\.} {[[:space:]]*\$" "$in" | cut -d: -f1)"
  { head -n "$at" "$in"; witness_block; tail -n "+$((at + 1))" "$in"; } > "$out"
}

# ── the run itself ───────────────────────────────────────────────────────────

STAGE="start"
CHANGED_UNIT=""      # "new" when this run created the unit
CHANGED_BIN=""       # path of the previous binary's backup, or "new"
CADDY_BACKUP=""      # set once the Caddyfile has been edited
DONE=""
TMP=""

rollback() {
  say "rolling back"
  if [ -n "$CADDY_BACKUP" ]; then
    cat "$CADDY_BACKUP" > "$CADDYFILE" && say "Caddyfile restored from $CADDY_BACKUP"
    if systemctl reload caddy; then
      say "caddy reloaded with the old file"
    else
      say "WARNING: caddy reload failed after the restore; check: systemctl status caddy"
    fi
  fi
  if [ "$CHANGED_UNIT" = "new" ]; then
    systemctl disable --now "$UNIT_NAME" >/dev/null 2>&1
    rm -f "$UNIT_PATH"
    systemctl daemon-reload
    say "$UNIT_NAME stopped, disabled and removed"
  fi
  if [ "$CHANGED_BIN" = "new" ]; then
    rm -f "$BIN" && say "$BIN removed"
  elif [ -n "$CHANGED_BIN" ]; then
    if install -m 0755 "$CHANGED_BIN" "$BIN"; then
      say "previous $BIN put back"
    else
      say "WARNING: could not put the previous $BIN back; it is kept as $CHANGED_BIN"
    fi
    [ "$CHANGED_UNIT" = "new" ] || systemctl restart "$UNIT_NAME" >/dev/null 2>&1
  fi
}

on_exit() {
  local code=$?
  [ -n "$TMP" ] && rm -rf "$TMP"
  if [ -z "$DONE" ]; then
    say "FAILED at: $STAGE"
    rollback
    say "VERDICT: not installed. Nothing is left changed except backups (/etc/caddy/Caddyfile.bak-witness2-*, $BIN.bak-witness2-*)."
    [ "$code" -eq 0 ] && code=1
  fi
  exit "$code"
}

die() { say "ERROR: $*"; exit 1; }

is_height() { printf '%s' "$1" | grep -Eq '^[0-9]+$' && [ "$1" -gt 0 ]; }

main() {
  local url="${1:-}" sha="${2:-}"
  trap on_exit EXIT
  STAGE="checking the arguments"
  valid_args "$url" "$sha" || die "usage: install-witness-2.sh <binary-url> <expected-sha256>"
  [ "$(id -u)" = "0" ] || die "run as root (az vm run-command does)"
  for c in curl systemctl caddy install awk grep; do
    command -v "$c" >/dev/null 2>&1 || die "$c is not on this machine"
  done
  [ -f "$CADDYFILE" ] || die "$CADDYFILE does not exist"
  [ -r /data/btx2/.cookie ] || die "/data/btx2/.cookie does not exist; is btxd2 running?"
  local stamp
  stamp="$(date -u +%Y%m%dT%H%M%SZ)"
  TMP="$(mktemp -d)"

  STAGE="downloading the binary"
  curl -fsSL --max-time 300 -o "$TMP/btx-witness" "$url" || die "download failed: $url"
  STAGE="checking the sha256"
  sha_matches "$TMP/btx-witness" "$sha" \
    || die "sha256 mismatch: expected $sha, got $(sha256_of "$TMP/btx-witness"). Refusing it."
  [ "$(head -c 4 "$TMP/btx-witness" | od -An -c | tr -d ' ')" = '177ELF' ] \
    || die "the download is not a Linux binary"
  chmod 0755 "$TMP/btx-witness"
  "$TMP/btx-witness" --help >/dev/null 2>&1 || die "the binary does not run on this machine (btx-witness --help failed)"
  say "binary ok, sha256 $sha"

  STAGE="checking 127.0.0.1:3081 is free"
  if [ ! -f "$UNIT_PATH" ] && curl -s --max-time 3 -o /dev/null "http://$WITNESS_LOCAL/" ; then
    die "something already answers on $WITNESS_LOCAL and it is not $UNIT_NAME"
  fi

  STAGE="installing $BIN"
  if [ -f "$BIN" ]; then
    if sha_matches "$BIN" "$sha"; then
      say "$BIN is already this binary"
    else
      cp -p "$BIN" "$BIN.bak-witness2-$stamp" || die "could not back up $BIN"
      CHANGED_BIN="$BIN.bak-witness2-$stamp"
      install -m 0755 -o root -g root "$TMP/btx-witness" "$BIN" || die "install to $BIN failed"
      say "$BIN replaced (old one kept as $CHANGED_BIN)"
    fi
  else
    CHANGED_BIN="new"
    install -m 0755 -o root -g root "$TMP/btx-witness" "$BIN" || die "install to $BIN failed"
    say "$BIN installed"
  fi

  STAGE="installing and starting $UNIT_NAME"
  [ -f "$UNIT_PATH" ] || CHANGED_UNIT="new"
  unit_text > "$UNIT_PATH" || die "could not write $UNIT_PATH"
  systemctl daemon-reload || die "systemctl daemon-reload failed"
  systemctl enable "$UNIT_NAME" >/dev/null 2>&1 || die "systemctl enable $UNIT_NAME failed"
  systemctl restart "$UNIT_NAME" || die "systemctl restart $UNIT_NAME failed"

  STAGE="waiting for the witness to answer on $WITNESS_LOCAL"
  local local_tip=""
  for _ in $(seq 1 90); do
    local_tip="$(curl -fsS --max-time 3 "http://$WITNESS_LOCAL/blocks/tip/height" 2>/dev/null)"
    is_height "$local_tip" && break
    local_tip=""
    sleep 1
  done
  if [ -z "$local_tip" ]; then
    journalctl -u "$UNIT_NAME" -n 15 --no-pager 2>/dev/null
    die "no height from $WITNESS_LOCAL after 90 s"
  fi
  say "witness answers locally: tip $local_tip"

  STAGE="checking the public Esplora route before touching Caddy"
  local esplora_before
  esplora_before="$(curl -fsS --max-time 10 --resolve "$SITE:443:127.0.0.1" "https://$SITE/blocks/tip/height" 2>/dev/null)"

  STAGE="editing $CADDYFILE"
  if caddy_has_witness "$CADDYFILE"; then
    say "$CADDYFILE already has the /witness/ route; not editing it"
  else
    cp -p "$CADDYFILE" "$CADDYFILE.bak-witness2-$stamp" || die "could not back up $CADDYFILE"
    caddy_insert_witness "$CADDYFILE" "$TMP/Caddyfile.new" || die "could not place the route"
    CADDY_BACKUP="$CADDYFILE.bak-witness2-$stamp"
    # cat >, not mv: keeps the file's owner, mode and inode.
    cat "$TMP/Caddyfile.new" > "$CADDYFILE" || die "could not write $CADDYFILE"
    say "$CADDYFILE edited (backup $CADDY_BACKUP)"
    STAGE="caddy validate"
    caddy validate --config "$CADDYFILE" --adapter caddyfile >"$TMP/validate.log" 2>&1 \
      || { tail -5 "$TMP/validate.log"; die "caddy validate refused the edited file"; }
    STAGE="systemctl reload caddy"
    systemctl reload caddy || die "systemctl reload caddy failed"
  fi

  STAGE="asking https://$SITE/witness/ through Caddy"
  local public_tip=""
  for _ in $(seq 1 15); do
    public_tip="$(curl -fsS --max-time 10 --resolve "$SITE:443:127.0.0.1" "https://$SITE/witness/blocks/tip/height" 2>/dev/null)"
    is_height "$public_tip" && break
    public_tip=""
    sleep 1
  done
  [ -n "$public_tip" ] || die "https://$SITE/witness/blocks/tip/height did not answer a height"
  if is_height "$esplora_before"; then
    STAGE="checking the Esplora route still answers"
    local esplora_after
    esplora_after="$(curl -fsS --max-time 10 --resolve "$SITE:443:127.0.0.1" "https://$SITE/blocks/tip/height" 2>/dev/null)"
    is_height "$esplora_after" || die "https://$SITE/blocks/tip/height stopped answering after the edit"
  fi

  DONE=1
  say "VERDICT: witness-2 is up. Local tip $local_tip, https://$SITE/witness/ tip $public_tip."
  [ -n "$CADDY_BACKUP" ] && say "Caddyfile backup: $CADDY_BACKUP"
  return 0
}

# Sourced by test-install-witness-2.sh for its functions: only run when executed.
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  main "$@"
fi
