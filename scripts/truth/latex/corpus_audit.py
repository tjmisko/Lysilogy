#!/usr/bin/env python3
import argparse
import json
import sys
from pathlib import Path

from align_cli import align


def build_parser():
    parser = argparse.ArgumentParser(
        description="Measure LaTeX-truth alignment loss across the arXiv corpus."
    )
    parser.add_argument(
        "--identities",
        type=Path,
        default=Path.home() / ".cache/lysilogy/arxiv-kb-data/paper-identities.json",
    )
    parser.add_argument(
        "--index-root",
        type=Path,
        default=Path.home() / ".cache/lysilogy/arxiv-kb-data/papers",
    )
    parser.add_argument(
        "--source-root", type=Path, default=Path.home() / "Corpora/arxiv/source"
    )
    parser.add_argument("--limit", type=int, default=0, help="maximum papers; 0 means all")
    parser.add_argument("--json", type=Path, help="write per-paper reports here")
    return parser


def pairs(identities, index_root, source_root):
    records = json.loads(identities.read_text(encoding="utf-8"))["records"]
    found = []
    for key, record in records.items():
        index = index_root / key / "reading-index.json"
        stem = record["relative_path"].rsplit(".pdf", 1)[0]
        source = source_root / (stem + ".src")
        if index.exists() and source.exists():
            found.append((stem, source, index))
    found.sort(key=lambda item: item[0])
    return found


def measure(found, limit):
    if limit:
        found = found[:limit]
    aggregate = {
        "papers": len(found),
        "statuses": {},
        "mean_completeness": 0.0,
        "bibliography_exhaustive": 0,
        "kind_expected": {},
        "kind_aligned": {},
        "unsupported_reasons": {},
        "rows": [],
    }
    completeness = []
    for stem, source, index in found:
        result = align(source, index)
        status = result.get("status", "invalid")
        aggregate["statuses"][status] = aggregate["statuses"].get(status, 0) + 1
        row = {"stem": stem, "status": status}
        if status == "aligned":
            alignment = result["alignment"]
            completeness.append(alignment["overall_completeness"])
            row["completeness"] = alignment["overall_completeness"]
            row["bibliography_exhaustive"] = alignment["bibliography_exhaustive"]
            if alignment["bibliography_exhaustive"]:
                aggregate["bibliography_exhaustive"] += 1
            for kind, coverage in result["kind_coverage"].items():
                aggregate["kind_expected"][kind] = (
                    aggregate["kind_expected"].get(kind, 0) + coverage["expected"]
                )
                aggregate["kind_aligned"][kind] = (
                    aggregate["kind_aligned"].get(kind, 0) + coverage["aligned"]
                )
        else:
            reason = result.get("reason", "")
            key = reason.split(":", 1)[0] or reason
            aggregate["unsupported_reasons"][key] = (
                aggregate["unsupported_reasons"].get(key, 0) + 1
            )
            row["reason"] = reason
        aggregate["rows"].append(row)
    if completeness:
        aggregate["mean_completeness"] = sum(completeness) / len(completeness)
    return aggregate


def summary_text(aggregate):
    statuses = aggregate["statuses"]
    lines = [
        f"papers {aggregate['papers']}",
        "aligned {}, unsupported {}, invalid {}".format(
            statuses.get("aligned", 0), statuses.get("unsupported", 0), statuses.get("invalid", 0)
        ),
        f"mean completeness {aggregate['mean_completeness']:.3f}",
        f"bibliography exhaustive {aggregate['bibliography_exhaustive']}",
    ]
    for kind in sorted(aggregate["kind_expected"]):
        expected = aggregate["kind_expected"][kind]
        aligned = aggregate["kind_aligned"].get(kind, 0)
        lines.append(f"  {kind:10} {aligned}/{expected}")
    if aggregate["unsupported_reasons"]:
        lines.append("unsupported reasons:")
        for reason, count in sorted(
            aggregate["unsupported_reasons"].items(), key=lambda item: (-item[1], item[0])
        ):
            lines.append(f"  {count:4} {reason}")
    return "\n".join(lines)


def main():
    args = build_parser().parse_args()
    aggregate = measure(pairs(args.identities, args.index_root, args.source_root), args.limit)
    if args.json:
        args.json.write_text(
            json.dumps(aggregate, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
    print(summary_text(aggregate))
    return 0


if __name__ == "__main__":
    sys.exit(main())
