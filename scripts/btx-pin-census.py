#!/usr/bin/env python3
"""BTX pin census: every place across Mende's BTX repos that pins an upstream
btxchain/btx engine version or archive hash, in one read-only report.

Why: one engine bump touches ~25 files in 7 repos, and the same archive sha
(e.g. 9dbaef6f..., btx-0.34.15-x86_64-linux-gnu) sits in at least 5 of them.
A copy that is missed keeps shipping the old engine. See
docs/btx-update-protocol.md (Phase 1, step 1).

What it does, all READ-ONLY:
  1. Asks GitHub for the sha256 digest of every asset on recent btxchain/btx
     releases (and reads each release's SHA384SUMS / SHA256SUMS), so a pinned
     hash can be named: "this is btx-0.34.15-x86_64-linux-gnu.tar.gz".
  2. Reads each repo at origin/main (NOT the working tree: local checkouts here
     are often stale or detached), plus the private ainode repo through the
     GitHub API.
  3. Reports (a) version pins in the known pin files and (b) ANY 64/96-hex
     string in any tracked text file that equals a known release digest, so an
     unlisted copy still shows up.
  4. With --target <tag>: marks each pin as on that tag or not, and exits 1
     if any engine pin is elsewhere. Without --target it only reports.

Usage:
  scripts/btx-pin-census.py                    # report
  scripts/btx-pin-census.py --target v0.34.15  # report + check, exit 1 on drift
  scripts/btx-pin-census.py --fetch            # git fetch each repo first
It never writes to any repo; --fetch only updates remote-tracking refs.
"""
import argparse, base64, json, os, re, subprocess, sys
from collections import defaultdict

REPOS = os.path.expanduser("~/repos")
UPSTREAM = "btxchain/btx"

# Known pin files per repo (paths at origin/main). Version tokens are reported
# only here; release-digest hashes are found everywhere.
PIN_FILES = {
    "easynode": [
        "apps/node/src-tauri/src/commands.rs",
        "apps/node/scripts/stage-node-pkg.sh",
        "apps/node/scripts/stage-node-pkg-linux.sh",
        "apps/node/scripts/stage-node-pkg-linux-source.sh",
        "apps/node/scripts/stage-node-pkg-mac-source.sh",
        "apps/node/scripts/stage-node-pkg-windows.ps1",
        ".github/workflows/node-linux-installer.yml",
        ".github/workflows/node-win-installer.yml",
        ".github/workflows/btxd-windows.yml",
        ".github/workflows/btxd-linux.yml",
        ".github/workflows/btxd-macos.yml",
    ],
    "EasyBTX": [
        "site/src/pages/node.astro",
        "site/public/updater/latest-node.json",
        "site/public/llms.txt",
    ],
    "btx-apps": [
        "apps/web/src/lib/config.ts",
        "apps/web/src/lib/portal/nodes.ts",
        "apps/web/src/lib/portal/stats.ts",
        ".claude/skills/btx-ops/recovery/UPGRADE-btxscan-engine.sh",
    ],
    "btx-esplora": [
        "deploy/btxd.service",
        "deploy/btxd2.service",
        "deploy/btx-supervisor.sh",
        "deploy/btx-supervisor.service",
        "deploy/healthcheck.sh",
    ],
    "btx-mine-hq": [
        "kit/ainode/vast-host.sh",
        "tests/test_ainode.py",
    ],
    "pq-wallet": [
        "tools/regtest/prove-send-in-parts.mjs",
    ],
    "qid": [],
}
# The private AI node repo is not cloned locally; read it through the API.
GH_REPOS = {
    "MendeMatthias/ainode": [
        "host/remote-setup.sh",
        "host/btx-modeld.service",
        "host/RUNBOOK.md",
        "host/vast/vast-host.sh",
        "app/scripts/stage-engine.sh",
        "app/scripts/stage-engine-linux.sh",
        "app/ENGINE-CONTRACT.md",
    ],
}

# v0.34.15, 0.34.15, v0.35.0rc1, v0.35.0-rc1, btx-035-, MIN_ENGINE=0.34.14 ...
VERSION_RE = re.compile(r"(?<![\w.])v?0\.3[0-9]\.\d+(?:-?rc\d+)?(?![\w.])|btx-0\d\d-[a-z0-9_-]+")
HEX_RE = re.compile(r"(?<![0-9a-f])(?:[0-9a-f]{96}|[0-9a-f]{64}|[0-9a-f]{40})(?![0-9a-f])")


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def release_digests(limit):
    """hex -> (tag, asset) for recent upstream releases."""
    out = run(["gh", "api", f"repos/{UPSTREAM}/releases?per_page={limit}"])
    if out.returncode != 0:
        sys.exit(f"error: gh api releases failed: {out.stderr.strip()}")
    known, tags = {}, []
    for rel in json.loads(out.stdout):
        tag = rel["tag_name"]
        tags.append(tag)
        for a in rel.get("assets", []):
            d = (a.get("digest") or "").removeprefix("sha256:")
            if d:
                known[d] = (tag, a["name"])
            if a["name"] in ("SHA384SUMS", "SHA256SUMS"):
                body = run(["gh", "api", "-H", "Accept: application/octet-stream",
                            f"repos/{UPSTREAM}/releases/assets/{a['id']}"])
                for line in body.stdout.splitlines():
                    parts = line.split()
                    if len(parts) >= 2 and HEX_RE.fullmatch(parts[0]):
                        known.setdefault(parts[0], (tag, parts[-1].lstrip("*")))
    # Tag commits too, so NODE_RELEASE_COMMIT / PROVEN_BTXD_* commit pins resolve.
    # From a local btx clone when there is one (cheap), else the tags API.
    clone = os.environ.get("BTX_CLONE", os.path.join(REPOS, "btx"))
    refs = run(["git", "-C", clone, "for-each-ref", "refs/tags",
                "--format=%(objectname) %(*objectname) %(refname:short)"])
    if refs.returncode == 0 and refs.stdout.strip():
        for line in refs.stdout.splitlines():
            parts = line.split()
            commit, tag = (parts[1], parts[2]) if len(parts) == 3 else (parts[0], parts[-1])
            known.setdefault(commit, (tag, "commit"))
    else:
        t = run(["gh", "api", f"repos/{UPSTREAM}/tags?per_page=100"])
        for item in json.loads(t.stdout or "[]"):
            known.setdefault(item["commit"]["sha"], (item["name"], "commit"))
    return known, tags


def local_files(repo, fetch):
    path = os.path.join(REPOS, repo)
    if not os.path.isdir(os.path.join(path, ".git")) and not os.path.isfile(os.path.join(path, ".git")):
        return None, f"not cloned at {path}"
    if fetch:
        run(["git", "-C", path, "fetch", "-q", "origin"])
    ref = "origin/main"
    if run(["git", "-C", path, "rev-parse", "--verify", "-q", ref]).returncode != 0:
        return None, "no origin/main"
    return (path, ref), None


def grep_hex(path, ref):
    """(file, line, text) for every 40/64/96-hex token in tracked text files."""
    out = run(["git", "-C", path, "grep", "-n", "-I", "-E", "[0-9a-f]{40}", ref, "--",
               ":!*.lock", ":!*lock.json", ":!**/target/**", ":!**/node_modules/**"])
    for raw in out.stdout.splitlines():
        try:
            _, f, ln, text = raw.split(":", 3)
        except ValueError:
            continue
        yield f, int(ln), text


def show(path, ref, f):
    out = run(["git", "-C", path, "show", f"{ref}:{f}"])
    return out.stdout if out.returncode == 0 else None


def gh_show(repo, f):
    out = run(["gh", "api", f"repos/{repo}/contents/{f}"])
    if out.returncode != 0:
        return None
    return base64.b64decode(json.loads(out.stdout)["content"]).decode("utf-8", "replace")


COMMENT_RE = re.compile(r"^\s*(//|#|\*|/\*|<!--|--|;)")


def is_doc(f):
    return f.endswith(".md") or f.startswith("docs/") or "/docs/" in f


def scan_text(text, want_versions, known, prose=False):
    """Version tokens on CODE lines only (comments are history, not pins), unless
    the file is prose (a runbook); release-digest hashes on every line."""
    for i, line in enumerate(text.splitlines(), 1):
        code = prose or not COMMENT_RE.match(line)
        vers = VERSION_RE.findall(line) if (want_versions and code) else []
        hexes = [h for h in HEX_RE.findall(line)]
        if vers or any(h in known for h in hexes):
            yield i, line.strip(), vers, [h for h in hexes if h in known], code


def norm(v):
    return v.lstrip("v").replace("-rc", "rc")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--target", help="tag every engine pin should be on, e.g. v0.34.15")
    ap.add_argument("--fetch", action="store_true", help="git fetch each repo first")
    ap.add_argument("--releases", type=int, default=20, help="recent upstream releases to index")
    args = ap.parse_args()

    known, tags = release_digests(args.releases)
    print(f"indexed {len(known)} digests from {len(tags)} upstream releases ({tags[-1]} .. {tags[0]})\n")
    target = norm(args.target) if args.target else None
    drift, rows = 0, 0

    def report(repo, f, ln, line, vers, hexes, history=False):
        nonlocal drift, rows
        rows += 1
        tagz = []
        doc = history or is_doc(f.split("  (")[0])
        for h in hexes:
            t, asset = known[h]
            mark = ""
            if target:
                ok = norm(t) == target or norm(t).startswith(target)
                # A doc naming an old archive is history; only code/config drift fails.
                mark = " OK" if ok else (" old (doc/comment, not counted)" if doc else " DRIFT")
                drift += 0 if (ok or doc) else 1
            tagz.append(f"{h[:8]}…={asset}@{t}{mark}")
        vmark = ""
        if target and vers:
            off = [v for v in vers if not v.startswith("btx-0") and norm(v) != target]
            if off:
                vmark = f"  [versions not {args.target}: {', '.join(sorted(set(off)))}]"
        print(f"  {f}:{ln}  {line[:110]}")
        for t in tagz:
            print(f"      hash {t}")
        if vmark:
            print(f"     {vmark}")

    for repo, files in PIN_FILES.items():
        loc, err = local_files(repo, args.fetch)
        print(f"== {repo}" + (f"  (skipped: {err})" if err else f"  @ {loc[1]}"))
        if err:
            continue
        path, ref = loc
        seen = set()
        for f in files:
            text = show(path, ref, f)
            if text is None:
                print(f"  {f}: (not on {ref})")
                continue
            for ln, line, vers, hexes, code in scan_text(text, True, known, prose=f.endswith('.md')):
                seen.add((f, ln))
                report(repo, f, ln, line, vers, hexes, history=not code)
        # Unlisted copies: any release digest anywhere else in the repo.
        for f, ln, text in grep_hex(path, ref):
            if (f, ln) in seen:
                continue
            hexes = [h for h in HEX_RE.findall(text) if h in known]
            if hexes:
                report(repo, f + "  (unlisted)", ln, text.strip(), [], hexes,
                       history=bool(COMMENT_RE.match(text)))
        print()

    for repo, files in GH_REPOS.items():
        print(f"== {repo}  @ GitHub default branch")
        for f in files:
            text = gh_show(repo, f)
            if text is None:
                print(f"  {f}: (unreadable via gh api)")
                continue
            for ln, line, vers, hexes, code in scan_text(text, True, known, prose=f.endswith('.md')):
                report(repo, f, ln, line, vers, hexes, history=not code or f.endswith('.md'))
        print()

    print(f"{rows} pin lines.", end=" ")
    if target:
        print(f"{drift} release hash(es) not on {args.target}." if drift else f"every release hash is on {args.target}.")
        print("Version tokens flagged above are for a human to judge: floors like MIN_ENGINE and")
        print("history in comments are expected to name other versions.")
        sys.exit(1 if drift else 0)
    print()


if __name__ == "__main__":
    main()
