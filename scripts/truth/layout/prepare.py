#!/usr/bin/env python3
"""Freeze ten source-available papers before any source-layout build."""
import argparse
import hashlib
import json
from pathlib import Path
import time

from inputs import select_ten
from policy import Refused, binding, regular_path, safe_relative


def prepare(prior_path, prior_sha, corpus, data, output):
    prior_path = regular_path(prior_path)
    if prior_path.stat().st_size > 16 * 1024 ** 2:
        raise Refused("prior selection exceeds its bound")
    raw = prior_path.read_bytes()
    if hashlib.sha256(raw).hexdigest() != prior_sha:
        raise Refused("prior selection identity mismatch")
    prior = json.loads(raw)
    if prior.get("detector_outputs_consulted") is not False or prior.get("selection_is_truth") is not False:
        raise Refused("prior pool lacks its pre-outcome availability attestation")
    corpus, data = regular_path(corpus, directory=True), regular_path(data, directory=True)
    started = time.monotonic()
    papers = select_ten(prior)
    for paper in papers:
        artifacts = {}
        for role in ("pdf", "source"):
            expected = paper["paper"][role]
            path = corpus / safe_relative(expected["path"])
            actual = binding(path)
            if actual["sha256"] != expected["sha256"] or actual["bytes"] != expected["bytes"]:
                raise Refused("original artifact identity mismatch")
            artifacts[role] = actual
        expected = paper["mapping"]["index"]
        actual = binding(data / safe_relative(expected["path"]))
        if actual["sha256"] != expected["sha256"]:
            raise Refused("original native-index identity mismatch")
        artifacts["index"] = actual
        paper["frozen_artifacts"] = artifacts
        # No PDF metadata engine has run during selection. A fixed epoch is
        # explicit unavailable date fidelity, never a guessed publication date.
        paper["source_date_epoch"] = 0
        paper["date_fidelity"] = "unavailable; fixed epoch zero, no outcome fitting"
    result = {
        "schema_version": 1, "issue": 110, "prior_selection": binding(prior_path),
        "selection_rule": "first category-rank1 from seven strata, then earliest three rank2 positions; original order",
        "inherited_limitations": ["availability-only <=8-page manual pool", "original old-two release exclusion",
                                  "original failed-index exclusion", "not a random representative sample"],
        "go_no_go": {"selected": 10, "minimum_builds": 8, "minimum_exact_whole_documents": 5,
                     "required_dpi_for_every_page": [96, 192]},
        "papers": papers, "deposited_source_executed": False, "truth_admission": False,
        "network_calls": 0, "model_calls": 0, "external_cost_usd": 0,
        "wall_seconds": time.monotonic() - started,
    }
    output = Path(output).absolute()
    regular_path(output.parent, directory=True)
    with output.open("x") as stream:
        json.dump(result, stream, sort_keys=True, indent=2)
        stream.write("\n")
    return binding(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prior-selection", type=Path, required=True)
    parser.add_argument("--prior-sha256", required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(prepare(args.prior_selection, args.prior_sha256, args.corpus, args.data, args.output)))


if __name__ == "__main__":
    main()
