#!/usr/bin/env python3
import argparse
import json
import sys
from pathlib import Path

from align import align_paper
from archive import UnsupportedSource, read_archive
from parser import parse_project

SUMMARY_FIELDS = (
    "aligned_objects_by_kind",
    "bibliography_eligible",
    "coverage",
    "eligible_kinds",
    "kind_coverage",
    "metric_eligibility",
)


def build_parser():
    parser = argparse.ArgumentParser(
        description="Align one deposited LaTeX source against a Lysilogy reading index."
    )
    parser.add_argument("--source", required=True, type=Path, help="arXiv .src/.tar.gz/.tex source")
    parser.add_argument("--index", required=True, type=Path, help="reading-index.json")
    parser.add_argument("--main", help="main .tex member when the root is ambiguous")
    parser.add_argument("--output", type=Path, help="write JSON here instead of stdout")
    parser.add_argument("--full", action="store_true", help="include every aligned object and entry")
    return parser


def align(source_path, index_path, main=None, full=False):
    raw = source_path.read_bytes()
    document = json.loads(index_path.read_text(encoding="utf-8"))
    index = document["index"] if isinstance(document, dict) and "index" in document else document
    try:
        files, members = read_archive(raw)
        parsed = parse_project(files, selected_main=main)
        result = align_paper(parsed, index)
    except UnsupportedSource as error:
        return {"status": "unsupported", "reason": str(error)}
    except (ValueError, KeyError) as error:
        return {"status": "invalid", "reason": f"{type(error).__name__}: {error}"}
    report = {"status": "aligned", "source_members": members, "main": parsed["main"]}
    if full:
        report.update(result)
    else:
        report.update({field: result[field] for field in SUMMARY_FIELDS})
        report["alignment"] = result["alignment"]
        report["excluded_objects"] = len(result["excluded_objects"])
        report["excluded_citations"] = len(result["excluded_citations"])
        report["excluded_references"] = len(result["excluded_references"])
    return report


def main():
    args = build_parser().parse_args()
    report = align(args.source, args.index, args.main, args.full)
    encoded = json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True)
    if args.output:
        args.output.write_text(encoded + "\n", encoding="utf-8")
    else:
        print(encoded)
    return 0


if __name__ == "__main__":
    sys.exit(main())
