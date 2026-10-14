#!/usr/bin/env python3
"""What this BTX node has given the network: bytes, blocks and headers served.

Three commands:

  snapshot  Counters since the node last started. Needs no history, so it is
            the first thing to run on any node, including one you have never
            measured before.
  sample    Append one row of counters to a local log. Run it every 5 minutes
            (systemd timer or cron). btxd resets every counter on restart, so
            without this log there is no "last 24 hours" and no total across
            restarts.
  report    Turn the log into time windows (default 1h, 24h and everything
            logged). If btxd's `net` debug category was on, it also reads
            debug.log for EXACT counts of blocks sent and header requests
            answered; otherwise blocks and headers are given in bytes only.

Read-only against the node, with one exception you opt into:
`sample --ensure-net-logging` turns on the `net` debug category through the
`logging` RPC. That is a runtime log toggle that a restart undoes, which is why
the sampler re-asserts it every run. It makes debug.log grow by one line per P2P
message, so on a busy archive node watch the disk.

Do NOT use --ensure-net-logging on a node the easyNode app manages. The app
judges stalls and the GPU verifier from the last 64 KB of debug.log, and net
lines would push those markers out of that window within minutes. Byte totals
need no log and are exact either way; only the block and header COUNTS need it.

Nothing leaves the machine. The log keeps counters, btxd's per-run peer ids and
connection times. It keeps a peer's address only for MANUAL connections, the
ones the operator configured, so the report can say how much went to a named
mirror; inbound strangers are never recorded by address.

Examples:
  python3 btx-service-stats.py --cli "btx-cli -datadir=/data/btx" snapshot
  python3 btx-service-stats.py --cli "btx-cli -datadir=/data/btx" \\
      --log ~/btx-service-stats.jsonl sample --ensure-net-logging
  python3 btx-service-stats.py --cli "btx-cli -datadir=/data/btx" \\
      --log ~/btx-service-stats.jsonl report --windows 1h,24h,all
"""
import argparse
import collections
import json
import os
import re
import shlex
import subprocess
import sys
import time
from datetime import datetime, timezone

# Block payload of the whole chain, measured 2026-09-04 with
# scripts/measure-chain-size.py: 123.8 GiB. It grows about 8 MB a day since the
# fork ended the 1 MB MatMul blocks, which does not move this figure at the
# precision it is printed with.
CHAIN_BYTES = 124 * 1024 ** 3

# btxd sends a full block either as one `block` message or, with bounded chunk
# relay, as one `blkchnkman` manifest followed by `blkchunk` payloads. Both
# shapes are one block served, so an exact count is block + blkchnkman.
GROUPS = [
    ("blocks", ("block", "blkchnkman", "blkchunk", "merkleblock")),
    ("new-block relay", ("cmpctblock", "blocktxn")),
    ("headers", ("headers",)),
    ("attestations", ("mmattest", "mmsketch", "mmatrefute")),
    ("snapshots", ("utxomanifest", "utxochunk")),
    ("transactions", ("tx", "shieldedtx", "shieldeddata", "dltx")),
    ("filters", ("cfilter", "cfheaders", "cfcheckpt")),
]
MSG_GROUP = {m: g for g, ms in GROUPS for m in ms}
GROUP_ORDER = [g for g, _ in GROUPS] + ["other"]

# A peer more than this many blocks behind us is still syncing. Same number as
# the census's settled depth and btxd's own emergency park depth.
BEHIND_BLOCKS = 6

SEND_RE = re.compile(r"sending (\S+) \((\d+) bytes\) peer=(\d+)")
RECV_RE = re.compile(r"received: (\S+) \((\d+) bytes\) peer=(\d+)")
STARTUP_RE = re.compile(r"\bBTX version v|Bitcoin Core version v")
TS_RE = re.compile(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(?:\.\d+)?Z")


class Node:
    def __init__(self, cli):
        self.cmd = shlex.split(cli)

    def rpc(self, *args):
        out = subprocess.run(self.cmd + list(args), capture_output=True, text=True, timeout=60)
        if out.returncode != 0:
            raise SystemExit("btx-cli %s failed (%d): %s" % (args[0], out.returncode, out.stderr.strip()))
        text = out.stdout.strip()
        # btx-cli prints a string result as a bare line, not JSON.
        try:
            return json.loads(text)
        except ValueError:
            return text

    def datadir(self):
        for a in self.cmd:
            if a.startswith("-datadir="):
                return os.path.expanduser(a.split("=", 1)[1])
        for d in ("~/.bitcoin", "~/.btx"):
            if os.path.isdir(os.path.expanduser(d)):
                return os.path.expanduser(d)
        return None


def fmt_bytes(n):
    """Decimal units, the same ones Yarek's archive report uses."""
    n = float(n)
    for unit, size in (("TB", 1e12), ("GB", 1e9), ("MB", 1e6), ("kB", 1e3)):
        if n >= size:
            return "%.1f %s" % (n / size, unit)
    return "%d B" % n


def fmt_span(secs):
    secs = int(max(0, secs))
    d, rem = divmod(secs, 86400)
    h, rem = divmod(rem, 3600)
    m = rem // 60
    if d:
        return "%dd %dh" % (d, h)
    if h:
        return "%dh %02dm" % (h, m)
    return "%dm" % m


def fmt_utc(t):
    return datetime.fromtimestamp(t, timezone.utc).strftime("%Y-%m-%d %H:%M UTC")


def chains(n):
    x = n / CHAIN_BYTES
    if x >= 0.1:
        return "%.1fx the full chain" % x
    return "%.2f%% of the full chain" % (100 * x)


def group_totals(per_msg):
    out = collections.Counter()
    for msg, b in per_msg.items():
        out[MSG_GROUP.get(msg, "other")] += b
    return out


def read_state(node):
    up = int(node.rpc("uptime"))
    nt = node.rpc("getnettotals")
    bc = node.rpc("getblockchaininfo")
    ni = node.rpc("getnetworkinfo")
    peers = node.rpc("getpeerinfo")
    return up, nt, bc, ni, peers


def peer_row(p):
    row = {
        "k": "%d:%d" % (p["id"], p.get("conntime", 0)),
        "in": bool(p.get("inbound")),
        "ct": p.get("connection_type", ""),
        "sb": p.get("synced_blocks", -1),
        "sh": p.get("startingheight", -1),
        "s": {m: b for m, b in p.get("bytessent_per_msg", {}).items() if b},
    }
    if p.get("connection_type") == "manual":
        row["a"] = p.get("addr", "")
    return row


def describe_node(bc, ni):
    services = set(ni.get("localservicesnames", []))
    kind = "archive (serves every block)" if "NETWORK" in services else (
        "pruned (serves only recent blocks)" if "NETWORK_LIMITED" in services else "does not serve blocks")
    return "%s  %s, tip %d" % (ni.get("subversion", "?"), kind, bc.get("blocks", -1))


def right_now_lines(bc, ni, peers):
    tip = bc.get("blocks", 0)
    inbound = sum(1 for p in peers if p.get("inbound"))
    behind = [p for p in peers if 0 <= p.get("synced_blocks", -1) < tip - BEHIND_BLOCKS]
    catching_up = [p for p in behind if p.get("bytessent_per_msg", {}).get("block", 0)
                   + p.get("bytessent_per_msg", {}).get("blkchunk", 0) > 0]
    return [
        ("Connections", "%d (%d inbound, %d outbound)" % (len(peers), inbound, len(peers) - inbound)),
        ("Peers more than %d blocks behind us" % BEHIND_BLOCKS, str(len(behind))),
        ("...of those, taking blocks from us", str(len(catching_up))),
    ]


def print_table(title, rows):
    print(title)
    w = max(len(k) for k, _ in rows) if rows else 0
    for k, v in rows:
        print("  %-*s  %s" % (w, k, v))
    print()


def cmd_snapshot(node, args):
    up, nt, bc, ni, peers = read_state(node)
    now = time.time()
    sent = nt["totalbytessent"]
    per_msg = collections.Counter()
    for p in peers:
        per_msg.update(p.get("bytessent_per_msg", {}))
    groups = group_totals(per_msg)
    attributed = sum(groups.values())

    print("BTX node service snapshot, %s. GB = 10^9 bytes." % fmt_utc(now))
    print(describe_node(bc, ni))
    print()
    rows = [
        ("Node up since", "%s (%s)" % (fmt_utc(now - up), fmt_span(up))),
        ("Data sent (exact)", "%s, %s" % (fmt_bytes(sent), chains(sent))),
        ("Data received (exact)", fmt_bytes(nt["totalbytesrecv"])),
    ]
    print_table("Since this node started", rows)
    rows = [(g, fmt_bytes(groups[g])) for g in GROUP_ORDER if groups.get(g)]
    rows.append(("(to peers since disconnected)", fmt_bytes(max(0, sent - attributed))))
    print_table("Of the data sent, by kind (peers still connected)", rows)
    print_table("Right now", right_now_lines(bc, ni, peers))
    manual = [p for p in peers if p.get("connection_type") == "manual"]
    if manual:
        rows = [(p.get("addr", "?"), "%s sent, connected %s" % (fmt_bytes(p.get("bytessent", 0)),
                                                                fmt_span(now - p.get("conntime", now))))
                for p in manual]
        print_table("Manual peers (configured links)", rows)


def cmd_sample(node, args):
    if not args.log:
        raise SystemExit("sample needs --log")
    if args.ensure_net_logging:
        on = node.rpc("logging")
        if isinstance(on, dict) and not on.get("net"):
            node.rpc("logging", '["net"]')
    up, nt, bc, ni, peers = read_state(node)
    row = {
        "t": int(time.time()),
        "up": up,
        "sent": nt["totalbytessent"],
        "recv": nt["totalbytesrecv"],
        "tip": bc.get("blocks", -1),
        "peers": [peer_row(p) for p in peers],
    }
    with open(os.path.expanduser(args.log), "a") as f:
        f.write(json.dumps(row, separators=(",", ":")) + "\n")


def load_log(path):
    rows = []
    with open(os.path.expanduser(path)) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                rows.append(json.loads(line))
            except ValueError:
                continue  # a torn last line from a killed writer
    rows.sort(key=lambda r: r["t"])
    return rows


def restarted(a, b):
    return b["up"] < a["up"] or b["sent"] < a["sent"]


def window_from_log(rows, start):
    """Sum deltas over the pairs that end inside [start, now].

    The baseline is the last row at or before `start`, so the covered span can
    begin up to one sample interval early; the report prints the real span.
    """
    base = 0
    for i, r in enumerate(rows):
        if r["t"] <= start:
            base = i
    span = rows[base:]
    res = {
        "from": span[0]["t"], "to": span[-1]["t"], "sent": 0, "recv": 0, "restarts": 0,
        "groups": collections.Counter(), "block_peers": set(), "inbound": set(), "manual": collections.Counter(),
    }
    for a, b in zip(span, span[1:]):
        if restarted(a, b):
            # Bytes between `a` and the shutdown are gone with the old counters.
            res["restarts"] += 1
            res["sent"] += b["sent"]
            res["recv"] += b["recv"]
            prev = {}
        else:
            res["sent"] += b["sent"] - a["sent"]
            res["recv"] += b["recv"] - a["recv"]
            prev = {p["k"]: p for p in a["peers"]}
        for p in b["peers"]:
            old = prev.get(p["k"], {}).get("s", {})
            delta = collections.Counter()
            for m, v in p["s"].items():
                d = v - old.get(m, 0)
                if d > 0:
                    delta[m] = d
            res["groups"].update(group_totals(delta))
            # A peer key is per btxd run; pair it with the run's start so two
            # runs reusing id 3 are not one peer.
            run_key = (b["t"] - b["up"]) // 60
            if delta.get("block") or delta.get("blkchnkman") or delta.get("blkchunk"):
                res["block_peers"].add((run_key, p["k"]))
            if p.get("in"):
                res["inbound"].add((run_key, p["k"]))
            if p.get("a"):
                res["manual"][p["a"]] += sum(delta.values())
    return res


def scan_debug_log(path, starts):
    """Exact per-message counts from btxd's `net` debug lines, one result per
    start time (None where no net line falls after it). One pass over the file,
    which on a busy archive node can be gigabytes."""
    if not path or not os.path.exists(path):
        return [None] * len(starts)
    start_s = [fmt_iso(s) for s in starts]
    earliest = min(start_s)
    res = [{"first": None, "sent": collections.Counter(), "sent_bytes": collections.Counter(),
            "recv": collections.Counter(), "block_peers": set()} for _ in starts]
    run = 0
    with open(path, errors="replace") as f:
        for line in f:
            m = TS_RE.match(line)
            if not m:
                continue
            ts = m.group(1)
            if STARTUP_RE.search(line):
                run += 1
                continue
            if ts < earliest or ("sending " not in line and "received: " not in line):
                continue
            s = SEND_RE.search(line)
            r = None if s else RECV_RE.search(line)
            if not s and not r:
                continue
            for i, st in enumerate(start_s):
                if ts < st:
                    continue
                w = res[i]
                w["first"] = w["first"] or ts
                if s:
                    msg = s.group(1)
                    w["sent"][msg] += 1
                    w["sent_bytes"][msg] += int(s.group(2))
                    if msg in ("block", "blkchnkman"):
                        w["block_peers"].add((run, s.group(3)))
                else:
                    w["recv"][r.group(1)] += 1
    return [w if w["first"] else None for w in res]


def parse_window(w):
    if w == "all":
        return None
    unit = {"m": 60, "h": 3600, "d": 86400}[w[-1]]
    return int(float(w[:-1]) * unit)


def cmd_report(node, args):
    if not args.log:
        raise SystemExit("report needs --log")
    rows = load_log(args.log)
    if len(rows) < 2:
        raise SystemExit("the log has %d row(s); let the sampler run for at least two intervals" % len(rows))
    now = rows[-1]["t"]
    debuglog = args.debuglog or (os.path.join(node.datadir(), "debug.log") if node.datadir() else None)

    print("BTX node service report, %s. GB = 10^9 bytes." % fmt_utc(now))
    try:
        up, nt, bc, ni, peers = read_state(node)
        print(describe_node(bc, ni))
    except SystemExit as e:
        bc = ni = peers = None
        print("(node not answering: %s)" % e)
    print("Log: %d samples, %s to %s." % (len(rows), fmt_utc(rows[0]["t"]), fmt_utc(now)))
    print()

    windows = []
    for w in args.windows.split(","):
        secs = parse_window(w.strip())
        start = rows[0]["t"] if secs is None else now - secs
        windows.append((w.strip(), secs, window_from_log(rows, start)))
    exacts = scan_debug_log(debuglog, [res["from"] for _, _, res in windows])

    for (w, secs, res), exact in zip(windows, exacts):
        label = "Everything logged" if secs is None else "Last %s" % w
        covered = res["to"] - res["from"]
        if secs is not None and covered + 600 < secs:
            label += " (log only covers %s)" % fmt_span(covered)
        out = [
            ("Span", "%s to %s (%s)" % (fmt_utc(res["from"]), fmt_utc(res["to"]), fmt_span(covered))),
            ("Data sent", "%s, %s" % (fmt_bytes(res["sent"]), chains(res["sent"]))),
            ("Data received", fmt_bytes(res["recv"])),
        ]
        g = res["groups"]
        for name in GROUP_ORDER:
            if g.get(name):
                out.append(("  %s" % name, fmt_bytes(g[name])))
        lost = res["sent"] - sum(g.values())
        if lost > 0:
            out.append(("  (to peers that left between samples)", fmt_bytes(lost)))
        out.append(("Peers we sent blocks to", str(len(res["block_peers"]))))
        out.append(("Inbound connections seen", str(len(res["inbound"]))))
        if res["restarts"]:
            out.append(("Node restarts in span", "%d (bytes after the last sample before each "
                                                  "restart are not counted)" % res["restarts"]))
        for addr, b in res["manual"].most_common():
            out.append(("To manual peer %s" % addr, fmt_bytes(b)))

        if exact:
            s = exact["sent"]
            note = "" if exact["first"] <= fmt_iso(res["from"] + 600) else " (net logging since %sZ)" % exact["first"]
            out.append(("Blocks sent, exact%s" % note,
                        "%d (%s)" % (s["block"] + s["blkchnkman"],
                                     fmt_bytes(sum(exact["sent_bytes"][m] for m in ("block", "blkchnkman", "blkchunk"))))))
            out.append(("Header requests answered, exact", "%d getheaders received, %d headers messages sent"
                        % (exact["recv"]["getheaders"], s["headers"])))
            out.append(("Compact new-block announcements", str(s["cmpctblock"])))
            out.append(("Attestations sent", str(s["mmattest"])))
            out.append(("Distinct peers served blocks (log)", str(len(exact["block_peers"]))))
        else:
            out.append(("Exact block/header counts", "n/a, btxd net logging was off (see --help before turning it on)"))
        print_table(label, out)

    if peers is not None:
        print_table("Right now", right_now_lines(bc, ni, peers))


def fmt_iso(t):
    return datetime.fromtimestamp(t, timezone.utc).strftime("%Y-%m-%dT%H:%M:%S")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--cli", default="btx-cli", help='btx-cli and its connection args, one string')
    ap.add_argument("--log", help="sample log (JSON lines)")
    ap.add_argument("--debuglog", help="btxd debug.log (default: <datadir>/debug.log)")
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("snapshot")
    sp = sub.add_parser("sample")
    sp.add_argument("--ensure-net-logging", action="store_true")
    rp = sub.add_parser("report")
    rp.add_argument("--windows", default="1h,24h,all")
    args = ap.parse_args()
    node = Node(args.cli)
    {"snapshot": cmd_snapshot, "sample": cmd_sample, "report": cmd_report}[args.cmd](node, args)


if __name__ == "__main__":
    main()
