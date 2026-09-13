#!/usr/bin/env python3
"""Execute only an exact independently reviewed frozen layout snapshot.

Invoke with python3 -I -S -B. Imports of experiment code compile the verified
source bytes directly; adjacent cached bytecode can never replace those bytes.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import stat
import sys
import time
import types

MODULES = ("policy", "confinement", "inputs", "pixels", "runtime", "experiment", "batch")


def raw_file(path, maximum=16 * 1024 ** 2):
    path = Path(path).absolute()
    for component in reversed((path, *path.parents)):
        if component.name in (".env", ".secrets") or component.name.startswith(".env."):
            raise ValueError("forbidden snapshot filename")
        if stat.S_ISLNK(component.lstat().st_mode):
            raise ValueError("snapshot path contains a symlink")
    metadata = path.stat()
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > maximum:
        raise ValueError("expected a bounded regular snapshot file")
    raw = path.read_bytes()
    if len(raw) > maximum:
        raise ValueError("snapshot file grew beyond its bound")
    return raw


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def load_snapshot(directory, launch_sha, review_path, review_sha):
    if not all(re.fullmatch(r"[0-9a-f]{64}", value) for value in (launch_sha, review_sha)):
        raise ValueError("exact launch and independent review hashes are required")
    launch_raw = raw_file(directory / "launch.json")
    if sha(launch_raw) != launch_sha:
        raise ValueError("snapshot launch identity mismatch")
    launch = json.loads(launch_raw)
    if set(launch["confinement_tools"]) != {"bwrap", "unshare"}:
        raise ValueError("unexpected confinement tool inventory")
    if Path(launch["python"]["path"]) != Path(sys.executable).resolve() or launch["python_version"] != sys.version:
        raise ValueError("controller Python identity changed")
    for expected in (launch["python"], *launch["confinement_tools"].values()):
        raw = raw_file(expected["path"])
        if len(raw) != expected["bytes"] or sha(raw) != expected["sha256"]:
            raise ValueError("controller or confinement executable bytes changed")
    if set(launch["modules"]) != set(MODULES):
        raise ValueError("unexpected snapshot module inventory")
    if sha(raw_file(directory / "execute.py")) != launch["bootstrap_sha256"]:
        raise ValueError("snapshot bootstrap identity mismatch")
    review_raw = raw_file(review_path)
    if sha(review_raw) != review_sha:
        raise ValueError("independent review identity mismatch")
    review = json.loads(review_raw)
    if review.get("status") != "clear" or review.get("launch_sha256") != launch_sha or review.get("source_commit") != launch["source_commit"]:
        raise ValueError("review does not clear this exact launch and source")
    raws = {}
    for name in MODULES:
        raw = raw_file(directory / "modules" / (name + ".py"))
        if sha(raw) != launch["modules"][name]:
            raise ValueError("snapshot source identity mismatch")
        raws[name] = raw
    for name in ("selection.json", "runtime.json"):
        if sha(raw_file(directory / name)) != launch["inputs"][name]:
            raise ValueError("frozen experiment input identity mismatch")
    return launch, raws


def install_sources(directory, raws):
    if any(name in sys.modules for name in MODULES):
        raise ValueError("snapshot namespace already contains imported code")
    for name in MODULES:
        filename = str(directory / "modules" / (name + ".py"))
        module = types.ModuleType(name)
        module.__file__ = filename
        module.__package__ = ""
        sys.modules[name] = module
        exec(compile(raws[name], filename, "exec"), module.__dict__)
    return sys.modules["batch"]


def main():
    if not (sys.flags.isolated and sys.flags.no_site and sys.dont_write_bytecode):
        raise ValueError("use python3 -I -S -B for frozen execution")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--launch-sha256", required=True)
    parser.add_argument("--review", type=Path, required=True)
    parser.add_argument("--review-sha256", required=True)
    parser.add_argument("--check-only", action="store_true")
    args = parser.parse_args()
    directory = Path(__file__).absolute().parent
    launch, raws = load_snapshot(directory, args.launch_sha256, args.review, args.review_sha256)
    batch = install_sources(directory, raws)
    if args.check_only:
        print(json.dumps({"status": "verified_without_execution", "launch_sha256": args.launch_sha256}))
        return
    started = time.monotonic()
    record = {"launch_sha256": args.launch_sha256, "review_sha256": args.review_sha256,
              "source_commit": launch["source_commit"], "status": "started"}
    with (directory / "execution-before.json").open("x") as stream:
        json.dump(record, stream, sort_keys=True, indent=2)
    try:
        result = batch.run({"output_parent": str(directory), "confinement_tools": launch["confinement_tools"]}, directory)
        record["status"] = result["status"]
        record["experiment_receipt_sha256"] = sha(raw_file(directory / "experiment/receipt.json"))
    except BaseException as error:
        record["status"] = "failed"
        record["error"] = type(error).__name__ + ": " + str(error)
        raise
    finally:
        record["wall_seconds"] = time.monotonic() - started
        try:
            _, final_raws = load_snapshot(directory, args.launch_sha256, args.review, args.review_sha256)
            record["frozen_inputs_unchanged"] = final_raws == raws
        except Exception as error:
            record["frozen_inputs_unchanged"] = False
            record["preservation_error"] = str(error)
            record["status"] = "failed"
        with (directory / "execution-receipt.json").open("x") as stream:
            json.dump(record, stream, sort_keys=True, indent=2)
            stream.write("\n")
        print(json.dumps(record))
    if record["status"] != "completed" or not record["frozen_inputs_unchanged"]:
        raise ValueError("layout execution failed or frozen inputs changed")


if __name__ == "__main__":
    main()
