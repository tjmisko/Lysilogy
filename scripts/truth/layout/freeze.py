#!/usr/bin/env python3
"""Prepare an immutable candidate snapshot for independent review; run nothing."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys

from execute import MODULES, raw_file
from policy import Refused, binding, regular_path
from confinement import check_tools


def freeze(selection, runtime, destination):
    code = Path(__file__).absolute().parent
    root = code.parents[2]
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    files = {}
    for name in ("execute.py", *(module + ".py" for module in MODULES)):
        path = code / name
        raw = raw_file(path)
        committed = subprocess.check_output(["git", "show", head + ":" + path.relative_to(root).as_posix()], cwd=root)
        if raw != committed:
            raise Refused("commit the exact experiment sources before freezing")
        files[name] = raw
    selection_raw, runtime_raw = raw_file(selection), raw_file(runtime)
    if json.loads(selection_raw).get("deposited_source_executed") is not False:
        raise Refused("selection was not frozen before source execution")
    destination = Path(destination).absolute()
    regular_path(destination.parent, directory=True)
    destination.mkdir(mode=0o700, exist_ok=False)
    (destination / "modules").mkdir(mode=0o700)
    for name, raw in files.items():
        path = destination / "execute.py" if name == "execute.py" else destination / "modules" / name
        with path.open("xb") as stream:
            stream.write(raw)
    for name, raw in (("selection.json", selection_raw), ("runtime.json", runtime_raw)):
        with (destination / name).open("xb") as stream:
            stream.write(raw)
    sha = lambda raw: hashlib.sha256(raw).hexdigest()
    launch = {"schema_version": 1, "issue": 110, "source_commit": head,
              "bootstrap_sha256": sha(files["execute.py"]),
              "confinement_tools": check_tools(), "python": binding(Path(sys.executable).resolve()),
              "python_version": sys.version,
              "modules": {name: sha(files[name + ".py"]) for name in MODULES},
              "inputs": {"selection.json": sha(selection_raw), "runtime.json": sha(runtime_raw)},
              "deposited_source_executed": False, "truth_admission": False}
    with (destination / "launch.json").open("x") as stream:
        json.dump(launch, stream, sort_keys=True, indent=2)
        stream.write("\n")
    return {"path": str(destination / "launch.json"), "sha256": sha(raw_file(destination / "launch.json"))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--selection", required=True, type=Path)
    parser.add_argument("--runtime", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps(freeze(args.selection, args.runtime, args.output)))


if __name__ == "__main__":
    main()
