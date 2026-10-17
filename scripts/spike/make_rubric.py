#!/usr/bin/env python3
"""Write the human grading sheet for a spike run.

Five papers spread across the sample (theory, math, ML, robotics, applied)
are rendered with the fields that have no automatic truth: thesis and read
signal, question, claims, method, limitations, supercut, and relations.
Each field gets a 1-5 score line and a notes line.

Usage: python3 -B scripts/spike/make_rubric.py --run <name>
"""
import argparse
import json
from pathlib import Path

PAPERS = ("0812.5080v5", "1912.01121v4", "1911.11582v1", "1910.06658v2", "2001.11761v3")

SCALE = """Scale: 5 = correct and complete, what an expert would write; 4 = correct, minor gaps;
3 = mostly right, a real omission or one wrong detail; 2 = misleading or largely incomplete;
1 = wrong. Score against the PDF, not against your expectations of the field."""


def grade(label):
    return f"\n**{label}** score (1-5): ___  \nnotes: \n"


def bullets(items, render):
    return "\n".join(f"- {render(item)}" for item in items) or "- (none)"


def evidence(item):
    quotes = item.get("evidence") or []
    if not quotes:
        return ""
    first = quotes[0]
    return f' — p{first.get("page")}: "{first.get("quote", "")[:160]}"'


def paper_section(stem, artifacts, record):
    identity = artifacts.get("identity") or {}
    card = artifacts.get("card") or {}
    read = card.get("read_signal") or {}
    method = artifacts.get("method") or {}
    lines = [
        f"## {identity.get('title', stem)}",
        f"`{stem}` · {record.get('pages')} pages · type: {(artifacts.get('type') or {}).get('value')} · "
        f"${record.get('totals', {}).get('cost_usd', 0):.3f}",
        "",
        f"**Thesis:** {(card.get('thesis') or {}).get('text', '')}",
        f"**Read signal (model judgment):** {read.get('judgment')} — {read.get('reason', '')}",
        grade("Thesis and read signal"),
        f"**Question:** {(artifacts.get('question') or {}).get('text', '')}",
        grade("Question"),
        "**Claims:**",
        bullets(artifacts.get("claims") or [], lambda claim: f"[{claim.get('strength')}] {claim.get('statement')} (support: {claim.get('support')}){evidence(claim)}"),
        grade("Claims and evidence map"),
        "**Method:**",
        f"- setting: {method.get('setting')}",
        f"- data: {method.get('data')}",
        f"- procedure: {method.get('procedure')}",
        f"- assumptions: {'; '.join(method.get('assumptions') or []) or '(none)'}",
        grade("Method card"),
        "**Limitations:**",
        bullets(artifacts.get("limitations") or [], lambda item: f"[{item.get('stated_by')}] {item.get('text')}"),
        "**Open questions:**",
        bullets(artifacts.get("open_questions") or [], lambda item: f"[{item.get('stated_by')}] {item.get('text')}"),
        grade("Limitations and open questions"),
        "**Supercut** (quotes in quotation marks are the paper's own words, admitted verbatim):",
        "",
        " ".join(
            f'"{segment["text"]}" (p{segment.get("page")})' if segment.get("kind") == "source" else f"*{segment['text']}*"
            for segment in artifacts.get("supercut") or []
        ),
        grade("Supercut coverage (does it carry the argument?)"),
        "**Relations:**",
        bullets(artifacts.get("relations") or [], lambda item: f"{item.get('relation')} → [{item.get('reference_key')}]: {item.get('why')}"),
        grade("Relation agreement"),
        "---",
    ]
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--run", required=True)
    args = parser.parse_args()
    run_dir = Path("eval/spike/runs") / args.run
    sections = [f"# Extraction spike grading sheet — {args.run}", "", SCALE, ""]
    for stem in PAPERS:
        path = run_dir / "results" / f"{stem}.json"
        if not path.exists():
            sections.append(f"## {stem}\n\n(no result)\n\n---")
            continue
        record = json.loads(path.read_text())
        sections.append(paper_section(stem, record.get("artifacts") or {}, record))
    out = run_dir / "rubric.md"
    out.write_text("\n".join(sections) + "\n")
    print(out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
