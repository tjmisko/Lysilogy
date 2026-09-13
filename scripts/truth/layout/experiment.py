"""Explicit source/layout experiment primitives, separate from K1 admission.

The final reviewed launcher owns selection/runtime/code pinning and the batch
deadline. These functions never repair sources, install packages or call models.
"""
import hashlib
import json
from pathlib import Path
import time
import re

from confinement import prepare_outputs, run_sandbox
from inputs import materialize
from pixels import compare_pages, whole_document
from policy import Limits, Refused, binding, engine_command, fixed_environment, job_name, output_names


def checked_artifacts(paper):
    for expected in paper["frozen_artifacts"].values():
        if binding(expected["path"]) != expected:
            raise Refused("frozen original artifact changed")


def build_paper(paper, runtime, destination, *, deadline=None):
    checked_artifacts(paper)
    started = time.monotonic()
    destination = Path(destination)
    destination.mkdir(mode=0o700, exist_ok=False)
    record = {"arxiv_id": paper["arxiv_id"], "version": paper["version"],
              "paper_id": paper["mapping"]["paper_id"], "passes": [],
              "originals": paper["frozen_artifacts"], "status": "prepared",
              "source_date_epoch": paper["source_date_epoch"], "date_fidelity": paper["date_fidelity"],
              "network_calls": 0, "model_calls": 0, "external_cost_usd": 0}
    try:
        original = paper["frozen_artifacts"]["source"]
        source = materialize(original["path"], original["sha256"], destination / "input")
        record["source"] = source
        main = source["main"]
        names = output_names(main)
        output = prepare_outputs(destination / "output", names)
        environment = fixed_environment(paper["source_date_epoch"])
        command = engine_command(main)
        for number in (1, 2):
            remaining = min(started + 90, deadline if deadline is not None else started + 90) - time.monotonic()
            if remaining <= 0:
                record["status"] = "paper_wall_timeout"
                break
            run_dir = destination / ("pass-" + str(number))
            run_dir.mkdir(mode=0o700)
            receipt = run_sandbox(run_dir=run_dir, readonly=[*runtime["engine_mounts"],
                                  (destination / "input", "/input")], output=output, names=names,
                                  command=command, environment=environment, limits=Limits(wall_seconds=remaining),
                                  expected_tools=runtime.get("confinement_tools"))
            record["passes"].append({"number": number, "receipt": binding(run_dir / "receipt.json"),
                                     "status": receipt["status"], "wall_seconds": receipt["wall_seconds"]})
            if receipt["status"] != "passed":
                record["status"] = receipt["status"]
                break
        else:
            pdf = output / (job_name(main) + ".pdf")
            with pdf.open("rb") as stream:
                if not stream.read(8).startswith(b"%PDF-"):
                    raise Refused("engine did not produce a bounded PDF")
            record["pdf"] = binding(pdf)
            record["status"] = "built"
        record["synctex"] = [binding(output / name) for name in names if "synctex" in name and (output / name).stat().st_size]
        record["synctex_query_available"] = False
        record["synctex_limitation"] = "No reviewed query decoder; output and any busy-file finalization failure are capability evidence only"
        for member in source["members"]:
            actual = binding(member["path"])
            if any(actual[key] != member[key] for key in ("path", "bytes", "sha256")):
                raise Refused("materialized source changed")
    except Refused as error:
        record["status"] = "unsupported_or_refused"
        record["error"] = str(error)
    except BaseException as error:
        record["status"] = "interrupted" if isinstance(error, (KeyboardInterrupt, SystemExit, InterruptedError)) else "failed"
        record["error"] = type(error).__name__ + ": " + str(error)
        raise
    finally:
        checked_artifacts(paper)
        record["wall_seconds"] = time.monotonic() - started
        with (destination / "receipt.json").open("x") as stream:
            json.dump(record, stream, sort_keys=True, indent=2)
            stream.write("\n")
    return record


def remaining_seconds(maximum, deadline):
    remaining = maximum if deadline is None else min(maximum, deadline - time.monotonic())
    if remaining <= 0:
        raise Refused("experiment deadline exhausted")
    return remaining


def render_page(pdf, runtime, directory, page, dpi, *, deadline=None):
    if type(page) is not int or not 1 <= page <= 8 or dpi not in (96, 192):
        raise Refused("unsupported page render request")
    pdf = Path(pdf)
    original = binding(pdf)
    directory = Path(directory)
    directory.mkdir(mode=0o700, exist_ok=False)
    output = prepare_outputs(directory / "output", ("unused",))
    command = ["/runtime/bin/mutool", "draw", "-q", "-F", "ppm", "-c", "rgb",
               "-A", "8", "-r", str(dpi), "-o", "-", "/runtime/document.pdf", str(page)]
    receipt = run_sandbox(run_dir=directory, readonly=[*runtime["render_mounts"],
                          (pdf, "/runtime/document.pdf")], output=output, names=("unused",),
                          command=command, environment={"PATH": "/runtime/bin", "HOME": "/unmounted", "LC_ALL": "C"},
                          limits=Limits(wall_seconds=remaining_seconds(30, deadline), cpu_seconds=15),
                          expected_tools=runtime.get("confinement_tools"))
    if binding(pdf) != original:
        raise Refused("PDF changed during rendering")
    if receipt["status"] != "passed":
        raise Refused("confined page renderer failed")
    return {"pdf": original, "page": page, "dpi": dpi, "raster": binding(directory / "stdout.log"),
            "receipt": binding(directory / "receipt.json")}


def page_count(pdf, runtime, directory, *, deadline=None):
    """Read the page-tree count through the same confined PDF implementation."""
    pdf = Path(pdf)
    original = binding(pdf)
    directory = Path(directory)
    directory.mkdir(mode=0o700, exist_ok=False)
    output = prepare_outputs(directory / "output", ("unused",))
    command = ["/runtime/bin/mutool", "show", "/runtime/document.pdf", "trailer/Root/Pages/Count"]
    receipt = run_sandbox(run_dir=directory, readonly=[*runtime["render_mounts"],
                          (pdf, "/runtime/document.pdf")], output=output, names=("unused",),
                          command=command, environment={"PATH": "/runtime/bin", "HOME": "/unmounted", "LC_ALL": "C"},
                          limits=Limits(wall_seconds=remaining_seconds(10, deadline), cpu_seconds=5, file_bytes=4096),
                          expected_tools=runtime.get("confinement_tools"))
    if receipt["status"] != "passed" or binding(pdf) != original:
        raise Refused("confined PDF page-count query failed")
    value = (directory / "stdout.log").read_text().strip()
    if not re.fullmatch(r"[1-9][0-9]{0,4}", value):
        raise Refused("invalid PDF page count")
    return {"count": int(value), "pdf": original, "receipt": binding(directory / "receipt.json")}


def compare_document(paper, rebuilt, runtime, destination, rebuilt_page_count, *, deadline=None):
    destination = Path(destination)
    destination.mkdir(mode=0o700, exist_ok=False)
    record = {"pages": [], "original_page_count": paper["page_count"],
              "rebuilt_page_count": rebuilt_page_count, "exact_whole_document": False,
              "original_identity": paper["frozen_artifacts"]["pdf"], "rebuilt_identity": binding(rebuilt),
              "status": "prepared", "truth_admission": False, "source_semantics_proved": False}
    try:
        if rebuilt_page_count != paper["page_count"]:
            record["status"] = "page_count_mismatch"
            return record
        for page in range(1, paper["page_count"] + 1):
            for dpi in (96, 192):
                records = {}
                for role, pdf in (("original", paper["frozen_artifacts"]["pdf"]["path"]), ("rebuilt", rebuilt)):
                    records[role] = render_page(pdf, runtime, destination / f"{role}-{page}-{dpi}", page, dpi, deadline=deadline)
                comparison = compare_pages(Path(records["original"]["raster"]["path"]).read_bytes(),
                                           Path(records["rebuilt"]["raster"]["path"]).read_bytes())
                record["pages"].append({"page": page, "dpi": dpi, **comparison, "evidence": records})
        record["exact_whole_document"] = whole_document(record["pages"], paper["page_count"], rebuilt_page_count)
        record["status"] = "compared"
        return record
    except BaseException as error:
        record["status"] = "failed"
        record["error"] = type(error).__name__ + ": " + str(error)
        raise
    finally:
        with (destination / "receipt.json").open("x") as stream:
            json.dump(record, stream, sort_keys=True, indent=2)
            stream.write("\n")
