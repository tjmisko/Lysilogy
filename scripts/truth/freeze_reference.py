#!/usr/bin/env python3
"""Explicit live freeze orchestration; all HTTP uses the existing Rust GraphHttp service."""
import argparse
import collections
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

from reference_truth import (MAX_BYTES, TruthError, canonical, doi, fingerprint,
                             load_snapshots, read_json, reject_symlinks, validate_request, validate_response, write_immutable)


def external_root(path):
    path = Path(os.path.abspath(Path(path).expanduser()))
    reject_symlinks(path)
    allowed = (Path.home() / ".cache/lysilogy", Path.home() / "Corpora")
    if not any(path.is_relative_to(root) and path != root for root in allowed):
        raise TruthError("Raw provider snapshots require a dedicated directory under ~/.cache/lysilogy or ~/Corpora")
    return path


def freeze_request(root, request, fetch):
    """The injected fetch is the sole live boundary; an existing frozen request never refreshes."""
    validate_request(request)
    identity = fingerprint(request)
    request_path = root / "requests" / (identity + ".json")
    if request_path.exists():
        manifest = read_json(request_path)
        if manifest.get("request") != request:
            raise TruthError("Frozen request fingerprint mismatch")
        if not isinstance(manifest.get("snapshots"), list) or len(manifest["snapshots"]) != 1:
            raise TruthError("A frozen request must identify exactly one response snapshot")
        loaded = load_snapshots(root, manifest)
        receipt = loaded[0]["receipt"]
        if receipt["provider"] != request["provider"] or receipt["requested_dois"] != request["dois"]:
            raise TruthError("Frozen response does not belong to the requested DOI lookup")
        return manifest["snapshots"][0]
    response = fetch(request)
    raw, missing = validate_response(request, response)
    body_file = f"responses/{response['body_sha256']}.json"
    write_immutable(root / body_file, raw)
    receipt = {key: response[key] for key in ("schema_version", "provider", "url", "retrieved_at", "frozen_at",
                                             "body_sha256", "cache_hit", "wall_seconds", "model_calls",
                                             "model_cost_usd", "provider_cost_usd")}
    receipt.update(body_file=body_file, requested_dois=request["dois"], missing_dois=missing)
    encoded = (canonical(receipt) + "\n").encode()
    digest = hashlib.sha256(encoded).hexdigest()
    descriptor = {"path": f"receipts/{digest}.json", "sha256": digest}
    write_immutable(root / descriptor["path"], encoded)
    manifest = {"schema_version": 1, "request": request, "snapshots": [descriptor]}
    write_immutable(request_path, (canonical(manifest) + "\n").encode())
    return descriptor


def invoke_helper(helper, request):
    command = [str(helper), "--provider", request["provider"], "--doi", *request["dois"]]
    completed = subprocess.run(command, check=False, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=45)
    if completed.returncode:
        # GraphHttp emits credential-redacted errors; arbitrary upstream bodies are never errors here.
        message = completed.stderr.decode(errors="replace").strip()[:500]
        raise TruthError("Provider truth lookup failed: " + message)
    if len(completed.stdout) > 2 * MAX_BYTES + 64 * 1024:
        raise TruthError("Truth transport receipt exceeds the bound")
    return json.loads(completed.stdout)


def seed_plan(selection, count, seed):
    """Choose citing metadata deterministically; this does not claim downloaded or mapped papers."""
    if type(count) is not int or count <= 0 or count > 100:
        raise TruthError("A citing-paper truth build must request between one and 100 seeds")
    if selection.get("selection_sha256") != fingerprint(selection["papers"]):
        raise TruthError("The source selection fingerprint does not match its records")
    groups = collections.defaultdict(list)
    seen = set()
    for paper in sorted(selection["papers"], key=lambda row: row["id"]):
        identifier = doi(paper.get("doi"))
        if not identifier or identifier in seen or not paper.get("categories"):
            continue
        seen.add(identifier)
        # Seed diversity follows observed primary category. K5 uses the target's actual field/year.
        group = paper["categories"][0].split(".", 1)[0].split("-", 1)[0]
        groups[group].append({"doi": identifier, "kind": "arxiv_metadata", "arxiv_id": paper["id"],
                              "metadata_sha256": fingerprint(paper), "selection_sha256": selection["selection_sha256"]})
    for rows in groups.values():
        rows.sort(key=lambda row: (fingerprint([seed, row["doi"]]), row["doi"]))
    if sum(map(len, groups.values())) < count:
        raise TruthError("Too few observed citing DOIs to fill the seed plan")
    selected, index = [], 0
    while len(selected) < count:
        for field in sorted(groups):
            if index < len(groups[field]):
                selected.append(groups[field][index])
                if len(selected) == count:
                    break
        index += 1
    return {"schema_version": 1, "seed": seed, "seeds": selected,
            "requests": [{"provider": "crossref", "dois": [row["doi"]]} for row in selected]}


def oa_plan(k2, batch_size=25):
    if not 1 <= batch_size <= 100 or k2.get("truth_set") != "K2":
        raise TruthError("OA lookup planning requires K2 and a batch size of one to100")
    identifiers = sorted({case["expected_doi"] for case in k2["cases"] if case["eligible_resolution"]})
    regular = [identifier for identifier in identifiers if not any(c in identifier for c in "|,")]
    regular_set = set(regular)
    special = [identifier for identifier in identifiers if identifier not in regular_set]
    requests = [{"provider": "openalex", "dois": regular[start:start + batch_size]}
                for start in range(0, len(regular), batch_size)]
    requests += [{"provider": "openalex", "dois": [identifier]} for identifier in special]
    return {"schema_version": 1, "k2_version": k2["version"], "requests": requests}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    seeds = sub.add_parser("seed-plan")
    seeds.add_argument("--selection", type=Path, required=True)
    seeds.add_argument("--count", type=int, default=30)
    seeds.add_argument("--seed", default="reference-truth-v1")
    seeds.add_argument("--output", type=Path, required=True)
    oa = sub.add_parser("oa-plan")
    oa.add_argument("--k2", type=Path, required=True)
    oa.add_argument("--batch-size", type=int, default=25)
    oa.add_argument("--output", type=Path, required=True)
    freeze = sub.add_parser("freeze")
    freeze.add_argument("--plan", type=Path, required=True)
    freeze.add_argument("--root", type=Path, default=Path.home() / ".cache/lysilogy/reference-truth")
    freeze.add_argument("--helper", type=Path, default=Path("target/debug/examples/freeze_reference_provider"))
    freeze.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == "seed-plan":
            result = seed_plan(read_json(args.selection, 128 * 1024 * 1024), args.count, args.seed)
        elif args.command == "oa-plan":
            result = oa_plan(read_json(args.k2), args.batch_size)
        else:
            root = external_root(args.root)
            plan = read_json(args.plan)
            if plan.get("schema_version") != 1 or not 1 <= len(plan["requests"]) <= 100:
                raise TruthError("Explicit truth builds require a plan with one to100 requests")
            descriptors = []
            for request in plan["requests"]:
                descriptors.append(freeze_request(root, request, lambda request: invoke_helper(args.helper, request)))
                print(canonical({"frozen": len(descriptors), "planned": len(plan["requests"])}), file=sys.stderr, flush=True)
            result = {"schema_version": 1, "plan_sha256": fingerprint(plan), "snapshots": descriptors}
        write_immutable(args.output, (canonical(result) + "\n").encode())
        return 0
    except (OSError, KeyError, TypeError, ValueError, subprocess.TimeoutExpired) as error:
        print(f"reference freeze: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
