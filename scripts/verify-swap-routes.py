#!/usr/bin/env python3
"""Compare a candidate's five swap routes with a reference Esplora, live.

    scripts/verify-swap-routes.py CANDIDATE [REFERENCE] [--json] [--heights H,H,...]
                                  [--source URL]

REFERENCE defaults to https://api.btxscan.io (electrs). Blocks and their txids
are listed from --source (default https://api.btxscan.io, which serves the
electrs listing routes a non-electrs reference may lack). For each sampled block
it fetches, from both, the routes an HTLC swap client checks:

    /block/<hash>/header
    /tx/<txid>/status  /tx/<txid>/hex  /tx/<txid>/merkle-proof
    /tx/<txid>/outspend/0  /tx/<txid>/outspend/1

and compares them byte for byte (status code, content type and body), or as
parsed JSON with --json, which is the right comparison against a server that
is not electrs: Byron Bay's index lists the same keys in a different order.

Independently of the reference it also checks what a client would: that every
merkle proof folds into the merkle root inside the header the candidate serves.

With --json, a reference 404/503 where the candidate answered 200 is listed as
REFABSTAIN, not FAIL: Byron Bay 404s an output index past the end of a
transaction where electrs answers {"spent":false}.

The candidate may ABSTAIN without failing: a 503 (index building or catching up
with a reorg) or a 404 where it cannot know (a tx below a pruned witness's
window). Those are counted and listed separately, because an abstention is
exactly what a quorum client is built to handle and a wrong answer is not.

Blocks are sampled from the reference's recent chain (several with more than one
transaction, so merkle branches are non-empty), plus --heights. A height whose
block hash differs between the two is reported as a chain disagreement and its
routes are not compared: they would differ for the right reason.

Prints PASS / FAIL / ABSTAIN lines and a summary; exits 1 on any FAIL.
"""
import hashlib
import json
import sys
import time
import urllib.error
import urllib.request

UA = "verify-swap-routes/1"


def get(base, path, tries=3):
    last = None
    for i in range(tries):
        try:
            req = urllib.request.Request(base + path, headers={"User-Agent": UA})
            r = urllib.request.urlopen(req, timeout=20)
            return r.status, (r.headers.get("Content-Type") or ""), r.read().decode()
        except urllib.error.HTTPError as e:
            return e.code, (e.headers.get("Content-Type") or ""), e.read().decode()
        except Exception as e:  # network trouble: retry, then report it
            last = e
            time.sleep(1 + i)
    return 0, "", "unreachable: %s" % last


def sha256d(b):
    return hashlib.sha256(hashlib.sha256(b).digest()).digest()


def folds(txid, branch, pos, root):
    acc = bytes.fromhex(txid)[::-1]
    for sib in branch:
        s = bytes.fromhex(sib)[::-1]
        acc = sha256d(acc + s) if pos & 1 == 0 else sha256d(s + acc)
        pos >>= 1
    return acc[::-1].hex() == root


def main(argv):
    args = [a for a in argv if not a.startswith("--")]
    as_json = "--json" in argv
    extra = []
    src = "https://api.btxscan.io"
    for flag in ("--heights", "--source"):
        if flag in argv:
            val = argv[argv.index(flag) + 1]
            args = [a for a in args if a != val]
            if flag == "--heights":
                extra = [int(x) for x in val.split(",") if x]
            else:
                src = val.rstrip("/")
    if not args:
        print(__doc__)
        return 2
    cand = args[0].rstrip("/")
    ref = (args[1] if len(args) > 1 else "https://api.btxscan.io").rstrip("/")
    mode = "parsed JSON" if as_json else "byte for byte"
    print("candidate : %s\nreference : %s\nsampled   : %s\ncompare   : %s" % (cand, ref, src, mode))

    counts = {"PASS": 0, "FAIL": 0, "ABSTAIN": 0, "REFABSTAIN": 0}

    def say(kind, msg):
        counts[kind] += 1
        print("%s %s" % (kind, msg))

    c, _, tip = get(src, "/blocks/tip/height")
    if c != 200 or not tip.isdigit():
        print("FAIL the sample source did not answer /blocks/tip/height (%s %s)" % (c, tip[:60]))
        return 1
    tip = int(tip)
    print("source tip %d" % tip)

    # Sample: recent multi-transaction blocks from the reference's listing,
    # ten below the tip, and any heights asked for.
    heights = [tip - 10]
    start = tip - 10
    while len(heights) < 5 and start > tip - 400:
        c, _, body = get(src, "/blocks/%d" % start)
        if c != 200:
            break
        for b in json.loads(body):
            if b.get("tx_count", 1) > 1 and b["height"] not in heights and len(heights) < 5:
                heights.append(b["height"])
        start -= 10
    heights += [h for h in extra if h not in heights]

    for h in heights:
        _, _, sh = get(src, "/block-height/%d" % h)
        rc, _, rh = get(ref, "/block-height/%d" % h)
        cc, _, ch = get(cand, "/block-height/%d" % h)
        if (cc == 200 and ch != sh) or (rc == 200 and rh != sh):
            print("NOTE height %d: candidate %s… reference %s… source %s… — a chain disagreement, not compared"
                  % (h, ch[:16], rh[:16], sh[:16]))
            continue
        bh = sh
        c, _, txids = get(src, "/block/%s/txids" % bh)
        if c != 200:
            print("NOTE height %d: the source does not list its txids (%d); skipped" % (h, c))
            continue
        txids = json.loads(txids)
        print("── height %d, block %s…, %d tx ──" % (h, bh[:16], len(txids)))

        paths = ["/block/%s/header" % bh]
        for t in txids[:8]:
            paths += ["/tx/%s/status" % t, "/tx/%s/hex" % t, "/tx/%s/merkle-proof" % t,
                      "/tx/%s/outspend/0" % t, "/tx/%s/outspend/1" % t]
        header = None
        answered = {}  # txid -> {"200": [routes], "404": [routes]} on the candidate
        for p in paths:
            rc, rt, rb = get(ref, p)
            cc, ct, cb = get(cand, p)
            # A 503 is a witness catching up with a block that just arrived;
            # ask again before counting it as an abstention.
            for wait in (2, 4):
                if cc != 503:
                    break
                time.sleep(wait)
                cc, ct, cb = get(cand, p)
            # status and merkle-proof come from the same index entry, so they
            # must agree; hex may come from a txindex or a block since pruned,
            # and honestly differ from them.
            if (p.endswith("/status") or p.endswith("/merkle-proof")) and cc in (200, 404):
                # {"confirmed":false} (in the mempool) has no proof, honestly.
                if not (p.endswith("/status") and cc == 200 and '"confirmed":true' not in cb):
                    answered.setdefault(p.split("/")[2], {}).setdefault(str(cc), []).append(p.split("/")[3])
            label = p.split("/")[-1] if "/outspend/" not in p else "outspend/" + p.rsplit("/", 1)[1]
            label = "%s %s…" % (label, p.split("/")[2][:12])
            if cc == 0:
                say("FAIL", "%s: the candidate did not answer (%s)" % (label, cb[:80]))
                continue
            if cc == 503:
                say("ABSTAIN", "%s: candidate 503 (%s)" % (label, cb[:60]))
                continue
            if cc == 404 and rc == 200:
                # A pruned witness abstains on what is outside its window. It
                # may never abstain on something it then contradicts, and the
                # 404 bodies are electrs' own words or the witness's one extra.
                if cb in ("Transaction not found", "Transaction not found or is unconfirmed",
                          "Output not known to this witness"):
                    say("ABSTAIN", "%s: candidate 404 '%s', reference 200" % (label, cb))
                else:
                    say("FAIL", "%s: candidate 404 '%s', reference 200" % (label, cb[:60]))
                continue
            if as_json and "/outspend/" in p and rc in (404, 503) and cc == 200:
                # A reference that is not electrs abstaining where the candidate
                # answered: Byron Bay 404s an output index past the end of a
                # transaction, where electrs (and so any candidate matching
                # it) answers {"spent":false}. Not the candidate's fault, and
                # not a value disagreement; listed so it is seen.
                say("REFABSTAIN", "%s: reference %d %r, candidate 200 %r"
                    % (label, rc, rb.strip()[:60], cb[:60]))
                continue
            if as_json and rt.startswith("application/json") and rc == cc == 200:
                try:
                    same = json.loads(cb) == json.loads(rb)
                except ValueError:
                    same = False
            else:
                same = (cc, ct.split(";")[0], cb) == (rc, rt.split(";")[0], rb) if as_json \
                    else (cc, ct, cb) == (rc, rt, rb)
            if same:
                say("PASS", "%s identical (%d, %d bytes)" % (label, cc, len(cb)))
            else:
                say("FAIL", "%s differs: candidate %d %s %r, reference %d %s %r"
                    % (label, cc, ct, cb[:90], rc, rt, rb[:90]))
            if p.endswith("/header") and cc == 200:
                header = cb
            if p.endswith("/merkle-proof") and cc == 200 and header:
                proof = json.loads(cb)
                root = bytes.fromhex(header[72:136])[::-1].hex()
                txid = p.split("/")[2]
                if folds(txid, proof["merkle"], proof["pos"], root):
                    say("PASS", "%s: the candidate's proof folds into its own header's merkle root" % label)
                else:
                    say("FAIL", "%s: the candidate's proof does NOT fold into its header's root" % label)

        # An abstention is honest only if it is consistent: a candidate that
        # answers one route about a txid and 404s another about the same txid
        # holds the transaction and is dropping answers it has.
        for txid, by in answered.items():
            if by.get("200") and by.get("404"):
                say("FAIL", "%s…: candidate answered %s but 404'd %s"
                    % (txid[:12], ",".join(by["200"]), ",".join(by["404"])))

    print("──────────────────────────────────────────────")
    print("PASS %d, FAIL %d, ABSTAIN %d, REFABSTAIN %d (%s)"
          % (counts["PASS"], counts["FAIL"], counts["ABSTAIN"], counts["REFABSTAIN"], mode))
    if counts["PASS"] == 0:
        print("FAIL nothing was compared")
        return 1
    return 1 if counts["FAIL"] else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
