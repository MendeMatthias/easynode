#!/usr/bin/env python3
"""Every `run:` block in .github/workflows must be valid shell.

WHY THIS EXISTS. On 2026-09-06 `btxd-linux.yml` carried a stray double quote
after an `fi`:

    else
      echo "building $ref (app pins $pinned)"
    fi"

The YAML still parsed, because a `run:` block is an opaque block scalar as far
as YAML is concerned, so every YAML linter said the file was fine. bash did
not: the step died with "unexpected EOF while looking for matching `\"'" at its
second step, before `actions/checkout` of btxchain/btx ever ran. That is the
ONLY CI path that builds the Linux release engine, so it also took down the
pristine-tree gate, the BUILD_GIT_DIRTY gate and the engine-pin check with it,
and it did so on every dispatch, silently, because nobody dispatches a build
they are not already expecting to take an hour.

A syntax error is the cheapest possible class of bug and the most embarrassing
one to ship in a gate. `bash -n` parses without executing, so this costs
milliseconds and needs no runner, no secrets and no network.

WHAT IT DOES NOT DO. It does not run the script, resolve `${{ }}` expressions,
or judge whether the shell is correct — only that it PARSES. GitHub substitutes
expressions before the shell sees them, so a `${{ }}` is replaced here with a
harmless placeholder rather than being left to confuse bash.

Usage:  python3 scripts/check-workflow-shell.py [workflow-dir]
Exit 0 when every run block parses, 1 otherwise, naming file, job and step.
"""
from __future__ import annotations

import os
import re
import subprocess
import sys
import tempfile

try:
    import yaml
except ImportError:  # pragma: no cover - CI installs it
    print("check-workflow-shell: pyyaml is required (pip install pyyaml)", file=sys.stderr)
    raise SystemExit(1)

# GitHub expands ${{ ... }} before the shell runs, so leaving it in would make
# bash judge syntax the shell never sees. A quoted placeholder keeps the
# surrounding quoting intact, which is the thing being checked.
EXPR = re.compile(r"\$\{\{[^}]*\}\}")
PLACEHOLDER = "gh_expr"


def shell_of(step: dict, workflow: dict, job: dict) -> str:
    for src in (step, job.get("defaults", {}).get("run", {}), workflow.get("defaults", {}).get("run", {})):
        sh = (src or {}).get("shell")
        if sh:
            return sh
    return "bash"


def check(path: str) -> list[str]:
    """Return a list of human-readable failures for one workflow file."""
    with open(path, encoding="utf-8") as fh:
        try:
            doc = yaml.safe_load(fh)
        except yaml.YAMLError as e:
            return [f"{path}: YAML does not parse: {e}"]
    if not isinstance(doc, dict):
        return []
    failures: list[str] = []
    for job_name, job in (doc.get("jobs") or {}).items():
        if not isinstance(job, dict):
            continue
        for i, step in enumerate(job.get("steps") or []):
            if not isinstance(step, dict) or "run" not in step:
                continue
            sh = shell_of(step, doc, job)
            # Only shells `bash -n` can speak for. pwsh, python and friends are
            # skipped rather than guessed at.
            if sh not in ("bash", "sh", "bash -e {0}", "bash -eo pipefail {0}"):
                continue
            script = EXPR.sub(PLACEHOLDER, step["run"])
            with tempfile.NamedTemporaryFile("w", suffix=".sh", delete=False, encoding="utf-8") as tmp:
                tmp.write(script)
                tmp_path = tmp.name
            try:
                proc = subprocess.run(["bash", "-n", tmp_path], capture_output=True, text=True)
            finally:
                os.unlink(tmp_path)
            if proc.returncode != 0:
                first = (proc.stderr or "").strip().splitlines()
                detail = first[0].split(": ", 1)[-1] if first else "shell syntax error"
                name = step.get("name", f"step {i}")
                failures.append(f"{path}: job {job_name!r}, step {name!r}: {detail}")
    return failures


def main(argv: list[str]) -> int:
    root = argv[1] if len(argv) > 1 else ".github/workflows"
    if not os.path.isdir(root):
        print(f"check-workflow-shell: no such directory: {root}", file=sys.stderr)
        return 1
    files = sorted(f for f in os.listdir(root) if f.endswith((".yml", ".yaml")))
    if not files:
        print(f"check-workflow-shell: no workflows in {root}", file=sys.stderr)
        return 1
    failures: list[str] = []
    for f in files:
        failures.extend(check(os.path.join(root, f)))
    if failures:
        for line in failures:
            print(f"error: {line}", file=sys.stderr)
        print(f"\ncheck-workflow-shell: {len(failures)} run block(s) do not parse.", file=sys.stderr)
        return 1
    print(f"OK: every run block in {len(files)} workflow(s) parses as shell.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
