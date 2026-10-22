#!/usr/bin/env python3
"""Freeze the extraction-spike sample.

The handoff's rule (aligned, with an exact figure/table/equation inventory)
selects no papers: the LaTeX harness withholds every inventory in this corpus
because each source uses at least one macro it does not interpret. The object
truth is therefore hand-verified from page images (eval/spike/inventory/), and
the sample is the first papers in stem order that are aligned, 8-25 pages,
and have a complete front-matter oracle (title, authors, abstract).

Usage: python3 -B scripts/spike/select_sample.py --audit <corpus-audit.json>
"""
import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "truth" / "latex"))

from frontmatter import front_matter_from_source  # noqa: E402

PAGE_MINIMUM, PAGE_MAXIMUM = 8, 25


def page_count(pdf):
    output = subprocess.run(["pdfinfo", str(pdf)], capture_output=True, text=True, timeout=30).stdout
    match = re.search(r"^Pages:\s+(\d+)", output, re.MULTILINE)
    return int(match[1]) if match else None


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def select(audit, corpus, size):
    chosen, skipped = [], {"pages": 0, "front_matter": 0, "missing_pdf": 0}
    rows = sorted((row for row in audit["rows"] if row["status"] == "aligned"), key=lambda row: row["stem"])
    for row in rows:
        if len(chosen) == size:
            break
        pdf = corpus / "pdf" / f"{row['stem']}.pdf"
        source = corpus / "source" / f"{row['stem']}.src"
        if not pdf.exists():
            skipped["missing_pdf"] += 1
            continue
        pages = page_count(pdf)
        if pages is None or not PAGE_MINIMUM <= pages <= PAGE_MAXIMUM:
            skipped["pages"] += 1
            continue
        front = front_matter_from_source(source, row["main"])
        if front["status"] != "ok" or front["withheld"]:
            skipped["front_matter"] += 1
            continue
        coverage = row["kind_coverage"]
        chosen.append({
            "stem": row["stem"],
            "identity": row["identity"],
            "pdf": str(pdf),
            "pdf_sha256": sha256(pdf),
            "source": str(source),
            "main": row["main"],
            "pages": pages,
            "source_counts": {kind: coverage[kind]["expected"] for kind in ("figure", "table", "equation", "algorithm")},
            "harness_inventory_eligible": {kind: coverage[kind]["eligible"] for kind in ("figure", "table", "equation")},
            "object_truth": f"eval/spike/inventory/{row['stem']}.json",
        })
    return chosen, skipped, len(rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--audit", required=True, type=Path)
    parser.add_argument("--corpus", type=Path, default=Path.home() / "Corpora/arxiv")
    parser.add_argument("--size", type=int, default=20)
    parser.add_argument("--out", type=Path, default=Path("eval/spike/sample.json"))
    args = parser.parse_args()
    audit = json.loads(args.audit.read_text(encoding="utf-8"))
    chosen, skipped, aligned = select(audit, args.corpus, args.size)
    manifest = {
        "frozen": "2026-10-16",
        "rule": f"aligned, {PAGE_MINIMUM}-{PAGE_MAXIMUM} pages, complete front-matter oracle; first {args.size} by stem",
        "deviation": (
            "The handoff rule (exact harness inventory for figure, table and equation) and its relaxation "
            "(inventory_known) select 0 of the aligned papers: every source triggers the harness's unknown-macro "
            "guard. Object truth is hand-verified from page images instead; source_counts are the parser's "
            "uncertified counts, kept for comparison."
        ),
        "audited_papers": audit["papers"],
        "aligned_papers": aligned,
        "skipped_before_full": skipped,
        "papers": chosen,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(f"{len(chosen)} papers -> {args.out} (skipped {skipped})")
    return 0 if len(chosen) == args.size else 1


if __name__ == "__main__":
    sys.exit(main())
