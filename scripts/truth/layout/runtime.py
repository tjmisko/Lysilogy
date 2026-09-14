"""Inventory only the installed trusted renderer and PDFLaTeX runtime.

Discovery runs ldd only on two fixed system executables. It never executes a
binary from an archive or metadata, installs a package, or reads a user TeX tree.
"""
import os
from pathlib import Path
import re
import stat
import subprocess

from policy import Refused, binding, regular_path, safe_relative

TEXMF = Path("/usr/share/texlive/texmf-dist")
ENGINE = Path("/usr/bin/pdftex")
RENDERER = Path("/usr/bin/mutool")
FORMAT = Path("/var/lib/texmf/web2c/pdftex/pdflatex.fmt")
CONFIG = TEXMF / "web2c/texmf.cnf"
FONT_MAP = Path("/var/lib/texmf/fonts/map/pdftex/updmap/pdftex.map")


def dependency_mounts(executable):
    if executable not in (ENGINE, RENDERER):
        raise Refused("dependency discovery accepts only fixed trusted executables")
    executable = executable.resolve(strict=True)
    regular_path(executable)
    result = subprocess.run(["/usr/bin/ldd", str(executable)], env={"PATH": "/usr/bin", "LC_ALL": "C"},
                            capture_output=True, text=True, timeout=10)
    if result.returncode or "not found" in result.stdout:
        raise Refused("trusted runtime dependency is unavailable")
    mounts = {}
    for line in result.stdout.splitlines():
        line = line.strip()
        if not line or line.startswith("linux-vdso.so."):
            continue
        match = re.fullmatch(r"(?:[^\s]+ => )?(/(?:usr/)?lib(?:64)?/[^\s]+) \(0x[0-9a-f]+\)", line)
        if match is None:
            raise Refused("unrecognized trusted dependency listing")
        destination = match[1]
        safe_relative(destination[1:])
        source = Path(destination).resolve(strict=True)
        regular_path(source)
        mounts[destination] = source
    if "/lib/ld-linux-aarch64.so.1" not in mounts:
        raise Refused("expected native dynamic loader is unavailable")
    return sorted((str(source), destination) for destination, source in mounts.items())


def tree_inventory(root):
    root = regular_path(root, directory=True)
    entries = []
    for base, dirs, files in os.walk(root, followlinks=False):
        dirs.sort()
        files.sort()
        for name in dirs + files:
            safe_relative(name)  # Refuse forbidden names without opening them.
            path = Path(base) / name
            relative = path.relative_to(root).as_posix()
            mode = path.lstat().st_mode
            if stat.S_ISLNK(mode):
                # The link itself is mounted as part of the read-only runtime.
                # Its outside target is not mounted or opened for this inventory.
                entries.append({"name": relative, "kind": "symlink", "target": os.readlink(path)})
            elif stat.S_ISREG(mode):
                entries.append({"name": relative, "kind": "regular", **binding(path)})
            elif stat.S_ISDIR(mode):
                entries.append({"name": relative, "kind": "directory"})
            else:
                raise Refused("trusted runtime contains a special filesystem object")
    return sorted(entries, key=lambda item: item["name"])


def discover():
    engine_mounts = [(str(ENGINE.resolve(strict=True)), "/runtime/bin/pdftex"),
                     (str(TEXMF), "/runtime/texmf"),
                     (str(FORMAT.resolve(strict=True)), "/runtime/format/pdflatex.fmt"),
                     (str(CONFIG.resolve(strict=True)), "/runtime/config/texmf.cnf"),
                     (str(FONT_MAP.resolve(strict=True)), "/runtime/maps/pdftex.map"),
                     *dependency_mounts(ENGINE)]
    render_mounts = [(str(RENDERER.resolve(strict=True)), "/runtime/bin/mutool"),
                     *dependency_mounts(RENDERER)]
    paths = sorted({source for source, _ in engine_mounts + render_mounts if Path(source).is_file()})
    return {"schema_version": 1, "engine_mounts": engine_mounts, "render_mounts": render_mounts,
            "files": [binding(path) for path in paths], "texmf_root": str(TEXMF),
            "texmf_inventory": tree_inventory(TEXMF), "network_calls": 0, "model_calls": 0,
            "deposited_source_executed": False}


def verify(record, *, full_tree=True):
    for expected in record["files"]:
        if binding(expected["path"]) != expected:
            raise Refused("trusted runtime bytes changed")
    if full_tree and tree_inventory(Path(record["texmf_root"])) != record["texmf_inventory"]:
        raise Refused("trusted runtime tree changed")
    return True
