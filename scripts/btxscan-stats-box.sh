# btxscan-stats-box.sh: what both btxd on the btxscan box have served, in one READ-ONLY run-command.
# Every call is a read: uptime, getnettotals, getnetworkinfo, getpeerinfo, getblockcount. Nothing is
# written, nothing restarts. Output stays under run-command's 4 KB stdout limit.
#
#   az vm run-command invoke -g BTX-ESPLORA-RG -n btx-esplora-1 --command-id RunShellScript \
#     --scripts @btxscan-stats-box.sh --query 'value[0].message' -o tsv
for UNIT in btxd btxd2; do
  # Same discovery as btx-ops box-resolve.sh: the LAST ExecStart of the unit names binary, datadir, conf.
  line=$(systemctl cat "$UNIT" 2>/dev/null | sed -n 's/^ExecStart=//p' | sed '/^[[:space:]]*$/d' | tail -1)
  bin=$(printf '%s' "$line" | awk '{print $1}'); args=$(printf '%s' "$line" | cut -s -d' ' -f2-)
  cli="$(dirname "$bin")/btx-cli"
  args=$(printf '%s' "$args" | tr ' ' '\n' | grep -E '^-(datadir|conf)=' | tr '\n' ' ')
  conf=$(printf '%s' "$args" | tr ' ' '\n' | sed -n 's/^-conf=//p')
  [ -n "$conf" ] || conf="$(printf '%s' "$args" | tr ' ' '\n' | sed -n 's/^-datadir=//p')/bitcoin.conf"
  echo "### $UNIT"
  { $cli $args uptime; echo '#NT'; $cli $args getnettotals; echo '#NI'; $cli $args getnetworkinfo
    echo '#PI'; $cli $args getpeerinfo; echo '#BC'; $cli $args getblockcount; } > /tmp/.stats.$UNIT 2> /tmp/.stats.$UNIT.err
  [ -s /tmp/.stats.$UNIT.err ] && { echo "$UNIT stderr:"; head -c 300 /tmp/.stats.$UNIT.err; echo; }
  python3 - "$UNIT" "$conf" /tmp/.stats.$UNIT <<'PY'
import json, re, sys, time
unit, conf, path = sys.argv[1:4]
raw = open(path).read()
up, rest = raw.split('#NT', 1)
nt, rest = rest.split('#NI', 1)
ni, rest = rest.split('#PI', 1)
pi, bc = rest.split('#BC', 1)
up, nt, ni, peers, tip = int(up.strip()), json.loads(nt), json.loads(ni), json.loads(pi), int(bc.strip())
G = {'blocks': ('block', 'blkchnkman', 'blkchunk', 'merkleblock'), 'relay': ('cmpctblock', 'blocktxn'),
     'headers': ('headers',), 'attest': ('mmattest', 'mmsketch', 'mmatrefute'),
     'snap': ('utxomanifest', 'utxochunk'), 'tx': ('tx', 'shieldedtx', 'shieldeddata', 'dltx')}
grp = {g: 0 for g in G}; grp['other'] = 0
for p in peers:
    for m, b in p.get('bytessent_per_msg', {}).items():
        grp[next((g for g, ms in G.items() if m in ms), 'other')] += b
sb = lambda p: sum(p.get('bytessent_per_msg', {}).get(m, 0) for m in G['blocks'])
inb = [p for p in peers if p.get('inbound')]
behind = [p for p in peers if 0 <= p.get('synced_blocks', -1) < tip - 6]
taking = [p for p in behind if sb(p) > 0]
gb = lambda n: '%.2f GB' % (n / 1e9)
ut = nt.get('uploadtarget', {})
cfg = {}
try:
    for l in open(conf):
        m = re.match(r'\s*(maxuploadtarget|prune|maxconnections|whitelist)\s*=\s*(\S+)', l)
        if m: cfg.setdefault(m.group(1), []).append(m.group(2))
except OSError as e:
    cfg = {'unreadable': str(e)[:40]}
top = sorted(peers, key=sb, reverse=True)[:5]
print(json.dumps({
    'unit': unit, 'ver': ni.get('subversion'), 'tip': tip,
    'up_since': time.strftime('%Y-%m-%dT%H:%MZ', time.gmtime(time.time() - up)), 'up_h': round(up / 3600, 1),
    'sent': gb(nt['totalbytessent']), 'recv': gb(nt['totalbytesrecv']),
    'uploadtarget': {k: ut.get(k) for k in ('target', 'target_reached', 'serve_historical_blocks', 'bytes_left_in_cycle')},
    'conf': cfg,
    'conns': {'all': len(peers), 'in': len(inb), 'out': len(peers) - len(inb)},
    'behind_gt6': len(behind), 'taking_blocks_from_us': len(taking),
    'sent_by_kind_connected_peers': {g: gb(b) for g, b in grp.items() if b},
    'top_block_takers': [{'ua': p.get('subver', '')[:28], 'in': p.get('inbound'), 'blocks': gb(sb(p)),
                          'h': p.get('synced_blocks'), 'min': int((time.time() - p.get('conntime', time.time())) / 60)} for p in top if sb(p)],
    'agents': sorted({re.sub(r'\(.*\)', '(nick)', p.get('subver', '?')) for p in peers})[:8],
}, separators=(',', ':')))
PY
  rm -f /tmp/.stats.$UNIT /tmp/.stats.$UNIT.err
done
