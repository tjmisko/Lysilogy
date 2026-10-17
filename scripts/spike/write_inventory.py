#!/usr/bin/env python3
"""Write a hand-verified object inventory from a compact spec on stdin.

Spec (JSON):
  {"figures": [[number, page], ...], "tables": [...], "algorithms": [...],
   "equations": ["1-40", "A.1-A.3", "7a"], "equation_pages": {"1": 2, ...},
   "notes": "..."}

Equation ranges expand to every label in the range. Equation pages not given
in the spec are taken from the text-layer draft when it found that label.
Output: eval/spike/inventory/<stem>.json

Usage: python3 -B scripts/spike/write_inventory.py <stem> < spec.json
"""
import json
import re
import subprocess
import sys
from datetime import date
from pathlib import Path

RANGE = re.compile(r"^(?P<prefix>(?:[A-Z]\.|\d+\.)?)(?P<start>\d+)-(?:(?P=prefix))?(?P<end>\d+)$")


def expand(labels):
    numbers = []
    for label in labels:
        match = RANGE.match(label)
        if not match:
            numbers.append(label)
            continue
        prefix = match["prefix"]
        numbers.extend(f"{prefix}{n}" for n in range(int(match["start"]), int(match["end"]) + 1))
    if len(numbers) != len(set(numbers)):
        raise SystemExit("duplicate equation labels in spec")
    return numbers


def main():
    stem = sys.argv[1]
    spec = json.load(sys.stdin)
    draft = json.loads(subprocess.run(
        [sys.executable, "-B", str(Path(__file__).with_name("inventory_draft.py")), stem],
        capture_output=True, text=True, check=True,
    ).stdout)
    draft_pages = {item["number"]: item["page"] for item in draft["draft"]["equation"]}
    given_pages = {str(key): value for key, value in spec.get("equation_pages", {}).items()}
    equations = [
        {"number": number, "page": given_pages.get(number, draft_pages.get(number))}
        for number in expand(spec.get("equations", []))
    ]

    def objects(key):
        return [{"number": str(number), "page": page} for number, page in spec.get(key, [])]

    inventory = {
        "stem": stem,
        "verified": str(date.today()),
        "method": "hand check of 60 dpi 2x2 page sheets, full pages where unclear, against a pdftotext draft",
        "pages": draft["pages"],
        "counts": {
            "figure": len(spec.get("figures", [])),
            "table": len(spec.get("tables", [])),
            "algorithm": len(spec.get("algorithms", [])),
            "equation": len(equations),
        },
        "source_counts": draft["source_counts"],
        "figures": objects("figures"),
        "tables": objects("tables"),
        "algorithms": objects("algorithms"),
        "equations": equations,
        "notes": spec.get("notes", ""),
    }
    out = Path("eval/spike/inventory") / f"{stem}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(inventory, indent=1) + "\n", encoding="utf-8")
    missing = sum(1 for equation in equations if equation["page"] is None)
    print(f"{out}: {inventory['counts']} (source {draft['source_counts']}); equation pages unknown: {missing}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
