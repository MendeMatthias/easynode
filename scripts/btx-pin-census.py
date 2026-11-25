#!/usr/bin/env python3
"""BTX pin census: every place across Mende's BTX repos that pins an upstream
btxchain/btx engine version, commit or archive hash, in one read-only report.

Why: one engine bump touches ~25 files in 8 repos, and the same archive sha
(e.g. 9dbaef6f..., btx-0.34.15-x86_64-linux-gnu) sits in at least 5 of them.
A copy that is missed keeps shipping the old engine. See
docs/btx-update-protocol.md (Phase 1, step 1).

What it does, all READ-ONLY:
  1. Asks GitHub for the sha256 digest of every asset on recent btxchain/btx
     releases (and reads each release's SHA384SUMS / SHA256SUMS), so a pinned
     hash can be named: "this is btx-0.34.15-x86_64-linux-gnu.tar.gz".
  2. Asks UPSTREAM which commit each tag names (git ls-remote), and compares
     that with the local btx clone. A tag can move: upstream cut v0.34.16 at
     d27530af and moved it to 074dac16, and a clone that fetched it before
     the move still answers d27530af. The moved tag is reported, and a pin on
     the commit a tag USED to name counts as drift.
  3. Reads each repo at origin/main (NOT the working tree: local checkouts
     are often stale or detached), and the two AInode repos through the
     GitHub API: the public MendeMatthias/ainode holds the app's engine pins,
     the private MendeMatthias/ainode-private holds the host's (`host/`).
  4. Reports (a) version pins in the known pin files and (b) ANY 64/96-hex
     string in any tracked text file that equals a known release digest, so an
     unlisted copy still shows up.
  5. With --target <tag>: marks each pin as on that tag or not, ends with the
     list of what still has to move, and exits 1 if any hash or commit pin is
     elsewhere. Without --target it only reports.
  6. With --target it FAILS CLOSED on what it could not read. A pin file that
     did not read (a private repo without a token, a file that moved) and a
     repo that is not cloned are named in the closing list and make the exit
     code 1, like a drifted pin: "nothing listed" must never mean "nothing
     looked at". --allow-missing accepts repos that are not cloned on this
     machine (they stay named); nothing accepts an unreadable pin file.

Usage:
  scripts/btx-pin-census.py                    # report
  scripts/btx-pin-census.py --target v0.34.16  # report + what still has to move
  scripts/btx-pin-census.py --fetch            # git fetch each repo first
  scripts/btx-pin-census.py --target v0.34.16 --repo easynode=.@HEAD
                                               # this checkout's branch, not origin/main
  scripts/btx-pin-census.py --target v0.34.16 --allow-missing
                                               # on a machine without every clone
Exit codes with --target: 0 every pin is on the tag and everything was read;
1 a pin is elsewhere, or something was not read. The last line says which.
It never writes to any repo; --fetch only updates remote-tracking refs.

GitHub access: `gh api` when gh is installed. Otherwise curl, with a token
from $GH_TOKEN / $GITHUB_TOKEN or from `git credential fill` (never printed);
without any token the public repos still read and the private one is listed
as unreadable. Local clones are looked up under $BTX_REPOS (default ~/repos).
"""
import argparse, base64, json, os, re, shutil, subprocess, sys
from collections import defaultdict

REPOS = os.path.expanduser(os.environ.get("BTX_REPOS", "~/repos"))
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
# AInode is read through the API, at each repo's default branch. It is two
# repos since 2026-11-24: `ainode` went public and carries the app, without
# `host/`; the model host's files stayed in the private `ainode-private`.
# Reading `host/` from `ainode` answers 404 for every file, which looked like
# "nothing pinned" while remote-setup.sh and vast-host.sh sat on the old engine.
GH_REPOS = {
    "MendeMatthias/ainode": [
        "app/scripts/stage-engine.sh",
        "app/scripts/stage-engine-linux.sh",
        "app/scripts/stage-engine-windows.sh",
        "app/scripts/build-engine-macos.sh",
        "app/ENGINE-CONTRACT.md",
        "scripts/upstream-check.sh",
    ],
    "MendeMatthias/ainode-private": [
        "host/remote-setup.sh",
        "host/btx-modeld.service",
        "host/RUNBOOK.md",
        "host/vast/vast-host.sh",
    ],
}

# v0.34.15, 0.34.15, v0.35.0rc1, v0.35.0-rc1, btx-035-, MIN_ENGINE=0.34.14 ...
VERSION_RE = re.compile(r"(?<![\w.])v?0\.3[0-9]\.\d+(?:-?rc\d+)?(?![\w.])|btx-0\d\d-[a-z0-9_-]+")
HEX_RE = re.compile(r"(?<![0-9a-f])(?:[0-9a-f]{96}|[0-9a-f]{64}|[0-9a-f]{40})(?![0-9a-f])")
# A line that pins an upstream COMMIT by name. A 40-hex value on such a line
# that no upstream tag names is reported too: that is what a pin on a moved or
# deleted tag looks like, and what a build from an untagged commit looks like.
COMMIT_PIN_RE = re.compile(r"NODE_RELEASE_COMMIT|PROVEN_BTXD_(COMMIT|ARTIFACT)|BTXCHAIN_REF|btxchain_ref|BTX_COMMIT")


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8",
                          errors="replace", **kw)


_TOKEN = None


def token():
    """A GitHub token for curl, or "". Never printed, never written anywhere."""
    global _TOKEN
    if _TOKEN is None:
        _TOKEN = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN") or ""
        if not _TOKEN:
            out = run(["git", "credential", "fill"], input="protocol=https\nhost=github.com\n\n",
                      env={**os.environ, "GIT_TERMINAL_PROMPT": "0"})
            m = re.search(r"^password=(.+)$", out.stdout, re.M)
            _TOKEN = m.group(1) if m else ""
    return _TOKEN


def api(path, accept="application/vnd.github+json"):
    """GET api.github.com/<path>; the body, or None when it did not answer 2xx."""
    if shutil.which("gh"):
        out = run(["gh", "api", "-H", f"Accept: {accept}", path])
        return out.stdout if out.returncode == 0 else None
    cmd = ["curl", "-sS", "-L", "--fail", "-H", f"Accept: {accept}",
           "-H", "X-GitHub-Api-Version: 2022-11-28"]
    if token():
        # -K - keeps the token out of the argument list.
        cmd += ["-K", "-"]
        out = run(cmd + [f"https://api.github.com/{path}"],
                  input=f'header = "Authorization: Bearer {token()}"\n')
    else:
        out = run(cmd + [f"https://api.github.com/{path}"])
    return out.stdout if out.returncode == 0 else None


def upstream_tags():
    """tag -> commit, as UPSTREAM answers right now. {} when it cannot be asked."""
    out = run(["git", "ls-remote", "--tags", f"https://github.com/{UPSTREAM}"],
              env={**os.environ, "GIT_TERMINAL_PROMPT": "0"})
    tags = {}
    for line in out.stdout.splitlines():
        sha, _, ref = line.partition("\t")
        if not ref.startswith("refs/tags/"):
            continue
        name = ref[len("refs/tags/"):]
        if name.endswith("^{}"):
            tags[name[:-3]] = sha  # the commit an annotated tag points at
        else:
            tags.setdefault(name, sha)
    if not tags:
        body = api(f"repos/{UPSTREAM}/tags?per_page=100")
        for item in json.loads(body or "[]"):
            tags[item["name"]] = item["commit"]["sha"]
    return tags


def local_tags():
    """tag -> commit in the local btx clone, or {} when there is none."""
    clone = os.environ.get("BTX_CLONE", os.path.join(REPOS, "btx"))
    refs = run(["git", "-C", clone, "for-each-ref", "refs/tags",
                "--format=%(objectname) %(*objectname) %(refname:short)"])
    tags = {}
    if refs.returncode == 0:
        for line in refs.stdout.splitlines():
            parts = line.split()
            if len(parts) == 3:
                tags[parts[2]] = parts[1]
            elif len(parts) == 2:
                tags[parts[1]] = parts[0]
    return tags


def release_digests(limit):
    """hex -> (tag, asset) for recent upstream releases, plus the moved tags."""
    body = api(f"repos/{UPSTREAM}/releases?per_page={limit}")
    if body is None:
        sys.exit("error: could not list upstream releases (gh api / curl failed)")
    known, tags = {}, []
    for rel in json.loads(body):
        tag = rel["tag_name"]
        tags.append(tag)
        for a in rel.get("assets", []):
            d = (a.get("digest") or "").removeprefix("sha256:")
            if d:
                known[d] = (tag, a["name"])
            if a["name"] in ("SHA384SUMS", "SHA256SUMS"):
                sums = api(f"repos/{UPSTREAM}/releases/assets/{a['id']}",
                           accept="application/octet-stream")
                for line in (sums or "").splitlines():
                    parts = line.split()
                    if len(parts) >= 2 and HEX_RE.fullmatch(parts[0]):
                        known.setdefault(parts[0], (tag, parts[-1].lstrip("*")))
    # Tag commits too, so NODE_RELEASE_COMMIT / PROVEN_BTXD_* commit pins
    # resolve. Upstream is asked first: it is the only one that knows where a
    # tag points TODAY. The local clone only adds what upstream did not answer,
    # and shows which of its tags have moved since it fetched them.
    remote, local, moved = upstream_tags(), local_tags(), []
    for tag, commit in remote.items():
        known.setdefault(commit, (tag, "commit"))
    for tag, commit in local.items():
        now = remote.get(tag)
        if now and now != commit:
            moved.append((tag, commit, now))
            known.setdefault(commit, (tag, "commit BEFORE upstream moved the tag"))
        elif not now:
            known.setdefault(commit, (tag, "commit" if not remote else "commit (local tag, not upstream)"))
    return known, tags, moved, bool(remote)


def local_files(repo, fetch, override):
    path, ref = os.path.join(REPOS, repo), "origin/main"
    if repo in override:
        path, _, wanted = override[repo].partition("@")
        path, ref = os.path.expanduser(path), wanted or ref
    if not os.path.isdir(os.path.join(path, ".git")) and not os.path.isfile(os.path.join(path, ".git")):
        return None, f"not cloned at {path}"
    if fetch:
        run(["git", "-C", path, "fetch", "-q", "origin"])
    if run(["git", "-C", path, "rev-parse", "--verify", "-q", ref]).returncode != 0:
        return None, f"no {ref}"
    return (path, ref), None


def grep_hex(path, ref):
    """(file, line, text) for every 40/64/96-hex token in tracked text files."""
    out = run(["git", "-C", path, "grep", "-n", "-I", "-E", "[0-9a-f]{40}", ref, "--",
               ":!*.lock", ":!*lock.json", ":!**/target/**", ":!**/node_modules/**"])
    for raw in out.stdout.splitlines():
        # <ref>:<file>:<line>:<text>; a ref such as origin/main holds no colon.
        rest = raw[len(ref) + 1:] if raw.startswith(ref + ":") else raw
        try:
            f, ln, text = rest.split(":", 2)
            yield f, int(ln), text
        except ValueError:
            continue


def show(path, ref, f):
    out = run(["git", "-C", path, "show", f"{ref}:{f}"])
    return out.stdout if out.returncode == 0 else None


def gh_show(repo, f):
    body = api(f"repos/{repo}/contents/{f}")
    if body is None:
        return None
    try:
        return base64.b64decode(json.loads(body)["content"]).decode("utf-8", "replace")
    except (ValueError, KeyError, TypeError):
        return None


COMMENT_RE = re.compile(r"^\s*(//|#|\*|/\*|<!--|--|;)")


def is_doc(f):
    return f.endswith(".md") or f.startswith("docs/") or "/docs/" in f


def scan_text(text, want_versions, known, prose=False):
    """Version tokens on CODE lines only (comments are history, not pins), unless
    the file is prose (a runbook); release-digest hashes on every line; and a
    40-hex value no tag names on a code line that pins a commit by name."""
    for i, line in enumerate(text.splitlines(), 1):
        code = prose or not COMMENT_RE.match(line)
        vers = VERSION_RE.findall(line) if (want_versions and code) else []
        hexes = HEX_RE.findall(line)
        named = [h for h in hexes if h in known]
        unnamed = []
        if want_versions and code and not prose and COMMIT_PIN_RE.search(line):
            unnamed = [h for h in hexes if len(h) == 40 and h not in known]
        if vers or named or unnamed:
            yield i, line.strip(), vers, named, unnamed, code


def norm(v):
    return v.lstrip("v").replace("-rc", "rc")


def verdict(target, drift, unreadable, not_cloned, allow_missing):
    # (exit code, last line) of a --target run. Fails closed: a pin file that
    # was not read counts like a pin that did not move, and so does a repo
    # that is not cloned unless --allow-missing was passed.
    reasons = []
    if drift:
        reasons.append(f"{drift} pin(s) not on {target}")
    if unreadable:
        reasons.append(f"{unreadable} pin file(s) not read")
    if not_cloned and not allow_missing:
        reasons.append(f"{not_cloned} repo(s) not cloned (pass --allow-missing to accept)")
    if reasons:
        return 1, f"FAIL (exit 1): {'; '.join(reasons)}."
    skipped = f" {not_cloned} repo(s) not cloned here were NOT checked." if not_cloned else ""
    return 0, f"OK (exit 0): everything read is on {target}.{skipped}"


def main():
    # A Windows console is cp1252 by default and prints "…" as a replacement
    # character, or dies on it when the output is piped.
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--target", help="tag every engine pin should be on, e.g. v0.34.16")
    ap.add_argument("--fetch", action="store_true", help="git fetch each repo first")
    ap.add_argument("--releases", type=int, default=20, help="recent upstream releases to index")
    ap.add_argument("--repo", action="append", default=[], metavar="NAME=PATH[@REF]",
                    help="read this repo from PATH (and REF) instead of $BTX_REPOS/NAME at "
                         "origin/main, e.g. easynode=.@HEAD for the branch checked out here")
    ap.add_argument("--allow-missing", action="store_true",
                    help="with --target: do not fail for a repo that is not cloned here "
                         "(it is still named as not checked)")
    args = ap.parse_args()
    override = dict(r.split("=", 1) for r in args.repo if "=" in r)

    known, tags, moved, asked_upstream = release_digests(args.releases)
    print(f"indexed {len(known)} digests from {len(tags)} upstream releases ({tags[-1]} .. {tags[0]})")
    if not asked_upstream:
        print("WARNING: upstream did not answer for its tags; tag commits come from the local clone,")
        print("         which cannot know about a tag that moved after it was fetched.")
    for tag, was, now in moved:
        print(f"MOVED TAG: upstream's {tag} is {now[:8]} now; the local btx clone still has {was[:8]}.")
        print(f"           Pin the commit, never the tag name, and `git fetch --tags --force` the clone.")
    print()
    target = norm(args.target) if args.target else None
    drift, rows = 0, 0
    todo = defaultdict(list)       # repo -> hash and commit pins not on the target (counted)
    judge = defaultdict(list)      # repo -> version tokens not on the target (a human decides)

    def report(repo, f, ln, line, vers, hexes, unnamed=(), history=False):
        nonlocal drift, rows
        rows += 1
        tagz = []
        doc = history or is_doc(f.split("  (")[0])
        where = f"{f.split('  (')[0]}:{ln}"
        for h in hexes:
            t, asset = known[h]
            mark = ""
            if target:
                # Exact: 0.34.1 is not 0.34.16, and the commit a tag named
                # before upstream moved it is not that tag.
                ok = norm(t) == target and "BEFORE" not in asset
                # A doc naming an old archive is history; only code/config drift fails.
                mark = " OK" if ok else (" old (doc/comment, not counted)" if doc else " DRIFT")
                if not (ok or doc):
                    drift += 1
                    todo[repo].append(f"{where}  {h[:8]}… is {asset} of {t}")
            tagz.append(f"{h[:8]}…={asset}@{t}{mark}")
        for h in unnamed:
            mark = ""
            if target:
                mark = " DRIFT"
                drift += 1
                todo[repo].append(f"{where}  {h[:8]}… is a commit no upstream tag names")
            tagz.append(f"{h[:8]}…=a commit no upstream tag names{mark}")
        vmark = ""
        if target and vers:
            off = sorted({v for v in vers if not v.startswith("btx-0") and norm(v) != target})
            if off:
                vmark = f"  [versions not {args.target}: {', '.join(off)}]"
                if not doc:
                    judge[repo].append(f"{where}  {', '.join(off)}")
        print(f"  {f}:{ln}  {line[:110]}")
        for t in tagz:
            print(f"      hash {t}")
        if vmark:
            print(f"     {vmark}")

    not_cloned = []                # repos with no local clone: nothing in them was checked
    unreadable = []                # pin files that did not read
    for repo, files in PIN_FILES.items():
        loc, err = local_files(repo, args.fetch, override)
        print(f"== {repo}" + (f"  (skipped: {err})" if err else f"  @ {loc[1]}"))
        if err:
            not_cloned.append(f"{repo}: {err}")
            continue
        path, ref = loc
        seen = set()
        for f in files:
            text = show(path, ref, f)
            if text is None:
                print(f"  {f}: (not on {ref})")
                unreadable.append(f"{repo}:{f} (not on {ref})")
                continue
            for ln, line, vers, hexes, unnamed, code in scan_text(text, True, known, prose=f.endswith('.md')):
                seen.add((f, ln))
                report(repo, f, ln, line, vers, hexes, unnamed, history=not code)
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
                print(f"  {f}: (unreadable via the GitHub API)")
                unreadable.append(f"{repo}:{f}")
                continue
            for ln, line, vers, hexes, unnamed, code in scan_text(text, True, known, prose=f.endswith('.md')):
                report(repo, f, ln, line, vers, hexes, unnamed, history=not code or f.endswith('.md'))
        print()

    print(f"{rows} pin lines.", end=" ")
    if not target:
        print()
        return
    print(f"{drift} hash or commit pin(s) not on {args.target}." if drift
          else f"every hash and commit pin that was read is on {args.target}.")
    if todo or judge or unreadable or not_cloned:
        print(f"\nSTILL TO MOVE to {args.target}")
    for repo in list(PIN_FILES) + list(GH_REPOS):
        if repo not in todo and repo not in judge:
            continue
        print(f"  {repo}")
        for line in todo.get(repo, []):
            print(f"    pin      {line}")
        for line in judge.get(repo, []):
            print(f"    version  {line}")
    if unreadable:
        print("  NOT READ, so not checked (a private repo needs a token; otherwise the file moved):")
        for f in unreadable:
            print(f"    {f}")
    if not_cloned:
        print("  NOT CLONED here, so not checked at all:")
        for r in not_cloned:
            print(f"    {r}")
    print("\n`pin` lines are hashes and commits on another release: they are counted, and each one")
    print("has to move. `version` lines are version tokens on code lines of the pin files, for a")
    print("human to judge: a floor like MIN_ENGINE or an example in a test names other versions on")
    print("purpose. Comments and docs are history and are not listed.")
    code, line = verdict(args.target, drift, len(unreadable), len(not_cloned), args.allow_missing)
    print("\n" + line)
    sys.exit(code)


if __name__ == "__main__":
    main()
