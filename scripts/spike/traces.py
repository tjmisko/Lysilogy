#!/usr/bin/env python3
"""Print the reasoning traces beside a spike run's result records.

Each result record names a `<stem>.reasoning.jsonl` written next to it, one
line per call: `{"stage": ..., "text": ...}`.

Usage:
  python3 -B scripts/spike/traces.py --run barebones-low-2026-10-19
  python3 -B scripts/spike/traces.py --run <run> --stage extract.pass2 --full
"""
import argparse
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--run", required=True, help="run name under eval/spike/runs")
    parser.add_argument("--stem", help="one paper only")
    parser.add_argument("--stage", help="one stage only")
    parser.add_argument("--chars", type=int, default=500, help="preview length per call")
    parser.add_argument("--full", action="store_true", help="print the whole trace")
    args = parser.parse_args()

    results = Path("eval/spike/runs") / args.run / "results"
    if not results.is_dir():
        raise SystemExit(f"no results directory: {results}")

    for result_file in sorted(results.glob("*.json")):
        if args.stem and result_file.stem != args.stem:
            continue
        result = json.loads(result_file.read_text())
        named = result.get("reasoning_trace")
        path = Path(named) if named else result_file.with_name(f"{result_file.stem}.reasoning.jsonl")
        if not path.exists():
            continue
        print(f"=== {result_file.stem} ===")
        for line in path.read_text().splitlines():
            trace = json.loads(line)
            if args.stage and trace["stage"] != args.stage:
                continue
            text = trace["text"]
            print(f"  [{trace['stage']}] {len(text)} chars")
            print(text if args.full else f"    {text[:args.chars]}{'…' if len(text) > args.chars else ''}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
