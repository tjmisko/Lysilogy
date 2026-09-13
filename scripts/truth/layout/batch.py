"""The frozen ten-paper experiment: complete denominators and no backfilling."""
import json
from pathlib import Path
import time

from experiment import build_paper, checked_artifacts, compare_document, page_count
from policy import Refused, binding, regular_path
import runtime


def run(config, snapshot):
    selection_path = snapshot / "selection.json"
    runtime_path = snapshot / "runtime.json"
    selection = json.loads(selection_path.read_text())
    installed = json.loads(runtime_path.read_text())
    papers = selection["papers"]
    if len(papers) != 10 or len({(p["arxiv_id"], p["version"]) for p in papers}) != 10:
        raise Refused("the experiment requires the frozen ten distinct papers")
    if selection["go_no_go"] != {"selected": 10, "minimum_builds": 8,
                                   "minimum_exact_whole_documents": 5,
                                   "required_dpi_for_every_page": [96, 192]}:
        raise Refused("experimental criteria changed")
    destination = regular_path(config["output_parent"], directory=True) / "experiment"
    destination.mkdir(mode=0o700, exist_ok=False)
    started = time.monotonic()
    record = {"schema_version": 1, "issue": 110, "selection": binding(selection_path),
              "runtime": binding(runtime_path), "papers": [], "status": "prepared",
              "network_calls": 0, "model_calls": 0, "external_cost_usd": 0,
              "truth_admission": False, "source_layout_codec": "not implemented"}
    try:
        runtime.verify(installed)
        for position, paper in enumerate(papers):
            item = {"position": position, "arxiv_id": paper["arxiv_id"], "version": paper["version"],
                    "status": "not_started", "built": False, "exact_whole_document": False}
            record["papers"].append(item)
            # Leave every unrun item in the original population. A single build
            # reserves its full90seconds; no job may cross the global deadline.
            remaining = 900 - (time.monotonic() - started)
            if remaining < 90:
                item["status"] = "batch_deadline_not_started"
                continue
            checked_artifacts(paper)
            paper_dir = destination / f"paper-{position:02d}"
            paper_dir.mkdir(mode=0o700)
            built = build_paper(paper, installed, paper_dir / "build")
            item["build_receipt"] = binding(paper_dir / "build/receipt.json")
            item["status"] = built["status"]
            if built["status"] != "built":
                continue
            item["built"] = True
            try:
                original_count = page_count(paper["frozen_artifacts"]["pdf"]["path"], installed, paper_dir / "original-count", deadline=started + 900)
                rebuilt_count = page_count(built["pdf"]["path"], installed, paper_dir / "rebuilt-count", deadline=started + 900)
                item["page_counts"] = {"original": original_count, "rebuilt": rebuilt_count}
                if original_count["count"] != paper["page_count"]:
                    raise Refused("original PDF page count disagrees with its frozen native index")
                comparison = compare_document(paper, built["pdf"]["path"], installed,
                                              paper_dir / "comparison", rebuilt_count["count"], deadline=started + 900)
                item["comparison_receipt"] = binding(paper_dir / "comparison/receipt.json")
                item["exact_whole_document"] = comparison["exact_whole_document"]
                item["status"] = comparison["status"]
            except Refused as error:
                item["status"] = "correspondence_unavailable"
                item["error"] = str(error)
            checked_artifacts(paper)
            with (destination / "progress.jsonl").open("a") as stream:
                stream.write(json.dumps(item, sort_keys=True) + "\n")
        runtime.verify(installed)
        record["status"] = "completed"
    except BaseException as error:
        record["status"] = "failed"
        record["error"] = type(error).__name__ + ": " + str(error)
        raise
    finally:
        for position in range(len(record["papers"]), len(papers)):
            paper = papers[position]
            record["papers"].append({"position": position, "arxiv_id": paper["arxiv_id"], "version": paper["version"],
                                      "status": "not_started_after_failure", "built": False, "exact_whole_document": False})
        record["builds"] = sum(item["built"] for item in record["papers"])
        record["exact_whole_documents"] = sum(item["exact_whole_document"] for item in record["papers"])
        record["exploratory_go"] = record["status"] == "completed" and record["builds"] >= 8 and record["exact_whole_documents"] >= 5
        record["wall_seconds"] = time.monotonic() - started
        with (destination / "receipt.json").open("x") as stream:
            json.dump(record, stream, sort_keys=True, indent=2)
            stream.write("\n")
    return record
