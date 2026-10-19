#!/usr/bin/env python3
"""Build the image-only test PDFs.

One real scanned PDF (Putnam 1975) is copied; three local text papers are
rasterized at 150 DPI so the image/OCR input dimension can be tested on known
content. Outputs land in eval/spike/image-only/ and are gitignored.

Usage: python3 -B scripts/spike/build_image_only.py
"""
import shutil
import subprocess
import tempfile
from pathlib import Path

LIBRARY = Path("/home/tjmisko/Projects/Lysilogy/local-articles")
OUT = Path("eval/spike/image-only")

RASTERIZE = [
    ("dijkstra1968", "Dijkstra - 1968 - GOTO Statements Considered Harmful.pdf"),
    ("box1976", "Box - 1976 - Science and Statistics.pdf"),
    ("granger1974", "Granger and Newbold - 1973 - Spurious Regressions in Econometrics.pdf"),
]
COPY = [("putnam1975", "Putnam - 1975 - The Meaning of ‘Meaning’.pdf")]


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for stem, filename in RASTERIZE:
        with tempfile.TemporaryDirectory() as directory:
            prefix = Path(directory) / "p"
            subprocess.run(
                ["pdftoppm", "-png", "-r", "150", str(LIBRARY / filename), str(prefix)],
                check=True,
            )
            pages = sorted(Path(directory).glob("p-*.png"))
            subprocess.run(["convert", *pages, str(OUT / f"{stem}.pdf")], check=True)
            print(f"{stem}: {len(pages)} pages")
    for stem, filename in COPY:
        shutil.copyfile(LIBRARY / filename, OUT / f"{stem}.pdf")
        print(f"{stem}: copied")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
