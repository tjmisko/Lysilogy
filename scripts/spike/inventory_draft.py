#!/usr/bin/env python3
"""Draft a paper's object inventory for hand verification.

Reads the PDF text layer for caption starts ("Figure 3:", "Table 2.") and
right-margin equation numbers ("(12)", "(A.3)"), records each with its page,
compares the counts with the source parser's, and renders 2x2 page sheets for
visual checking. The draft is a starting point, never truth: the verified
inventory in eval/spike/inventory/ is written after looking at the pages.

Usage: python3 -B scripts/spike/inventory_draft.py <stem> [--sheets DIR]
"""
import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

CAPTION = re.compile(
    r"^\s*(?P<kind>Figure|Fig\.|FIG\.|FIGURE|Table|TABLE|Algorithm|ALGORITHM)\s*"
    r"(?P<number>[A-Z]?\.?\d+(?:\.\d+)?|[IVXL]+)\s*[:.|—–-]"
)
EQUATION = re.compile(r"\((?P<number>(?:[A-Z]\.?)?\d+(?:\.\d+)?[a-z]?)\)\s*$")
KINDS = {"fig": "figure", "table": "table", "algorithm": "algorithm"}


def page_texts(pdf, pages):
    for number in range(1, pages + 1):
        text = subprocess.run(
            ["pdftotext", "-layout", "-f", str(number), "-l", str(number), str(pdf), "-"],
            capture_output=True, text=True, timeout=60,
        ).stdout
        yield number, text


def draft(paper):
    found = {"figure": {}, "table": {}, "algorithm": {}, "equation": {}}
    for page, text in page_texts(paper["pdf"], paper["pages"]):
        for line in text.splitlines():
            caption = CAPTION.match(line)
            if caption:
                kind = next(value for key, value in KINDS.items() if caption["kind"].lower().startswith(key))
                found[kind].setdefault(caption["number"], page)
            equation = EQUATION.search(line)
            # A short line or a wide gap before the number marks a displayed equation.
            if equation and (len(line.strip()) < 100 or "   " in line[: equation.start()]):
                found["equation"].setdefault(equation["number"], page)
    return {
        kind: [{"number": number, "page": page} for number, page in items.items()]
        for kind, items in found.items()
    }


def sheets(paper, directory):
    directory.mkdir(parents=True, exist_ok=True)
    prefix = directory / "page"
    subprocess.run(["pdftoppm", "-r", "60", "-png", paper["pdf"], str(prefix)], check=True, timeout=300)
    pages = sorted(directory.glob("page-*.png"))
    written = []
    for start in range(0, len(pages), 4):
        group = pages[start:start + 4]
        target = directory / f"sheet-{start // 4 + 1:02d}.png"
        subprocess.run(
            ["montage", *map(str, group), "-tile", "2x2", "-geometry", "+6+6", "-label", "%f", str(target)],
            check=True, timeout=120,
        )
        written.append(str(target))
    return written


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("stem")
    parser.add_argument("--sample", type=Path, default=Path("eval/spike/sample.json"))
    parser.add_argument("--sheets", type=Path, default=Path.home() / ".cache/lysilogy/spike/sheets")
    args = parser.parse_args()
    paper = next(p for p in json.loads(args.sample.read_text())["papers"] if p["stem"] == args.stem)
    result = draft(paper)
    counts = {kind: len(items) for kind, items in result.items()}
    print(json.dumps({
        "stem": paper["stem"],
        "pages": paper["pages"],
        "source_counts": paper["source_counts"],
        "text_counts": counts,
        "draft": result,
        "sheets": sheets(paper, args.sheets / paper["stem"]),
    }, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
