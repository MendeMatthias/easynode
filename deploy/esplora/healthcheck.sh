#!/usr/bin/env bash
# Esplora node health monitor. Run it from cron every ~5 min. Ported from
# btx-esplora; one change, explained below.
#
# Checks: btxd tip vs the network, electrs liveness + tip-lag, disk headroom.
# Emits a single status line; exits non-zero on any RED so a cron mailer or an
# uptime pinger can alert.
#
# THE ONE CHANGE. The original compared btxd against two explorers,
# esplora.btxbyronbay.com and explorer.minebtx.com, "independent operators
# only". Both are gone, and on 2026-09-05 the remaining explorer sat on a
# minority branch for a day, so an explorer's height is not a network tip. The
# reference is now the chain census at easybtx.com/api/nodes: the tip of the
# chain that carries the most work, measured from every reachable node's own
# headers. A stale or unreachable census makes the tip check UNKNOWN (said in
# the status line), never GREEN by default.
set -u

BCLI="${BTX_CLI:-btx-cli}"
ESPLORA="${BTX_LOCAL_ESPLORA:-http://127.0.0.1:3000}"
CENSUS_URL="${BTX_CENSUS_URL:-https://easybtx.com/api/nodes}"
CENSUS_MAX_AGE="${BTX_CENSUS_MAX_AGE:-1800}"
DISK_MOUNT="${BTX_DISK_MOUNT:-/}"
MAX_TIP_LAG=6            # blocks electrs may trail btxd before RED (~9 min at 90s spacing)
MAX_NET_LAG=10           # blocks btxd may trail the network before RED
DISK_WARN_PCT=80         # RED above this

status="GREEN"; msgs=()

# --- btxd tip vs the census ---
node_tip=$($BCLI getblockcount 2>/dev/null)
net_tip=$(curl -sS -m 15 -A "easynode-healthcheck" "$CENSUS_URL" 2>/dev/null | python3 -c '
import json, sys, time
try:
    d = json.load(sys.stdin)
except Exception:
    sys.exit(0)
if time.time() - int(d.get("checkedAt") or 0) > int(sys.argv[1]):
    sys.exit(0)
for c in ((d.get("chains") or {}).get("chains") or []):
    if c.get("heaviest") and c.get("tipHeight") is not None:
        print(int(c["tipHeight"]))
        break
' "$CENSUS_MAX_AGE" 2>/dev/null)
if [ -z "$node_tip" ]; then status="RED"; msgs+=("btxd_rpc_down");
elif [[ "$net_tip" =~ ^[0-9]+$ ]] && [ "$net_tip" -gt 0 ]; then
  lag=$(( net_tip - node_tip ))
  [ "$lag" -gt "$MAX_NET_LAG" ] && { status="RED"; msgs+=("btxd_behind_census:$lag"); }
else
  msgs+=("census_unknown")
fi

# --- electrs liveness + tip-lag ---
esplora_tip=$(curl -s -m 10 "$ESPLORA/blocks/tip/height" 2>/dev/null)
if ! [[ "$esplora_tip" =~ ^[0-9]+$ ]]; then status="RED"; msgs+=("electrs_down");
elif [ -n "$node_tip" ]; then
  elag=$(( node_tip - esplora_tip ))
  [ "$elag" -gt "$MAX_TIP_LAG" ] && { status="RED"; msgs+=("electrs_tiplag:$elag"); }
fi

# --- disk ---
dpct=$(df --output=pcent "$DISK_MOUNT" 2>/dev/null | tail -1 | tr -dc '0-9')
[ -n "$dpct" ] && [ "$dpct" -ge "$DISK_WARN_PCT" ] && { status="RED"; msgs+=("disk:${dpct}%"); }

echo "$(date -u +%FT%TZ) $status btxd_tip=${node_tip:-NA} census_tip=${net_tip:-NA} electrs_tip=${esplora_tip:-NA} disk=${dpct:-NA}% ${msgs[*]:-ok}"
[ "$status" = "GREEN" ]
