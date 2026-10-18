#!/usr/bin/env python3
"""Run `lysilogy extract` over the frozen spike sample.

Each paper writes eval/spike/runs/<run>/results/<stem>.json; every call is
billed to eval/spike/runs/<run>/spend.jsonl. Papers run a few at a time.
The per-paper budget is enforced inside extract; the run budget is enforced
per process from the ledger total at its start, so with concurrency it is a
soft cap of roughly budget + (jobs - 1) * paper budget.

Needs OPENROUTER_API_KEY in the environment.
Usage: python3 -B scripts/spike/run_sample.py --run <name> [--model M] [--passes 2] [--jobs 4]
"""
import argparse
import json
import os
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path


def run_one(paper, args, run_dir, binary):
    out = run_dir / "results" / f"{paper['stem']}.json"
    if out.exists() and not args.force:
        return paper["stem"], "skipped (exists)", 0.0
    log = run_dir / "logs" / f"{paper['stem']}.log"
    started = time.monotonic()
    with log.open("w") as handle:
        status = subprocess.run(
            [
                str(binary),
                "--library", str(run_dir / "empty-library"),
                "--data", str(Path.home() / ".cache/lysilogy/spike/data"),
                "extract", "--pdf", paper["pdf"],
                "--model", args.model,
                "--passes", str(args.passes),
                "--paper-budget-usd", str(args.paper_budget),
                "--objects", args.objects,
                "--object-pages", str(args.object_pages),
                "--work", str(Path.home() / ".cache/lysilogy/spike/work" / paper["stem"]),
                "--out", str(out),
            ],
            stdout=handle, stderr=subprocess.STDOUT,
            env={
                **os.environ,
                "LYSILOGY_SPEND_LEDGER": str(run_dir / "spend.jsonl"),
                "LYSILOGY_MODEL_BUDGET_USD": str(args.budget),
            },
        ).returncode
    return paper["stem"], "ok" if status == 0 else f"exit {status}", time.monotonic() - started


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--run", required=True)
    parser.add_argument("--model", default="deepseek/deepseek-v4.1-flash")
    parser.add_argument("--passes", type=int, default=2, choices=(1, 2))
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("--budget", type=float, default=5.0)
    parser.add_argument("--paper-budget", type=float, default=0.15)
    parser.add_argument("--objects", choices=("chunked", "inline"), default="chunked")
    parser.add_argument("--object-pages", type=int, default=6)
    parser.add_argument("--sample", type=Path, default=Path("eval/spike/sample.json"))
    parser.add_argument("--binary", type=Path, default=Path("target/release/lysilogy"))
    parser.add_argument("--force", action="store_true", help="rerun papers that already have a result")
    args = parser.parse_args()
    if not os.environ.get("OPENROUTER_API_KEY"):
        sys.exit("OPENROUTER_API_KEY is not set")
    papers = json.loads(args.sample.read_text())["papers"]
    run_dir = Path("eval/spike/runs") / args.run
    for sub in ("results", "logs", "empty-library"):
        (run_dir / sub).mkdir(parents=True, exist_ok=True)
    (run_dir / "run.json").write_text(json.dumps({
        "run": args.run, "model": args.model, "passes": args.passes, "jobs": args.jobs,
        "budget_usd": args.budget, "paper_budget_usd": args.paper_budget,
        "objects": args.objects, "object_pages": args.object_pages,
        "sample": str(args.sample), "started": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
    }, indent=2) + "\n")
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for stem, status, seconds in pool.map(lambda paper: run_one(paper, args, run_dir, args.binary.resolve()), papers):
            print(f"{stem:16} {status:18} {seconds:6.0f}s", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
