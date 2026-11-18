#!/usr/bin/env python3
"""Download the review set's PDFs, and optionally their neighbours.

Reads eval/review/papers.json, tries each candidate open-access URL in order,
keeps a download only when it is a real PDF, and prints a table of what was
fetched, what was already present, what failed, and what needs to be dropped
in by hand (paywalled papers with no open copy).

Usage, from the repository root:

    python3 scripts/review/fetch_papers.py --target ~/Papers/review-set
    python3 scripts/review/fetch_papers.py --target ~/Papers/review-set --neighbours
    python3 scripts/review/fetch_papers.py --target ~/Papers/review-set --only card1990 --dry-run

Papers land at <target>/<filename>; neighbours at
<target>/neighbours/<paper id>/<neighbour id>.pdf. Point a Lysilogy library
folder at <target> (the home page's folder list) so the review panel can find
the papers.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Iterable

DEFAULT_MANIFEST = Path(__file__).resolve().parents[2] / "eval" / "review" / "papers.json"
# A generic client name: no personal contact details leave this machine.
USER_AGENT = "Mozilla/5.0 (compatible; paper-fetch/1.0)"
MAX_BYTES = 80 * 1024 * 1024

FETCHED = "fetched"
PRESENT = "present"
FAILED = "failed"
MANUAL = "needs-manual"
PLANNED = "planned"


@dataclass
class Job:
    """One PDF to obtain."""

    key: str
    title: str
    urls: list[str]
    path: Path
    hint: str = ""


@dataclass
class Outcome:
    job: Job
    status: str
    detail: str = ""
    tried: list[str] = field(default_factory=list)


class ManifestError(ValueError):
    pass


def load_manifest(path: Path) -> dict:
    with path.open(encoding="utf-8") as handle:
        manifest = json.load(handle)
    validate_manifest(manifest)
    return manifest


def validate_manifest(manifest: dict) -> None:
    """Reject a manifest the panel and this script could not use."""
    papers = manifest.get("papers")
    if not isinstance(papers, list) or not papers:
        raise ManifestError("manifest has no papers")
    seen: set[str] = set()
    for paper in papers:
        paper_id = paper.get("id")
        if not isinstance(paper_id, str) or not paper_id:
            raise ManifestError("a paper is missing its id")
        if paper_id in seen:
            raise ManifestError(f"duplicate paper id {paper_id}")
        seen.add(paper_id)
        for key in ("title", "filename"):
            if not isinstance(paper.get(key), str) or not paper[key].strip():
                raise ManifestError(f"{paper_id} is missing {key}")
        if not paper["filename"].lower().endswith(".pdf") or "/" in paper["filename"]:
            raise ManifestError(f"{paper_id} filename must be a bare .pdf name")
        if not isinstance(paper.get("urls", []), list):
            raise ManifestError(f"{paper_id} urls must be a list")
        neighbour_ids: set[str] = set()
        for neighbour in paper.get("neighbours", []):
            neighbour_id = neighbour.get("id")
            if not isinstance(neighbour_id, str) or not neighbour_id:
                raise ManifestError(f"{paper_id} has a neighbour without an id")
            if neighbour_id in neighbour_ids:
                raise ManifestError(f"{paper_id} lists neighbour {neighbour_id} twice")
            neighbour_ids.add(neighbour_id)
            if not str(neighbour.get("why", "")).strip():
                raise ManifestError(f"{paper_id}/{neighbour_id} needs a one-line why")


def candidate_urls(entry: dict) -> list[str]:
    """The entry's own URLs, then an arXiv PDF URL when it has an arXiv id."""
    urls = [url for url in entry.get("urls", []) if isinstance(url, str) and url.startswith("https://")]
    arxiv = entry.get("arxiv")
    if isinstance(arxiv, str) and arxiv.strip():
        derived = f"https://arxiv.org/pdf/{arxiv.strip()}"
        if derived not in urls:
            urls.append(derived)
    return urls


def manual_hint(entry: dict) -> str:
    if entry.get("doi"):
        return f"doi:{entry['doi']}"
    if entry.get("jstor"):
        return f"jstor:{entry['jstor']}"
    return "search by title"


def plan_jobs(manifest: dict, target: Path, include_neighbours: bool, only: Iterable[str] = ()) -> list[Job]:
    wanted = set(only)
    jobs: list[Job] = []
    for paper in manifest["papers"]:
        if wanted and paper["id"] not in wanted:
            continue
        jobs.append(Job(paper["id"], paper["title"], candidate_urls(paper), target / paper["filename"], manual_hint(paper)))
        if not include_neighbours:
            continue
        for neighbour in paper.get("neighbours", []):
            jobs.append(
                Job(
                    f"{paper['id']}/{neighbour['id']}",
                    neighbour.get("title", neighbour["id"]),
                    candidate_urls(neighbour),
                    target / "neighbours" / paper["id"] / f"{neighbour['id']}.pdf",
                    manual_hint(neighbour),
                )
            )
    return jobs


def is_pdf(data: bytes) -> bool:
    """PDF files start with %PDF-, possibly after a little leading junk."""
    return b"%PDF-" in data[:1024]


def file_is_pdf(path: Path) -> bool:
    try:
        with path.open("rb") as handle:
            return is_pdf(handle.read(1024))
    except OSError:
        return False


def http_fetch(url: str, timeout: float) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT, "Accept": "application/pdf,*/*;q=0.5"})
    with urllib.request.urlopen(request, timeout=timeout) as response:  # noqa: S310 - https only, see candidate_urls
        data = response.read(MAX_BYTES + 1)
    if len(data) > MAX_BYTES:
        raise ValueError("larger than the size limit")
    return data


def write_atomic(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.part")
    temporary.write_bytes(data)
    os.replace(temporary, path)


def obtain(job: Job, fetch: Callable[[str], bytes], dry_run: bool = False) -> Outcome:
    if file_is_pdf(job.path):
        return Outcome(job, PRESENT)
    if not job.urls:
        return Outcome(job, MANUAL, f"no open copy known ({job.hint}); save it as {job.path}")
    if dry_run:
        return Outcome(job, PLANNED, job.urls[0])
    tried: list[str] = []
    for url in job.urls:
        try:
            data = fetch(url)
        except (urllib.error.URLError, OSError, ValueError) as error:
            tried.append(f"{url}: {error}")
            continue
        if not is_pdf(data):
            tried.append(f"{url}: not a PDF")
            continue
        write_atomic(job.path, data)
        return Outcome(job, FETCHED, url, tried)
    return Outcome(job, FAILED, f"{tried[-1]}; save it by hand as {job.path}", tried)


def format_table(outcomes: list[Outcome]) -> str:
    rows = [("status", "paper", "detail")]
    rows += [(outcome.status, outcome.job.key, outcome.detail) for outcome in outcomes]
    width = [max(len(row[column]) for row in rows) for column in range(2)]
    lines = [f"{row[0]:<{width[0]}}  {row[1]:<{width[1]}}  {row[2]}".rstrip() for row in rows]
    lines.insert(1, f"{'-' * width[0]}  {'-' * width[1]}  ------")
    counts: dict[str, int] = {}
    for outcome in outcomes:
        counts[outcome.status] = counts.get(outcome.status, 0) + 1
    lines.append("")
    lines.append(", ".join(f"{count} {status}" for status, count in sorted(counts.items())))
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--target", type=Path, required=True, help="directory to save PDFs into")
    parser.add_argument("--neighbours", action="store_true", help="also fetch each paper's surrounding papers")
    parser.add_argument("--only", action="append", default=[], help="fetch only this paper id (repeatable)")
    parser.add_argument("--dry-run", action="store_true", help="show what would be fetched")
    parser.add_argument("--timeout", type=float, default=60.0)
    args = parser.parse_args(argv)

    manifest = load_manifest(args.manifest)
    jobs = plan_jobs(manifest, args.target.expanduser(), args.neighbours, args.only)
    if args.only and not jobs:
        print(f"no paper matches {', '.join(args.only)}", file=sys.stderr)
        return 2
    outcomes = [obtain(job, lambda url: http_fetch(url, args.timeout), args.dry_run) for job in jobs]
    print(format_table(outcomes))
    return 1 if any(outcome.status == FAILED for outcome in outcomes) else 0


if __name__ == "__main__":
    sys.exit(main())
