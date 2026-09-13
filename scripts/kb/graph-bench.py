#!/usr/bin/env python3
"""Collect O28 from the production SQLite query on the exact full graph."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]
TRUTH = "eval/truth/kb-graph-synthetic.json"
TRACE = "eval/inputs/kb-neighborhood-observations.json"


def fingerprint(path):
    return {"path": path, "version": "kb-store-v1", "sha256": hashlib.sha256((ROOT / path).read_bytes()).hexdigest()}


def implementation():
    paths = ["Cargo.toml", "Cargo.lock", "examples/kb_graph_bench.rs", "scripts/kb/graph-bench.py", "src/kb/types.rs"]
    paths.extend(str(path.relative_to(ROOT)) for path in sorted((ROOT / "src/kb/store").rglob("*")) if path.suffix in (".rs", ".sql"))
    return [fingerprint(path) for path in paths]


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


def validate(report, truth):
    for key in ("works", "edges", "queries"):
        if report[key] != truth[key]:
            raise ValueError(f"Full graph requires exact {key}={truth[key]}")
    samples = report["samples"]
    if len(samples) != truth["queries"] or not report["fts5"]:
        raise ValueError("Missing samples or FTS5 support")
    if sum(sample["stratum"] == "hub" for sample in samples) != 40:
        raise ValueError("Expected 40 hub queries and 160 uniform queries")
    if sum(sample["stratum"] == "uniform" for sample in samples) != 160:
        raise ValueError("Expected 40 hub queries and 160 uniform queries")
    import math
    for sample in samples:
        if not sample["correct"] or sample["nodes_truncated"] or sample["edges_truncated"]:
            raise ValueError("Incomplete or incorrect neighborhood cannot establish O28")
        if not math.isfinite(sample["milliseconds"]) or sample["milliseconds"] < 0 or sample["nodes"] < 1:
            raise ValueError("Invalid measured sample")
    return [sample["milliseconds"] for sample in samples]


def main():
    truth = json.loads((ROOT / TRUTH).read_text())
    before = implementation()
    cache = Path.home() / ".cache/lysilogy/kb-graph-bench"
    cache.mkdir(parents=True, exist_ok=True)
    data = cache / uuid.uuid4().hex
    environment = dict(os.environ, CARGO_BUILD_JOBS="1", CARGO_PROFILE_DEV_DEBUG="0", CARGO_PROFILE_TEST_DEBUG="0", CARGO_INCREMENTAL="0", CARGO_NET_OFFLINE="true")
    started = time.monotonic()
    run = subprocess.run(["cargo", "run", "--offline", "--release", "--example", "kb_graph_bench", "--", "--data", str(data)], cwd=ROOT, env=environment, text=True, stdout=subprocess.PIPE, check=True)
    report = json.loads(run.stdout)
    values = validate(report, truth)
    if implementation() != before:
        raise ValueError("Implementation changed while benchmark was running")
    report["data_root"] = str(data)
    atomic_json(ROOT / TRACE, report)
    collector = {"schema_version": 1, "suite": "scale", "collector": "kb-neighborhood-v1", "implementation": before,
                 "truth_sets": {"synthetic": {**fingerprint(TRUTH), "version": truth["version"]}},
                 "metrics": {"O28": {"sample": {"method": "p95", "values": values}, "cases": len(values), "evidence": [fingerprint(TRACE)]}},
                 "cost_usd": 0, "wall_seconds": time.monotonic() - started}
    atomic_json(ROOT / "eval/inputs/scale/kb-neighborhood.json", collector)
    print(json.dumps({"O28_p95_ms": sorted(values)[189], "queries": len(values), "data_root": str(data), "wall_seconds": collector["wall_seconds"]}, sort_keys=True))


if __name__ == "__main__":
    main()
