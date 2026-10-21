#!/usr/bin/env python3
"""Score an extraction-spike run against the oracles.

Truth:
- identity/abstract: TeX front matter (scripts/truth/latex/frontmatter.py)
- figures/tables/algorithms/equations: hand-verified eval/spike/inventory/
- references: the source parser's bibliography count (uncertified, labelled)
- quotes: admission outcomes recorded by `lysilogy extract`

Pass 1 and the final (self-checked, admitted) artifacts are both scored, so
the self-check delta is measured on the same truth.

Usage: python3 -B scripts/spike/benchmark.py --run <name>
"""
import argparse
import difflib
import json
import math
import re
import statistics
import sys
import unicodedata
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "truth" / "latex"))

from archive import UnsupportedSource, read_archive  # noqa: E402
from frontmatter import front_matter_from_source  # noqa: E402
from parser import parse_project  # noqa: E402

KINDS = ("figure", "table", "algorithm", "equation")
ABSTRACT_MATCH_F1 = 0.95

# A paper is "confused" at a stage when the model spends at least this many
# reasoning tokens there and at least this multiple of the cohort's median.
CONFUSED_FACTOR = 2.0
CONFUSED_FLOOR = 8_000


def fold(text):
    text = unicodedata.normalize("NFKD", text or "")
    text = "".join(char for char in text if not unicodedata.combining(char))
    for special, plain in (("ı", "i"), ("ł", "l"), ("Ł", "L"), ("ø", "o"), ("ß", "ss"),
                           ("đ", "d"), ("ð", "d"), ("þ", "th"), ("æ", "ae"), ("œ", "oe"), ("ɡ", "g")):
        text = text.replace(special, plain)
    return text.lower()


def compact(text):
    return re.sub(r"[^a-z0-9]+", "", fold(text))


def words(text):
    """Alphabetic words of two or more letters: math symbols and variables drop out."""
    return re.findall(r"[a-z]{2,}", fold(text))


def token_f1(predicted, truth):
    predicted, truth = Counter(words(predicted)), Counter(words(truth))
    overlap = sum((predicted & truth).values())
    if not overlap:
        return 0.0
    precision = overlap / sum(predicted.values())
    recall = overlap / sum(truth.values())
    return 2 * precision * recall / (precision + recall)


def name_key(name):
    """Family name plus first given initial, so 'J. Smith' and 'John Smith' agree."""
    name = re.sub(r"\b([A-Za-z])\s+([A-Z]{2,})\b", r"\1\2", name or "")
    parts = [part for part in re.split(r"[\s.]+", fold(name)) if part]
    if not parts:
        return ""
    family = re.sub(r"[^a-z]", "", parts[-1])
    initial = re.sub(r"[^a-z]", "", parts[0])[:1] if len(parts) > 1 else ""
    return f"{family}:{initial}"


def author_name(author):
    """An author entry, whether the model wrote an object or a bare name."""
    return author.get("name", "") if isinstance(author, dict) else str(author)


def loose(value):
    """A comparison key that survives case, accents, spacing and ligatures."""
    text = unicodedata.normalize("NFKD", value or "")
    text = "".join(char for char in text if not unicodedata.combining(char)).casefold()
    for special, plain in (("ı", "i"), ("ł", "l"), ("ø", "o"), ("ß", "ss"), ("đ", "d"), ("ð", "d"),
                           ("þ", "th"), ("æ", "ae"), ("œ", "oe"), ("ɡ", "g")):
        text = text.replace(special, plain)
    return re.sub(r"[^a-z0-9]+", " ", text).strip()


def similar(left, right):
    """A 0..1 edit ratio on the loose keys."""
    return difflib.SequenceMatcher(None, loose(left), loose(right)).ratio()


def authors_match(predicted, truth):
    """Order-insensitive: each truth name has a distinct close predicted name."""
    predicted = [loose(author_name(author)) for author in predicted]
    truth = [loose(name) for name in truth]
    if len(predicted) != len(truth):
        return False
    used = set()
    for name in truth:
        for index, candidate in enumerate(predicted):
            if index not in used and similar(name, candidate) >= 0.85:
                used.add(index)
                break
        else:
            return False
    return True


def score_identity(artifacts, oracle):
    identity = (artifacts or {}).get("identity") or {}
    if not isinstance(identity, dict):
        identity = {}
    result = {}
    if oracle.get("title"):
        title = identity.get("title") or ""
        result["title_exact"] = compact(title) == compact(oracle["title"])
        result["title_f1"] = round(token_f1(title, oracle["title"]), 4)
        result["title_matchable"] = result["title_exact"] or similar(title, oracle["title"]) >= 0.95
    if oracle.get("authors"):
        predicted = [name_key(author_name(author)) for author in identity.get("authors") or []]
        truth = [name_key(name) for name in oracle["authors"]]
        overlap = sum((Counter(predicted) & Counter(truth)).values())
        result["authors_ordered_exact"] = predicted == truth
        result["authors_set_exact"] = Counter(predicted) == Counter(truth)
        result["authors_matchable"] = result["authors_set_exact"] or authors_match(identity.get("authors") or [], oracle["authors"])
        result["authors_f1"] = round(2 * overlap / (len(predicted) + len(truth)), 4) if predicted or truth else 1.0
        result["authors_count_error"] = len(predicted) - len(truth)
    if oracle.get("abstract"):
        abstract = ((artifacts or {}).get("abstract") or {}).get("text") or ""
        f1 = token_f1(abstract, oracle["abstract"])
        result["abstract_f1"] = round(f1, 4)
        result["abstract_match"] = f1 >= ABSTRACT_MATCH_F1
        result["abstract_near"] = f1 >= 0.90
        result["abstract_exact_words"] = words(abstract) == words(oracle["abstract"])
    return result


def interval_iou(left, right):
    """Page-interval intersection over union, inclusive of both ends."""
    start, end = max(left[0], right[0]), min(left[1], right[1])
    if end < start:
        return 0.0
    intersection = end - start + 1
    union = (left[1] - left[0] + 1) + (right[1] - right[0] + 1) - intersection
    return intersection / union if union else 0.0


def score_sections_oracle(artifacts, oracle):
    """Align predicted units to the audited spans: count, IoU, exact boundaries."""
    truth = []
    for section in oracle.get("sections") or []:
        pages = section.get("pages") or {}
        if pages.get("start") and pages.get("end"):
            truth.append((pages["start"], pages["end"]))
    if not truth:
        return {}
    predicted = []
    for section in (artifacts or {}).get("sections") or []:
        pages = section.get("pages") if isinstance(section, dict) else None
        if isinstance(pages, dict) and pages.get("start") and pages.get("end"):
            predicted.append((pages["start"], pages["end"]))
    best = [max((interval_iou(unit, guess) for guess in predicted), default=0.0) for unit in truth]
    return {
        "truth_units": len(truth),
        "predicted_units": len(predicted),
        "unit_count_error": len(predicted) - len(truth),
        "unit_iou": round(statistics.mean(best), 4),
        "boundary_exact": round(sum(1 for unit in truth if unit in predicted) / len(truth), 4),
    }


def identity_resolvable(score):
    """The exact and perfect-enough verdicts across the identity fields present."""
    exact = [value for key, value in score.items() if key in ("title_exact", "authors_set_exact", "abstract_match")]
    matchable = [value for key, value in score.items() if key in ("title_matchable", "authors_matchable", "abstract_match")]
    return (None if not exact else all(exact), None if not matchable else all(matchable))


def object_number(item):
    raw = item.get("number") or ""
    if not raw:
        match = re.search(r"([A-Z]?\.?\d+(?:\.\d+)*[a-z]?|[IVXL]+)\s*\)?\s*$", item.get("label") or "")
        raw = match[1] if match else ""
    return re.sub(r"[\s()]|^(?:eq|eqn|equation|fig|figure|table|algorithm)\.?", "", raw, flags=re.IGNORECASE).rstrip(".").upper()


def score_objects(artifacts, inventory):
    objects = (artifacts or {}).get("objects") or []
    result = {}
    for kind in KINDS:
        truth = {item["number"].upper(): item["page"] for item in inventory.get(f"{kind}s", [])}
        predicted_items = [item for item in objects if item.get("kind") == kind]
        numbered = {}
        for item in predicted_items:
            number = object_number(item)
            if number:
                numbered.setdefault(number, item.get("page"))
        matched = set(numbered) & set(truth)
        result[kind] = {
            "truth": len(truth),
            "predicted": len(predicted_items),
            "predicted_numbered": len(numbered),
            "count_error": len(numbered) - len(truth),
            "matched": len(matched),
            "precision": round(len(matched) / len(numbered), 4) if numbered else (1.0 if not truth else 0.0),
            "recall": round(len(matched) / len(truth), 4) if truth else 1.0,
            "page_correct": sum(1 for number in matched if numbered[number] == truth[number]),
            "missing": sorted(set(truth) - matched)[:20],
            "extra": sorted(set(numbered) - set(truth))[:20],
        }
    return result


def bibliography_count(paper):
    try:
        files, _ = read_archive(Path(paper["source"]).read_bytes())
        return len(parse_project(files, selected_main=paper["main"])["entries"])
    except (UnsupportedSource, OSError):
        return None


def quote_stats(record):
    by_kind = {}
    for outcome in record.get("admission") or []:
        kind = by_kind.setdefault(outcome["kind"], Counter())
        kind[outcome["status"]] += 1
    return {kind: dict(counts) for kind, counts in by_kind.items()}


def reasoning_profile(record, result_file=None):
    """Reasoning tokens per stage, and trace characters when the trace exists."""
    by_stage = Counter()
    completion = 0
    for call in record.get("calls") or []:
        by_stage[call["stage"]] += call.get("reasoning_tokens") or 0
        completion += call.get("completion_tokens") or 0
    total = sum(by_stage.values())
    path = None
    if record.get("reasoning_trace"):
        path = Path(record["reasoning_trace"])
    elif result_file is not None:
        result_file = Path(result_file)
        path = result_file.with_name(f"{result_file.stem}.reasoning.jsonl")
    chars = None
    chars_by_stage = {}
    if path is not None and path.exists():
        for line in path.read_text().splitlines():
            if not line.strip():
                continue
            trace = json.loads(line)
            chars_by_stage[trace["stage"]] = chars_by_stage.get(trace["stage"], 0) + len(trace["text"])
        chars = sum(chars_by_stage.values())
    return {
        "tokens": total,
        "by_stage": dict(by_stage),
        "completion_tokens": completion,
        "share": round(total / completion, 3) if completion else None,
        "chars": chars,
        "chars_by_stage": chars_by_stage,
    }


def score_paper(paper, record, oracle, inventory, bibliography, result_file=None):
    final = record.get("artifacts")
    pass1 = record.get("pass1")
    final_identity = score_identity(final, oracle)
    exact, matchable = identity_resolvable(final_identity)
    corrections = Counter(outcome["status"] for outcome in record.get("correction_outcomes") or [])
    return {
        "stem": paper["stem"],
        "pages": record.get("pages"),
        "succeeded": bool(final) and not record.get("failures"),
        "failures": record.get("failures") or [],
        "budget_stopped": record.get("budget_stopped"),
        "cost_usd": record.get("totals", {}).get("cost_usd"),
        "wall_s": record.get("totals", {}).get("wall_s"),
        "calls": [
            {key: call.get(key) for key in ("stage", "max_tokens", "prompt_tokens", "cached_tokens", "completion_tokens", "reasoning_tokens", "cost_usd", "latency_s", "finish_reason", "cost_source")}
            for call in record.get("calls") or []
        ],
        "identity": {"pass1": score_identity(pass1, oracle), "final": final_identity},
        "identity_exact": exact,
        "identity_matchable": matchable,
        "sections_oracle": score_sections_oracle(final, oracle),
        "objects": {"pass1": score_objects(pass1, inventory), "final": score_objects(final, inventory)},
        "references": {
            "predicted": len((final or {}).get("references") or []),
            "source_parser": bibliography,
        },
        "sections": score_sections(final, record.get("pages")),
        "counts": {key: len((final or {}).get(key) or []) for key in ("sections", "supercut", "claims", "key_numbers", "citations", "relations", "notation", "glossary")},
        "type": ((final or {}).get("type") or {}).get("value"),
        "read_signal": (((final or {}).get("card") or {}).get("read_signal") or {}).get("judgment"),
        "features": record.get("features", ""),
        "core_mode": record.get("core_mode", "combined"),
        "reasoning": record.get("reasoning", ""),
        "reasoning_trace": record.get("reasoning_trace"),
        "reasoning_profile": reasoning_profile(record, result_file),
        "object_chunks": len(record.get("object_chunks") or []),
        "object_retries": sum(1 for chunk in record.get("object_chunks") or [] if chunk.get("retried")),
        "recovered_failures": len(record.get("recovered_failures") or []),
        "object_merge": record.get("object_merge"),
        "pass1_check": record.get("pass1_check"),
        "pass2_check": record.get("pass2_check"),
        "corrections": dict(corrections),
        "quotes": quote_stats(record),
    }


def mean(values):
    values = [value for value in values if value is not None]
    return round(statistics.mean(values), 4) if values else None


def wilson(successes, total, z=1.96):
    """A 95% Wilson interval for a rate, which behaves at 0 and 1."""
    if not total:
        return None
    p = successes / total
    denom = 1 + z * z / total
    center = (p + z * z / (2 * total)) / denom
    half = z * math.sqrt(p * (1 - p) / total + z * z / (4 * total * total)) / denom
    return {
        "n": successes,
        "total": total,
        "rate": round(p, 4),
        "lower": round(max(0.0, center - half), 4),
        "upper": round(min(1.0, center + half), 4),
    }


def score_sections(artifacts, pages):
    """Section count, page coverage, and the defects the design names."""
    sections = (artifacts or {}).get("sections") or []
    result = {
        "count": len(sections),
        "coverage": 0.0,
        "last_page": 0,
        "short_units": 0,
        "long_units": 0,
        "malformed": 0,
    }
    if not sections:
        return result
    for section in sections:
        span = section.get("pages") if isinstance(section, dict) else None
        if not isinstance(span, dict):
            result["malformed"] += 1
            continue
        start = span.get("start") or 0
        end = span.get("end") or 0
        result["last_page"] = max(result["last_page"], end)
        length = end - start + 1
        if length < 1:
            result["short_units"] += 1
        elif length > 5:
            result["long_units"] += 1
    if pages:
        result["coverage"] = round(result["last_page"] / pages, 4)
    return result


def worst_case(rows):
    """Papers that miss a floor, not just lower the average."""
    return {
        "title": [row["stem"] for row in rows if row.get("identity") and row["identity"]["final"].get("title_exact") is False],
        "authors": [row["stem"] for row in rows if row.get("identity") and row["identity"]["final"].get("authors_ordered_exact") is False],
        "abstract": [row["stem"] for row in rows if row.get("identity") and row["identity"]["final"].get("abstract_match") is False],
        "identity_exact": [row["stem"] for row in rows if row.get("identity_exact") is False],
        "identity_matchable": [row["stem"] for row in rows if row.get("identity_matchable") is False],
        "sections": [
            row["stem"]
            for row in rows
            if row.get("sections") and (row["sections"]["coverage"] < 0.95 or row["sections"]["malformed"] > 0)
        ],
    }


def rate(values):
    values = [bool(value) for value in values if value is not None]
    return f"{sum(values)}/{len(values)}" if values else "-"


def mark_confused(rows, factor=CONFUSED_FACTOR, floor=CONFUSED_FLOOR):
    """Flag the stages where a paper spends far more reasoning than its cohort."""
    profiles = [row["reasoning_profile"] for row in rows if row.get("reasoning_profile")]
    stages = set()
    for profile in profiles:
        stages.update(profile["by_stage"])
    medians = {}
    for stage in stages:
        values = [profile["by_stage"][stage] for profile in profiles if stage in profile["by_stage"]]
        medians[stage] = statistics.median(values) if values else 0
    for row in rows:
        profile = row.get("reasoning_profile")
        if not profile:
            row["confused"] = None
            continue
        flagged = []
        for stage, tokens in profile["by_stage"].items():
            median = medians.get(stage, 0)
            if tokens >= max(floor, factor * median):
                flagged.append({
                    "stage": stage,
                    "tokens": tokens,
                    "median": median,
                    "ratio": round(tokens / median, 2) if median else None,
                })
        row["confused"] = bool(flagged)
        row["confused_stages"] = flagged
    return medians


def aggregate(rows):
    scored = [row for row in rows if row.get("identity")]
    summary: dict = {"papers": len(rows), "succeeded": sum(1 for row in rows if row.get("succeeded"))}
    for stage in ("pass1", "final"):
        identity = [row["identity"][stage] for row in scored]
        summary[f"identity_{stage}"] = {
            "title_exact": rate(item.get("title_exact") for item in identity),
            "title_matchable": rate(item.get("title_matchable") for item in identity),
            "authors_ordered_exact": rate(item.get("authors_ordered_exact") for item in identity),
            "authors_set_exact": rate(item.get("authors_set_exact") for item in identity),
            "authors_matchable": rate(item.get("authors_matchable") for item in identity),
            "authors_f1": mean(item.get("authors_f1") for item in identity),
            "abstract_match": rate(item.get("abstract_match") for item in identity),
            "abstract_near": rate(item.get("abstract_near") for item in identity),
            "abstract_f1": mean(item.get("abstract_f1") for item in identity),
        }
        objects = {}
        for kind in KINDS:
            stats = [row["objects"][stage][kind] for row in scored]
            truth = sum(item["truth"] for item in stats)
            numbered = sum(item["predicted_numbered"] for item in stats)
            matched = sum(item["matched"] for item in stats)
            objects[kind] = {
                "truth": truth,
                "predicted_numbered": numbered,
                "matched": matched,
                "precision": round(matched / numbered, 4) if numbered else None,
                "recall": round(matched / truth, 4) if truth else None,
                "exact_count_papers": f"{sum(1 for item in stats if item['count_error'] == 0)}/{len(stats)}",
                "page_correct_of_matched": f"{sum(item['page_correct'] for item in stats)}/{matched}",
            }
        summary[f"objects_{stage}"] = objects
    quotes = Counter()
    for row in scored:
        for counts in row["quotes"].values():
            quotes.update(counts)
    summary["quotes_final"] = dict(quotes)
    raw = Counter()
    for row in scored:
        for key, value in (row.get("pass1_check") or {}).items():
            raw[key] += value
    summary["quotes_pass1_raw"] = dict(raw)
    corrections = Counter()
    for row in scored:
        corrections.update(row["corrections"])
    summary["corrections"] = dict(corrections)
    costs = [row["cost_usd"] for row in scored if row.get("cost_usd") is not None]
    walls = [row["wall_s"] for row in scored if row.get("wall_s") is not None]
    summary["cost_usd"] = {"total": round(sum(costs), 4), "mean": mean(costs), "max": max(costs) if costs else None}
    summary["wall_s"] = {"mean": mean(walls), "max": max(walls) if walls else None}
    stage_costs = Counter()
    for row in scored:
        for call in row["calls"]:
            stage_costs[call["stage"]] += call["cost_usd"]
    summary["cost_by_stage"] = {stage: round(value, 4) for stage, value in stage_costs.items()}
    prompt = sum(call.get("prompt_tokens") or 0 for row in scored for call in row["calls"])
    cached = sum(call.get("cached_tokens") or 0 for row in scored for call in row["calls"])
    summary["cached_tokens"] = {
        "cached": cached,
        "prompt": prompt,
        "hit_rate": round(cached / prompt, 4) if prompt else None,
    }
    summary["failures"] = [(row["stem"], failure["stage"], failure["error"][:120]) for row in rows for failure in row.get("failures") or []]
    summary["object_chunks"] = {
        "chunks": sum(row.get("object_chunks", 0) for row in scored),
        "retries": sum(row.get("object_retries", 0) for row in scored),
        "recovered_failures": sum(row.get("recovered_failures", 0) for row in scored),
    }
    over = [row["stem"] for row in scored if (row.get("cost_usd") or 0) > 0.15]
    summary["papers_over_15_cents"] = over
    medians = mark_confused(rows)
    summary["reasoning_by_stage_median"] = {stage: value for stage, value in sorted(medians.items())}
    summary["confused_papers"] = [
        {"stem": row["stem"], "stages": row.get("confused_stages")}
        for row in rows
        if row.get("confused")
    ]
    summary["worst_case"] = worst_case(scored)
    def field_interval(key):
        values = [row["identity"]["final"] for row in scored if key in row["identity"]["final"]]
        return wilson(sum(1 for value in values if value.get(key)), len(values))

    summary["intervals"] = {
        "title_exact": field_interval("title_exact"),
        "authors_ordered_exact": field_interval("authors_ordered_exact"),
        "abstract_match": field_interval("abstract_match"),
        "title_matchable": field_interval("title_matchable"),
        "authors_set_exact": field_interval("authors_set_exact"),
        "authors_matchable": field_interval("authors_matchable"),
        "abstract_near": field_interval("abstract_near"),
        "identity_exact": wilson(
            sum(1 for row in scored if row.get("identity_exact")),
            len(scored),
        ),
        "identity_matchable": wilson(
            sum(1 for row in scored if row.get("identity_matchable")),
            len(scored),
        ),
    }
    coverages = [row["sections"]["coverage"] for row in scored if row.get("sections")]
    summary["sections"] = {
        "mean_coverage": mean(coverages),
        "papers_below_95": sum(1 for value in coverages if value < 0.95),
        "malformed_units": sum(row["sections"]["malformed"] for row in scored if row.get("sections")),
        "short_units": sum(row["sections"]["short_units"] for row in scored if row.get("sections")),
        "long_units": sum(row["sections"]["long_units"] for row in scored if row.get("sections")),
    }
    aligned = [row["sections_oracle"] for row in scored if row.get("sections_oracle")]
    summary["sections_oracle"] = {
        "papers": len(aligned),
        "mean_iou": mean(item["unit_iou"] for item in aligned),
        "mean_unit_count_error": mean(item["unit_count_error"] for item in aligned),
        "mean_boundary_exact": mean(item["boundary_exact"] for item in aligned),
        "truth_units": sum(item["truth_units"] for item in aligned),
        "predicted_units": sum(item["predicted_units"] for item in aligned),
    }
    summary["graph_counts"] = {
        "citations": sum((row.get("counts") or {}).get("citations", 0) for row in scored),
        "relations": sum((row.get("counts") or {}).get("relations", 0) for row in scored),
        "references": sum((row.get("references") or {}).get("predicted", 0) for row in scored),
    }
    return summary


def print_table(rows):
    print(f"{'stem':18} {'cost':>7} {'wall':>6} {'reason':>8} {'cache':>6} {'share':>6}  confused")
    for row in rows:
        profile = row.get("reasoning_profile") or {}
        if not row.get("identity"):
            print(f"{row['stem']:18} {'-':>7} {'-':>6} {'-':>8} {'-':>6} {'-':>6}  failed")
            continue
        flagged = ",".join(
            f"{entry['stage'].replace('extract.', '')}(x{entry['ratio']})" if entry["ratio"] else entry["stage"].replace("extract.", "")
            for entry in row.get("confused_stages") or []
        )
        marker = f"*{flagged}" if flagged else ""
        share = profile.get("share")
        prompt = sum(call.get("prompt_tokens") or 0 for call in row["calls"])
        cached = sum(call.get("cached_tokens") or 0 for call in row["calls"])
        hit = cached / prompt if prompt else 0
        print(
            f"{row['stem']:18} {row.get('cost_usd') or 0:7.4f} {row.get('wall_s') or 0:6.0f} "
            f"{profile.get('tokens', 0):8d} {hit:6.2f} {share if share is not None else 0:6.2f}  {marker}"
        )


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--run", required=True)
    parser.add_argument("--sample", type=Path, default=Path("eval/spike/sample.json"))
    parser.add_argument("--table", action="store_true", help="print a per-paper table after the summary")
    args = parser.parse_args()
    run_dir = Path("eval/spike/runs") / args.run
    rows = []
    for paper in json.loads(args.sample.read_text())["papers"]:
        result = run_dir / "results" / f"{paper['stem']}.json"
        if not result.exists():
            rows.append({"stem": paper["stem"], "succeeded": False, "failures": [{"stage": "run", "error": "no result file"}]})
            continue
        record = json.loads(result.read_text())
        oracle = paper.get("oracle") or front_matter_from_source(paper["source"], paper["main"])
        inventory = json.loads(Path(paper["object_truth"]).read_text()) if paper.get("object_truth") else {}
        bibliography = bibliography_count(paper) if paper.get("source") else None
        rows.append(score_paper(paper, record, oracle, inventory, bibliography, result))
    summary = aggregate(rows)
    (run_dir / "metrics.json").write_text(json.dumps({"summary": summary, "papers": rows}, indent=1) + "\n")
    print(json.dumps(summary, indent=1))
    if args.table:
        print()
        print_table(rows)
    return 0


if __name__ == "__main__":
    sys.exit(main())
