#!/usr/bin/env python3
"""Append curated papers from the local library to the frozen spike sample.

The original 20 are kept. Each addition carries a hand-checked identity oracle
(title and printed authors) read from page 1, since these PDFs have no LaTeX
source. Objects are not scored for them.

Usage: python3 -B scripts/spike/add_local_sample.py [--library DIR]
"""
import argparse
import json
from pathlib import Path

LIBRARY = Path("/home/tjmisko/Projects/Lysilogy/local-articles")

# stem, filename, pages, title, authors (printed order).
LOCAL = [
    ("amodei2016", "Amodei et al. - 2016 - Concrete Problems in AI Safety.pdf", 29,
     "Concrete Problems in AI Safety",
     ["Dario Amodei", "Chris Olah", "Jacob Steinhardt", "John Schulman", "Paul Christiano", "Dan Mané"]),
    ("christiano2017", "Christiano et al. - 2017 - Deep Reinforcement Learning from Human Preferences.pdf", 17,
     "Deep Reinforcement Learning from Human Preferences",
     ["Paul F. Christiano", "Jan Leike", "Tom B. Brown", "Miljan Martic", "Shane Legg", "Dario Amodei"]),
    ("hubinger2019", "Hubinger et al. - 2019 - Risks from Learned Optimization in Advanced Machine Learning Systems.pdf", 39,
     "Risks from Learned Optimization in Advanced Machine Learning Systems",
     ["Evan Hubinger", "Chris van Merwijk", "Vladimir Mikulik", "Joar Skalse", "Scott Garrabrant"]),
    ("kaplan2020", "Kaplan et al. - 2020 - Scaling Laws for Neural Language Models.pdf", 30,
     "Scaling Laws for Neural Language Models",
     ["Jared Kaplan", "Sam McCandlish", "Tom Henighan", "Tom B. Brown", "Benjamin Chess", "Rewon Child",
      "Scott Gray", "Alec Radford", "Jeffrey Wu", "Dario Amodei"]),
    ("autor2013", "Autor, Dorn, and Hanson - 2013 - The China Syndrome - Local Labor Market Effects of Import Competition in the United States.pdf", 48,
     "The China Syndrome: Local Labor Market Effects of Import Competition in the United States",
     ["David H. Autor", "David Dorn", "Gordon H. Hanson"]),
    ("shannon1948", "Shannon - 1948 - A Mathematical Theory of Communication.pdf", 55,
     "A Mathematical Theory of Communication", ["Claude E. Shannon"]),
    ("clark1998", "Clark and Chalmers - 1998 - The Extended Mind.pdf", 14,
     "The Extended Mind", ["Andy Clark", "David Chalmers"]),
    ("dijkstra1968", "Dijkstra - 1968 - GOTO Statements Considered Harmful.pdf", 4,
     "Go To Statement Considered Harmful", ["Edsger W. Dijkstra"]),
    ("granger1974", "Granger and Newbold - 1973 - Spurious Regressions in Econometrics.pdf", 10,
     "Spurious Regressions in Econometrics", ["C.W.J. Granger", "P. Newbold"]),
    ("box1976", "Box - 1976 - Science and Statistics.pdf", 10,
     "Science and Statistics", ["George E. P. Box"]),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--library", type=Path, default=LIBRARY)
    parser.add_argument("--sample", type=Path, default=Path("eval/spike/sample.json"))
    parser.add_argument("--oracles", type=Path, default=Path("eval/spike/ground-truth/oracles"))
    args = parser.parse_args()
    manifest = json.loads(args.sample.read_text())
    by_stem = {paper["stem"]: paper for paper in manifest["papers"]}
    for stem, filename, pages, title, authors in LOCAL:
        pdf = args.library / filename
        if not pdf.exists():
            raise SystemExit(f"missing local paper: {pdf}")
        checked = args.oracles / f"{stem}.json"
        oracle = (
            json.loads(checked.read_text())
            if checked.exists()
            else {"title": title, "authors": authors, "abstract": None}
        )
        by_stem[stem] = {
            "stem": stem,
            "pdf": str(pdf),
            "pages": pages,
            "local": True,
            "oracle": oracle,
            "oracle_source": str(checked) if checked.exists() else "hand-checked title and authors",
            "object_truth": None,
        }
    manifest["papers"] = list(by_stem.values())
    manifest["local_additions"] = [stem for stem, *_ in LOCAL]
    args.sample.write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"{len(manifest['papers'])} papers -> {args.sample}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
