#!/usr/bin/env python3
"""Copy THIS interpreter's runtime into a Linux engine package's lib directory.

    <python> -I engine-python-bundle.py <swapnet sources> <lib dir> <soname> <linked libpython>

Called by engine-runtime-linux.sh (engine_stage_python), with the interpreter
btxd was built against. From 0.34.16 btxd links libpythonX.Y.so.1.0 for the
in-process SwapNet worker (cmake/SwapNet.cmake). The library must load on every
start, also with -swapnet=0, and -swapnet=1 needs that interpreter's standard
library plus what the frozen SwapNet sources import (cryptography, sqlite3).

What is written, all inside <lib dir> (bin/lib of the package):

    libpythonX.Y.so.1.0                 the library btxd links
    pythonX.Y/                          standard library and lib-dynload
    pythonX.Y/site-packages/            only the third-party modules the SwapNet
                                        sources really load, with their dist-info
                                        (the licence texts are in there)
    pythonX.Y/EASYNODE-RUNTIME.json     what this is and where it came from

btxd finds it without help: src/swapnet/embed.cpp uses the directory above the
loaded libpython as the Python home when that library is in a directory named
`lib` with pythonX.Y/os.py and pythonX.Y/lib-dynload beside it.

This file copies and checks imports. The shared libraries the extension modules
need, the run paths and the glibc floor are engine-runtime-linux.sh's job,
because they are the same job for every other ELF in the package.

The selection follows upstream's scripts/release/bundle_python_runtime.py at
074dac16 on purpose (same excluded directories, same excluded extension
modules, third-party modules found by importing rather than by listing), so a
module the worker can import from upstream's archive can be imported from ours.
"""

from __future__ import annotations

import ast
import hashlib
import importlib
import importlib.metadata
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import sysconfig
from pathlib import Path

MANIFEST = "EASYNODE-RUNTIME.json"
GENERATOR = "apps/node/scripts/lib/engine-python-bundle.py"

# Not used by an embedded worker that has no terminal and no display.
STDLIB_EXCLUDED_DIRS = frozenset({
    "__pycache__", "test", "idlelib", "tkinter", "turtledemo", "ensurepip", "lib2to3",
    "site-packages", "dist-packages",
})
# Static libpython, Makefile and headers for building extensions.
STDLIB_EXCLUDED_PREFIXES = ("config-",)
# Extension modules for a terminal, dbm files, crypt(3) and CPython's own
# tests. Leaving them out keeps readline, ncurses, gdbm and Tk out of the
# package. The SwapNet sources import none of them.
DYNLOAD_EXCLUDED_PREFIXES = (
    "readline.", "_curses.", "_curses_panel.", "_dbm.", "_gdbm.", "_crypt.", "_tkinter.",
    "_test", "_xxtestfuzz.", "xxlimited", "xxsubtype.", "_ctypes_test.", "termios.",
)

# Run in a fresh interpreter whose home is the staged tree: every module the
# worker imports must come from there, and none from the build machine.
CHECK = r"""
import importlib, json, os, sys
root = os.path.realpath(sys.argv[2])
for name in json.loads(sys.argv[1]):
    importlib.import_module(name)
outside = sorted(
    f"{n} {os.path.realpath(m.__file__)}" for n, m in sys.modules.items()
    if getattr(m, "__file__", None) and not os.path.realpath(m.__file__).startswith(root + os.sep))
print(json.dumps({"outside": outside}))
"""


def die(message: str) -> None:
    print(f"engine-python-bundle: {message}", file=sys.stderr)
    sys.exit(1)


def swapnet_imports(package: Path) -> list[str]:
    """Absolute imports of the frozen SwapNet sources, outside the package itself.

    Imports inside functions count too: the worker reaches them after start.
    """
    sources = sorted(package.glob("*.py"))
    if not sources:
        die(f"no SwapNet sources in {package}")
    names: set[str] = set()
    for source in sources:
        tree = ast.parse(source.read_text(encoding="utf-8"), filename=str(source))
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                names.update(alias.name for alias in node.names)
            elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
                names.add(node.module)
    return sorted(name for name in names if name.split(".")[0] != "swapnet")


def copy_file(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)  # follows a symlink: the package holds files only
    destination.chmod(destination.stat().st_mode | 0o200)


def copy_stdlib(source: Path, destination: Path) -> list[str]:
    """Copy the standard library. Returns the symbolic links that were not copied."""
    skipped: list[str] = []
    real_root = source.resolve()
    for directory, subdirs, files in os.walk(source):
        here = Path(directory)
        rel = here.relative_to(source)
        top = rel == Path(".")
        kept = []
        for name in sorted(subdirs):
            if name == "__pycache__":
                continue
            if top and (name in STDLIB_EXCLUDED_DIRS or name.startswith(STDLIB_EXCLUDED_PREFIXES)):
                continue
            if (here / name).is_symlink():  # os.walk does not enter it; say so
                skipped.append(str((here / name).relative_to(source)) + "/")
                continue
            kept.append(name)
        subdirs[:] = kept
        for name in sorted(files):
            item = here / name
            if name.endswith((".pyc", ".pyo")):
                continue
            if rel == Path("lib-dynload") and name.startswith(DYNLOAD_EXCLUDED_PREFIXES):
                continue
            if item.is_symlink() and real_root not in item.resolve().parents:
                skipped.append(str(item.relative_to(source)))  # host policy, not stdlib
                continue
            copy_file(item, destination / rel / name)
    return skipped


def copy_tree(source: Path, destination: Path) -> None:
    if source.is_dir():
        for directory, subdirs, files in os.walk(source):
            subdirs[:] = sorted(d for d in subdirs if d != "__pycache__")
            for name in sorted(files):
                if name.endswith((".pyc", ".pyo")):
                    continue
                item = Path(directory) / name
                copy_file(item, destination / item.relative_to(source))
    else:
        copy_file(source, destination)


def dist_info_dir(dist: importlib.metadata.Distribution, site: Path) -> Path | None:
    path = getattr(dist, "_path", None)
    if path is not None and Path(path).is_dir():
        return Path(path)
    stem = re.sub(r"[-_.]+", "_", dist.metadata["Name"]).lower()
    for candidate in sorted(site.glob("*.dist-info")):
        if candidate.name.lower() == f"{stem}-{dist.version}.dist-info":
            return candidate
    return None


def tree_size(path: Path) -> int:
    return sum(f.stat().st_size for f in path.rglob("*") if f.is_file())


def main(argv: list[str]) -> None:
    if len(argv) != 5:
        die("usage: <python> -I engine-python-bundle.py <swapnet sources> <lib dir> <soname> <linked libpython>")
    swapsrc, libdir, soname, linked = Path(argv[1]), Path(argv[2]), argv[3], Path(argv[4])
    if sys.version_info < (3, 11):
        die(f"Python {platform.python_version()} is older than the 3.11 the SwapNet worker needs")
    if not sys.flags.isolated:
        die("run with -I: the worker runs isolated, and what it can import must be found the same way")

    version = "%d.%d" % sys.version_info[:2]
    inst = sysconfig.get_config_var("INSTSONAME") or ""
    hostlib = Path(sysconfig.get_config_var("LIBDIR") or "") / inst
    if inst != soname:
        die(f"this interpreter is {inst or 'a static build'}; btxd links {soname}")
    if not hostlib.is_file():
        die(f"this interpreter reports {hostlib}, which does not exist")
    if not linked.is_file() or not hostlib.samefile(linked):
        die(f"btxd loads {linked}, which is not {hostlib} of this interpreter")
    if sys.platlibdir != "lib":
        die(f"this interpreter uses platlibdir {sys.platlibdir!r}; btxd looks in lib/python{version}")
    stdlib = Path(os.__file__).resolve().parent
    if not (stdlib / "lib-dynload").is_dir():
        die(f"no lib-dynload directory in {stdlib}")
    site = Path(sysconfig.get_path("purelib")).resolve()
    if Path(sysconfig.get_path("platlib")).resolve() != site or site != stdlib / "site-packages":
        die(f"unexpected site-packages location {site}; this script handles a plain prefix install "
            "(lib/pythonX.Y/site-packages), not a distribution's patched layout")

    # Which third-party modules does the worker really load? Import, then look.
    imports = swapnet_imports(swapsrc)
    before = set(sys.modules)  # site and .pth start-up policy of this machine: never shipped
    failed = []
    for name in imports:
        try:
            importlib.import_module(name)
        except Exception as exc:  # noqa: BLE001 - every failure is reported the same way
            failed.append(f"{name} ({type(exc).__name__}: {exc})")
    if failed:
        die("this interpreter cannot import what the SwapNet sources import: " + "; ".join(failed))
    roots: dict[str, Path] = {}
    for name, module in sorted(sys.modules.items()):
        file = getattr(module, "__file__", None)
        if not file or name in before:
            continue
        path = Path(file).resolve()
        if site in path.parents:
            top = name.split(".")[0]
            top_file = Path(getattr(sys.modules.get(top), "__file__", None) or path).resolve()
            root = top_file.parent if top_file.name.startswith("__init__.") else top_file
            if root.parent != site:
                die(f"{top} is imported from {root}, not from directly inside {site}")
            roots.setdefault(top, root)
        elif stdlib not in path.parents:
            die(f"{name} is imported from {path}, outside this interpreter")

    destination = libdir / f"python{version}"
    if destination.exists() or (libdir / inst).exists():
        die(f"{destination} or {libdir / inst} already exists")
    libdir.mkdir(parents=True, exist_ok=True)
    copy_file(hostlib, libdir / inst)
    skipped = copy_stdlib(stdlib, destination)
    if not (destination / "os.py").is_file() or not (destination / "lib-dynload").is_dir():
        die(f"{destination} has no os.py or no lib-dynload after the copy; btxd would not accept it")

    dest_site = destination / "site-packages"
    dest_site.mkdir()
    owners = importlib.metadata.packages_distributions()
    packages: dict[str, str] = {}
    for top, root in sorted(roots.items()):
        copy_tree(root, dest_site / root.name)
        names = sorted(set(owners.get(top, [])))
        if not names:
            die(f"no installed distribution owns the module {top}; cannot record what is being shipped")
        for dist_name in names:
            dist = importlib.metadata.distribution(dist_name)
            packages[dist.metadata["Name"]] = dist.version
            info = dist_info_dir(dist, site)
            if info is None:
                die(f"cannot find the dist-info directory of {dist_name}; its licence text would be missing")
            if not (dest_site / info.name).exists():
                copy_tree(info, dest_site / info.name)

    manifest = {
        "generator": GENERATOR,
        "python": platform.python_version(),
        "soname": inst,
        "libpython_sha256": hashlib.sha256(hostlib.read_bytes()).hexdigest(),
        "build_prefix": sys.prefix,
        "packages": packages,
        "third_party": sorted(root.name for root in roots.values()),
        "swapnet_imports": imports,
        "stdlib_excluded_dirs": sorted(STDLIB_EXCLUDED_DIRS) + [p + "*" for p in STDLIB_EXCLUDED_PREFIXES],
        "lib_dynload_excluded": [p + "*" for p in DYNLOAD_EXCLUDED_PREFIXES],
        "symlinks_not_copied": skipped,
    }
    (destination / MANIFEST).write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    # Import the same list again from the staged tree alone. btxd reports only
    # the exception class when its worker fails to start, so a missing module
    # is found here, where there is a traceback, and not there.
    home = libdir.parent
    env = {
        "PATH": "/usr/bin:/bin", "LC_ALL": "C.UTF-8",
        "PYTHONHOME": str(home), "PYTHONNOUSERSITE": "1", "PYTHONDONTWRITEBYTECODE": "1",
        "LD_LIBRARY_PATH": str(hostlib.parent),  # only so this interpreter binary finds its own libpython
        "OPENSSL_MODULES": "/nonexistent-engine-python-bundle",
    }
    result = subprocess.run(
        [sys.executable, "-B", "-s", "-P", "-c", CHECK, json.dumps(imports), str(libdir)],
        capture_output=True, text=True, env=env, check=False)
    if result.returncode != 0:
        die("the staged runtime cannot import what the SwapNet sources import:\n" + result.stderr.rstrip())
    outside = json.loads(result.stdout)["outside"]
    if outside:
        die("the staged runtime loaded modules from outside the package: " + "; ".join(outside[:5]))

    print(f"    python {platform.python_version()} ({inst}, {(libdir / inst).stat().st_size} bytes)")
    print(f"    standard library + lib-dynload: {tree_size(destination) - tree_size(dest_site)} bytes, "
          f"{len(list((destination / 'lib-dynload').iterdir()))} extension modules")
    print(f"    site-packages: {tree_size(dest_site)} bytes: "
          + ", ".join(f"{name} {ver}" for name, ver in sorted(packages.items())))
    print(f"    the worker's {len(imports)} imports resolve from the staged tree alone")
    if skipped:
        print(f"    not copied (symbolic links): {', '.join(skipped)}")


if __name__ == "__main__":
    main(sys.argv)
