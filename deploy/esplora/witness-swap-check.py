#!/usr/bin/env python3
"""Check a running btx-witness's five swap routes against the node it serves.

Called by deploy/esplora/test-witness.sh, which starts the witness and counts
the PASS / FAIL lines this prints. Every expected answer is computed from the
node's own RPC (through btx-cli), never from another explorer, and in electrs'
exact formats, so a PASS here means "byte for byte what electrs would say about
this node's chain". Read-only.

Environment: BTXCLI, DATADIR, CONF (used if the file exists), RPCPORT
(optional), BASE (the witness, http://127.0.0.1:port), HEIGHT, PRUNEH, WINDOW.
"""
import hashlib
import json
import os
import subprocess
import time
import urllib.error
import urllib.request

BASE = os.environ["BASE"]
HEIGHT = int(os.environ["HEIGHT"])
PRUNEH = int(os.environ.get("PRUNEH") or 0)
WINDOW = int(os.environ.get("WINDOW") or 2000)


def cli(*args):
    cmd = [os.environ["BTXCLI"], "-datadir=" + os.environ["DATADIR"]]
    conf = os.environ.get("CONF", "")
    if conf and os.path.isfile(conf):
        cmd.append("-conf=" + conf)
    if os.environ.get("RPCPORT"):
        cmd.append("-rpcport=" + os.environ["RPCPORT"])
    out = subprocess.run(cmd + [str(a) for a in args], capture_output=True, text=True)
    if out.returncode != 0:
        raise RuntimeError(out.stderr.strip())
    s = out.stdout.strip()
    # Only objects and arrays are parsed: a bare hash or hex string is printed
    # unquoted, and one made only of digits would otherwise parse as a number.
    return json.loads(s) if s[:1] in ("{", "[") else s


def get_once(path):
    try:
        r = urllib.request.urlopen(BASE + path, timeout=15)
        return r.status, r.headers.get("Content-Type"), r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.headers.get("Content-Type"), e.read().decode()


def get(path):
    # A 503 means the index is catching up with a block that arrived during
    # the run (it polls every two seconds); ask again rather than fail on it.
    for _ in range(5):
        r = get_once(path)
        if r[0] != 503:
            return r
        time.sleep(2)
    return r


def check(cond, what):
    print(("PASS " if cond else "FAIL ") + what)
    return cond


def status_json(h, b):
    return '{"confirmed":true,"block_height":%d,"block_hash":"%s","block_time":%d}' % (
        h, b["hash"], b["time"])


def sha256d(b):
    return hashlib.sha256(hashlib.sha256(b).digest()).digest()


def folds(txid, branch, pos, root):
    acc = bytes.fromhex(txid)[::-1]
    for sib in branch:
        s = bytes.fromhex(sib)[::-1]
        acc = sha256d(acc + s) if pos & 1 == 0 else sha256d(s + acc)
        pos >>= 1
    return acc[::-1].hex() == root


# A recent block with more than one transaction (a real merkle branch) and a
# confirmed spend, searched downward from six below the tip.
multi = spend = None
for h in range(HEIGHT - 6, max(HEIGHT - 6 - 400, PRUNEH), -1):
    b = cli("getblock", cli("getblockhash", h), 1)
    if b["nTx"] < 2:
        continue
    b2 = cli("getblock", b["hash"], 2)
    multi = multi or (h, b2)
    for t in b2["tx"][1:]:
        for i, v in enumerate(t["vin"]):
            if "txid" in v:
                spend = spend or (h, b2, t["txid"], i, v["txid"], v["vout"])
    if multi and spend:
        break
plain_h = HEIGHT - 6
plain = cli("getblock", cli("getblockhash", plain_h), 2)

blocks = [(plain_h, plain)] + ([multi] if multi else [])
if not multi:
    print("(no block with two or more transactions in the last 400; merkle branches are all empty)")

for h, b in blocks:
    for pos, t in enumerate(b["tx"][:6]):
        txid = t["txid"]
        c, ty, body = get("/tx/%s/status" % txid)
        check((c, ty, body) == (200, "application/json", status_json(h, b)),
              "status of %s… at %d is the node's block, in electrs' format" % (txid[:12], h))
        c, ty, body = get("/tx/%s/hex" % txid)
        raw = cli("getrawtransaction", txid, "false", b["hash"])
        check((c, ty, body) == (200, "text/plain", raw),
              "hex of %s… is the node's bytes (%d chars)" % (txid[:12], len(raw)))
        c, ty, body = get("/tx/%s/merkle-proof" % txid)
        ok = c == 200 and ty == "application/json"
        if ok:
            p = json.loads(body)
            ok = (p.get("block_height") == h and p.get("pos") == pos
                  and folds(txid, p["merkle"], pos, b["merkleroot"])
                  and body == json.dumps({"block_height": h, "merkle": p["merkle"], "pos": pos},
                                         separators=(",", ":")))
        check(ok, "merkle proof of %s… (pos %d of %d) folds into the header's root"
              % (txid[:12], pos, len(b["tx"])))
    c, ty, body = get("/block/%s/header" % b["hash"])
    check((c, ty, body) == (200, "text/plain", cli("getblockheader", b["hash"], "false")),
          "header of %d is the node's (%d hex chars)" % (h, len(body)))

if spend:
    h, b, spender, vin, ptx, pvout = spend
    want = '{"spent":true,"txid":"%s","vin":%d,"status":%s}' % (spender, vin, status_json(h, b))
    c, ty, body = get("/tx/%s/outspend/%d" % (ptx, pvout))
    check((c, ty, body) == (200, "application/json", want),
          "outspend %s…:%d names the spend in block %d, input %d" % (ptx[:12], pvout, h, vin))
else:
    print("(no confirmed spend in the last 400 blocks to check)")

# An output the node still holds unspent: "unspent", exactly.
cb = plain["tx"][0]["txid"]
held = cli("gettxout", cb, 0, "true")
c, ty, body = get("/tx/%s/outspend/0" % cb)
if held:
    check((c, ty, body) == (200, "application/json", '{"spent":false}'),
          "outspend of an unspent coinbase output -> {\"spent\":false}")
else:
    check(c == 200 and body.startswith('{"spent":true'),
          "outspend of a spent coinbase output names its spend")

# Below the window: the witness abstains rather than guessing.
below = HEIGHT - WINDOW - 50
if below > PRUNEH:
    bb = cli("getblock", cli("getblockhash", below), 1)
    t = bb["tx"][0]
    c, ty, body = get("/tx/%s/status" % t)
    check((c, body) == (404, "Transaction not found"),
          "a tx below the %d-block window -> 404, an abstention, not a guess" % WINDOW)
    c, ty, body = get("/tx/%s/outspend/0" % t)
    held = cli("gettxout", t, 0, "true")
    if held:
        check((c, body) == (200, '{"spent":false}'), "below the window, an output still in the UTXO set -> unspent")
    elif c == 200 and body.startswith('{"spent":true'):
        s = json.loads(body)
        sp = cli("getrawtransaction", s["txid"], 1, s["status"]["block_hash"])
        v = sp["vin"][s["vin"]]
        check(v.get("txid") == t and v.get("vout") == 0,
              "below the window, a spend made inside it is named and real")
    else:
        check((c, body) == (404, "Output not known to this witness"),
              "below the window, a spent output it cannot place -> 404")

# The header route needs no index: below the prune height too.
h1 = cli("getblockhash", 1)
c, ty, body = get("/block/%s/header" % h1)
check((c, body) == (200, cli("getblockheader", h1, "false")),
      "the header of block 1 is served%s" % (" (below pruneheight %d)" % PRUNEH if PRUNEH > 1 else ""))

# A block the node knows only off the active chain: 404, as electrs answers.
side = [t for t in cli("getchaintips") if t["status"] != "active"]
if side:
    c, ty, body = get("/block/%s/header" % side[0]["hash"])
    check((c, ty, body) == (404, "text/plain", "Block not found"),
          "a side-branch block (%s, %s) -> 404 Block not found" % (side[0]["hash"][:12], side[0]["status"]))

# Misses, in electrs' words.
nobody = "f" * 64
for path, want in [("/tx/%s/status" % nobody, "Transaction not found"),
                   ("/tx/%s/hex" % nobody, "Transaction not found"),
                   ("/tx/%s/merkle-proof" % nobody, "Transaction not found or is unconfirmed"),
                   ("/block/%s/header" % nobody, "Block not found")]:
    c, ty, body = get(path)
    check((c, ty, body) == (404, "text/plain", want), "%s… -> 404 %s" % (path[:16], want))

# One spelling: everything else is refused.
t = plain["tx"][0]["txid"]
for path in ["/tx/%s/status" % t.upper(), "/tx/%s/status/" % t, "/tx/%s/hex?x=1" % t,
             "/tx/%s/outspend/01" % t, "/tx/%s/outspend/10000" % t, "/tx/%s/outspend/-1" % t,
             "/tx/%s/outspends" % t, "/tx/%s/raw" % t, "/block/%s" % plain["hash"],
             "/block/%s/header/" % plain["hash"], "/block/%s/txids" % plain["hash"]]:
    c, _, _ = get(path)
    check(c == 404, "%s… -> 404" % path[:72])
