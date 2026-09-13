#!/usr/bin/env python3
"""Measure G4 using genuine K2 records; missing truth stays unavailable."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]
OUTPUT = ROOT / "eval/inputs/resolution/kb-rebuild.json"
TRACE = "eval/inputs/kb-rebuild-observations.json"


def fingerprint(path):
    return {"path": path, "version": "kb-rebuild-v1", "sha256": hashlib.sha256((ROOT / path).read_bytes()).hexdigest()}


def atomic_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, prefix=path.name + ".", delete=False) as stream:
        temporary = Path(stream.name)
        try:
            json.dump(value, stream, indent=2, sort_keys=True)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
            os.replace(temporary, path)
        finally:
            temporary.unlink(missing_ok=True)


def validate(report):
    if report.get("truth_id") != "K2" or report.get("source_records", 0) < 1 or report.get("deposited_pairs", 0) < 1:
        raise ValueError("G4 needs nonempty K2 source records and deposited DOI pairs")
    if report.get("summary", {}).get("works", 0) < 1 or report["summary"].get("aliases", 0) < 1:
        raise ValueError("G4 must exercise actual entities and alias replay")
    hashes = [report.get(key) for key in ("initial_sha256", "transactional_sha256", "deleted_sha256")]
    if not report.get("identical") or any(not isinstance(value, str) or len(value) != 64 for value in hashes) or len(set(hashes)) != 1:
        raise ValueError("Rebuild results differ or their evidence is incomplete")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--truth", default="eval/truth/reference-resolution.json")
    args = parser.parse_args()
    truth = (ROOT / args.truth).resolve()
    if not truth.is_relative_to(ROOT / "eval/truth") or truth.suffix != ".json":
        raise ValueError("K2 truth must be a repository eval/truth JSON file")
    if not truth.exists():
        OUTPUT.unlink(missing_ok=True)
        print(json.dumps({"G4": "unavailable", "reason": "K2 reference truth does not exist yet"}))
        return
    paths = ["Cargo.toml", "Cargo.lock", "src/kb/types.rs", "examples/kb_rebuild_eval.rs", "scripts/kb/rebuild-eval.py"]
    paths.extend(str(path.relative_to(ROOT)) for path in sorted((ROOT / "src/kb/store").rglob("*")) if path.suffix in (".rs", ".sql"))
    before = [fingerprint(path) for path in paths]
    cache = Path.home() / ".cache/lysilogy/kb-rebuild-eval"
    cache.mkdir(parents=True, exist_ok=True)
    data = cache / uuid.uuid4().hex
    environment = dict(os.environ, CARGO_BUILD_JOBS="1", CARGO_PROFILE_DEV_DEBUG="0", CARGO_PROFILE_TEST_DEBUG="0", CARGO_INCREMENTAL="0", CARGO_NET_OFFLINE="true")
    started = time.monotonic()
    run = subprocess.run(["cargo", "run", "--offline", "--example", "kb_rebuild_eval", "--", "--truth", str(truth), "--data", str(data)], cwd=ROOT, env=environment, text=True, stdout=subprocess.PIPE, check=True)
    report = json.loads(run.stdout)
    validate(report)
    if before != [fingerprint(path) for path in paths]:
        raise ValueError("Implementation changed during rebuild measurement")
    report["data_root"] = str(data)
    atomic_json(ROOT / TRACE, report)
    collector = {"schema_version": 1, "suite": "resolution", "collector": "kb-rebuild-v1", "implementation": before,
                 "truth_sets": {"K2": {**fingerprint(str(truth.relative_to(ROOT))), "version": report["truth_version"]}},
                 "metrics": {"G4": {"sample": {"method": "ratio", "numerator": report["source_records"], "denominator": report["source_records"]}, "cases": report["source_records"], "evidence": [fingerprint(TRACE)]}},
                 "cost_usd": 0, "wall_seconds": time.monotonic() - started}
    atomic_json(OUTPUT, collector)
    print(json.dumps({"G4": 1, "source_records": report["source_records"], "deposited_pairs": report["deposited_pairs"], "data_root": str(data)}))


if __name__ == "__main__":
    main()
