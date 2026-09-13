#!/usr/bin/env python3
"""Measure G4 using genuine K2 records; missing truth stays unavailable."""
import argparse
import hashlib
import importlib.util
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


def load_truth_api():
    path = ROOT / "scripts/truth/reference_truth.py"
    if not path.is_file():
        return None
    spec = importlib.util.spec_from_file_location("kb_reference_truth", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def prepare_input(truth, snapshots, api):
    """Reconstruct K2 from independent frozen records before using them in G4."""
    if truth.get("schema_version") != 1 or truth.get("truth_set") != "K2":
        raise ValueError("G4 requires the actual versioned E8.4 K2 shape")
    expected = truth.get("sources")
    if not isinstance(expected, list) or not expected:
        raise ValueError("K2 requires frozen source provenance")
    selected = [item for item in snapshots if item["receipt"]["provider"] == "crossref"]
    sources = [api.source_record(item, truth["built_at"]) for item in sorted(selected, key=lambda item: item["snapshot_id"])]
    if sources != expected:
        raise ValueError("Frozen Crossref sources differ from K2")
    seeds = [origin for work in truth["works"] for origin in work["origins"]]
    # Missing works have only their requested DOI retained, and contribute no raw record.
    seeds += [{"doi": value} for value in truth["coverage"].get("missing_seed_dois", [])]
    rebuilt = api.build_k2(selected, seeds, truth["built_at"])
    if rebuilt != truth:
        raise ValueError("K2 differs from deterministic frozen-source reconstruction")
    return {"schema_version": 1, "kind": "kb_rebuild_input", "truth_id": "K2",
            "version": truth["version"], "truth_sha256": api.fingerprint(truth), "sources": sources,
            "records": [{"retrieved_at": item["receipt"]["retrieved_at"],
                         "snapshot_id": item["snapshot_id"], "crossref": item["payload"]["message"],
                         "doi": api.doi(item["payload"]["message"]["DOI"]),
                         "deposited_references": [{"doi": case["expected_doi"], "case_id": case["case_id"]}
                                                  for case in truth["cases"] if case["snapshot_id"] == item["snapshot_id"]]}
                        for item in sorted(selected, key=lambda item: item["snapshot_id"])]}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--truth", default="eval/truth/reference-resolution.json")
    parser.add_argument("--frozen-root", type=Path, default=Path.home() / ".cache/lysilogy/reference-truth")
    parser.add_argument("--manifest", type=Path)
    args = parser.parse_args()
    truth = (ROOT / args.truth).resolve()
    if not truth.is_relative_to(ROOT / "eval/truth") or truth.suffix != ".json":
        raise ValueError("K2 truth must be a repository eval/truth JSON file")
    if not truth.exists():
        OUTPUT.unlink(missing_ok=True)
        print(json.dumps({"G4": "unavailable", "reason": "K2 reference truth does not exist yet"}))
        return
    loader_path = "scripts/truth/reference_truth.py"
    loader_before = fingerprint(loader_path) if (ROOT / loader_path).is_file() else None
    api = load_truth_api()
    if api is None:
        OUTPUT.unlink(missing_ok=True)
        print(json.dumps({"G4": "unavailable", "reason": "E8.4 frozen truth loader is not available in this checkout"}))
        return
    if args.manifest is None:
        raise ValueError("G4 requires the exact frozen --manifest for K2")
    original_truth = truth.read_bytes()
    manifest_bytes = args.manifest.read_bytes()
    def prepared():
        return prepare_input(api.read_json(truth), api.load_snapshots(args.frozen_root, api.read_json(args.manifest)), api)
    verified_input = prepared()
    paths = ["scripts/truth/reference_truth.py", "src/kb/titles.rs", "src/citation_graph/mod.rs", "Cargo.toml", "Cargo.lock", "src/kb/types.rs", "examples/kb_rebuild_eval.rs", "scripts/kb/rebuild-eval.py"]
    paths.extend(str(path.relative_to(ROOT)) for path in sorted((ROOT / "src/kb/store").rglob("*")) if path.suffix in (".rs", ".sql"))
    before = [fingerprint(path) for path in paths]
    if before[0] != loader_before:
        raise ValueError("Frozen truth loader changed during validation")
    cache = Path.home() / ".cache/lysilogy/kb-rebuild-eval"
    cache.mkdir(parents=True, exist_ok=True)
    run_root = cache / uuid.uuid4().hex
    run_root.mkdir()
    data = run_root / "data"
    input_path = run_root / "verified-input.json"
    if len(api.canonical(verified_input).encode()) > api.MAX_AGGREGATE_BYTES:
        raise ValueError("Verified G4 input exceeds the frozen aggregate bound")
    atomic_json(input_path, verified_input)
    environment = dict(os.environ, CARGO_BUILD_JOBS="1", CARGO_PROFILE_DEV_DEBUG="0", CARGO_PROFILE_TEST_DEBUG="0", CARGO_INCREMENTAL="0", CARGO_NET_OFFLINE="true")
    started = time.monotonic()
    run = subprocess.run(["cargo", "run", "--offline", "--example", "kb_rebuild_eval", "--", "--truth", str(input_path), "--data", str(data)], cwd=ROOT, env=environment, text=True, stdout=subprocess.PIPE, check=True)
    report = json.loads(run.stdout)
    validate(report)
    if truth.read_bytes() != original_truth or args.manifest.read_bytes() != manifest_bytes or prepared() != verified_input:
        raise ValueError("K2 or frozen source provenance changed during rebuild measurement")
    if before != [fingerprint(path) for path in paths]:
        raise ValueError("Implementation changed during rebuild measurement")
    report["data_root"] = str(data)
    report["verified_input_sha256"] = hashlib.sha256(input_path.read_bytes()).hexdigest()
    report["frozen_manifest_sha256"] = hashlib.sha256(manifest_bytes).hexdigest()
    report["frozen_sources"] = verified_input["sources"]
    report["k2_content_sha256"] = verified_input["truth_sha256"]
    atomic_json(ROOT / TRACE, report)
    collector = {"schema_version": 1, "suite": "resolution", "collector": "kb-rebuild-v1", "implementation": before,
                 "truth_sets": {"K2": {**fingerprint(str(truth.relative_to(ROOT))), "version": report["truth_version"]}},
                 "metrics": {"G4": {"sample": {"method": "ratio", "numerator": report["source_records"], "denominator": report["source_records"]}, "cases": report["source_records"], "evidence": [fingerprint(TRACE)]}},
                 "cost_usd": 0, "wall_seconds": time.monotonic() - started}
    atomic_json(OUTPUT, collector)
    print(json.dumps({"G4": 1, "source_records": report["source_records"], "deposited_pairs": report["deposited_pairs"], "data_root": str(data)}))


if __name__ == "__main__":
    main()
