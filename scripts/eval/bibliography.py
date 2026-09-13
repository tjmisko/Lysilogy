#!/usr/bin/env python3
"""Measure O8/O9/O10 from independent K1/K2 labels, without network or models.

See eval/bibliography-contract.md. Synthetic unit fixtures never publish inputs.
"""
import argparse
from bisect import bisect_left
from collections import Counter, defaultdict, deque
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
import unicodedata

ROOT = Path(__file__).resolve().parents[2]
VERSION = "bibliography-v1"
FIELDS = ("title", "first_author", "year")
SOURCES = (
    "src/objects/bibliography.rs", "src/objects/mod.rs", "src/source_index.rs", "src/source_index/cache.rs",
    "src/domain.rs", "src/kb/names.rs", "src/kb/titles.rs", "examples/objects_fixture.rs",
    "scripts/eval/bibliography.py", "Cargo.toml", "Cargo.lock",
)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def safe_file(root, relative):
    path = Path(relative)
    if path.is_absolute() or not path.parts or any(part in (".", "..", ".env", ".secrets") or part.startswith(".env.") for part in path.parts):
        raise ValueError("expected a safe relative evidence path")
    resolved = (root / path).resolve(strict=True)
    if not resolved.is_relative_to(root.resolve()) or any(part in (".env", ".secrets") or part.startswith(".env.") for part in resolved.parts):
        raise ValueError("evidence path escapes its root or resolves to a protected name")
    if not resolved.is_file():
        raise ValueError("evidence must be a regular file")
    return resolved


def evidence(path, version):
    source = safe_file(ROOT, path)
    return {"path": path, "sha256": digest(source.read_bytes()), "version": version}


def read_truth(relative, kind):
    path = safe_file(ROOT, relative)
    data = path.read_bytes()
    truth = json.loads(data)
    if truth.get("schema_version") != 1 or truth.get("truth_set") != kind or not isinstance(truth.get("version"), str) or not truth["version"].strip():
        raise ValueError(f"invalid {kind} truth metadata")
    return truth, {"path": relative, "sha256": digest(data), "version": truth["version"]}


def atomic_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, prefix=path.name + ".", delete=False) as stream:
        temporary = Path(stream.name)
        try:
            json.dump(value, stream, sort_keys=True, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
            os.replace(temporary, path)
        finally:
            temporary.unlink(missing_ok=True)


class Membership:
    """Source positions, ignoring whitespace only; validate UTF-16 boundaries."""
    def __init__(self, text):
        self.visible = []
        self.boundaries = {0}
        offset = 0
        for char in text:
            width = len(char.encode("utf-16-le")) // 2
            if not char.isspace():
                self.visible.extend(range(offset, offset + width))
            offset += width
            self.boundaries.add(offset)

    def span(self, span):
        start, end = span.get("start"), span.get("end")
        if type(start) is not int or type(end) is not int or start >= end or start not in self.boundaries or end not in self.boundaries:
            raise ValueError("invalid UTF-16 source span")
        return start, end

    def signature(self, spans):
        if not isinstance(spans, list) or not spans:
            raise ValueError("entry requires source members")
        result, previous = [], -1
        for span in spans:
            start, end = self.span(span)
            if start < previous:
                raise ValueError("entry members must be ordered and disjoint")
            result.extend(self.visible[bisect_left(self.visible, start):bisect_left(self.visible, end)])
            previous = end
        if not result:
            raise ValueError("entry has no non-whitespace source content")
        return tuple(result)


def labels(entry):
    values = entry.get("field_labels", {})
    if not isinstance(values, dict):
        raise ValueError("field_labels must be an object")
    for field in FIELDS:
        value = values.get(field)
        if value is not None and (not isinstance(value, str) or not value.strip()):
            raise ValueError("known field labels must be nonempty strings")
        if field == "year" and value is not None and not re.fullmatch(r"(?:18|19|20)\d{2}[a-z]?", value):
            raise ValueError("invalid known publication year")
    return values


def predicted_fields(fields):
    authors = fields.get("authors", {}).get("value")
    return {"title": fields.get("title", {}).get("value"),
            "first_author": authors[0] if authors else None,
            "year": fields.get("year", {}).get("value")}


def normalize(field, value, title_keys):
    if value is None:
        return None
    if field == "title":
        return title_keys[value]
    if field == "year":
        match = re.fullmatch(r"((?:18|19|20)\d{2})[a-z]?", value)
        return match[1] if match else None
    return re.sub(r"[\W_]+", " ", unicodedata.normalize("NFKC", value).casefold()).strip()


def field_outcomes(expected, predicted, title_keys):
    output = {}
    for field in FIELDS:
        wanted = expected.get(field)
        actual = predicted.get(field)
        normalized_wanted = normalize(field, wanted, title_keys)
        normalized_actual = normalize(field, actual, title_keys)
        output[field] = {"known": wanted is not None, "expected": wanted, "predicted": actual,
                         "correct": bool(normalized_wanted) and normalized_actual == normalized_wanted}
    return output


def score_paper(index, paper, artifact, normalize_titles):
    membership = Membership(index["text"])
    truth_entries = paper["entries"]
    predicted = [entry for entry in artifact["objects"] if entry["kind"] == "bib_entry"]
    expected_ids = [entry["id"] for entry in truth_entries]
    predicted_ids = [entry["id"] for entry in predicted]
    if len(set(expected_ids)) != len(expected_ids) or len(set(predicted_ids)) != len(predicted_ids):
        raise ValueError("bibliography entry IDs must be unique")
    if any(not isinstance(identity, str) or not identity for identity in expected_ids + predicted_ids):
        raise ValueError("entry IDs must be nonempty strings")
    matches_by_span = defaultdict(deque)
    truth_signatures = {}
    claimed = set()
    for entry in truth_entries:
        signature = membership.signature(entry["spans"])
        if claimed.intersection(signature):
            raise ValueError("independent truth entries cannot overlap")
        claimed.update(signature)
        matches_by_span[signature].append(entry["id"])
        truth_signatures[entry["id"]] = digest(json.dumps(signature).encode())
        labels(entry)
    mapping, decisions = {}, []
    for entry in predicted:
        spans = entry["member_anchors"] or [entry["anchor"]]
        signature = membership.signature(spans)
        candidates = matches_by_span.get(signature)
        target = candidates.popleft() if candidates else None
        if target is not None:
            mapping[entry["id"]] = target
        decisions.append({"predicted_id": entry["id"], "truth_id": target, "spans": spans,
                          "signature_sha256": digest(json.dumps(signature).encode()), "fields": entry["bibliography"]})
    inverse = {target: identity for identity, target in mapping.items()}
    by_id = {entry["id"]: entry for entry in predicted}
    comparisons = []
    for entry in truth_entries:
        prediction = by_id.get(inverse.get(entry["id"]))
        comparisons.append((entry, predicted_fields(prediction["bibliography"]) if prediction else {}))
    titles = sorted({value for entry, fields in comparisons for value in (labels(entry).get("title"), fields.get("title")) if value is not None})
    title_keys = dict(zip(titles, normalize_titles(titles), strict=True))
    outcomes = [{"truth_id": entry["id"], "predicted_id": inverse.get(entry["id"]),
                 "fields": field_outcomes(labels(entry), fields, title_keys)} for entry, fields in comparisons]

    truth_pairs = Counter()
    for mention in paper["mentions"]:
        start, end = membership.span(mention)
        if mention["target"] not in expected_ids:
            raise ValueError("truth mention targets an unknown entry")
        truth_pairs[start, end, True, mention["target"]] += 1
    if any(count != 1 for count in truth_pairs.values()):
        raise ValueError("truth repeats an identical occurrence/target pair")
    page_fragments = defaultdict(Counter)
    for entry in predicted:
        for mention in entry["mentions"]:
            start, end = membership.span(mention["anchor"])
            page_fragments[start, end, entry["id"]][mention["anchor"]["page"]] += 1
    predicted_pairs = Counter()
    for (start, end, identity), pages in page_fragments.items():
        # One occurrence may have geometry on two pages. Duplicates on the same
        # page still count, rather than being silently discarded by set logic.
        predicted_pairs[start, end, identity in mapping, mapping.get(identity, identity)] += max(pages.values())
    correct = sum((truth_pairs & predicted_pairs).values())
    pair = lambda key, count: {"start": key[0], "end": key[1], "target": key[3] if key[2] else None,
                              "unmatched_prediction_id": None if key[2] else key[3], "count": count}
    return {"segmentation": {"true_positive": len(mapping), "false_positive": len(predicted) - len(mapping), "false_negative": len(truth_entries) - len(mapping)},
            "citations": {"true_positive": correct, "false_positive": sum(predicted_pairs.values()) - correct, "false_negative": sum(truth_pairs.values()) - correct},
            "entry_decisions": decisions, "truth_signatures": truth_signatures, "field_outcomes": outcomes,
            "missed_pairs": [pair(key, count) for key, count in sorted((truth_pairs - predicted_pairs).items())],
            "extra_pairs": [pair(key, count) for key, count in sorted((predicted_pairs - truth_pairs).items())]}


def summarize(papers, k2_cases):
    summary = {"segmentation": Counter(), "citations": Counter(), "fields": {kind: {field: Counter(known=0, excluded=0, correct=0) for field in FIELDS} for kind in ("K1", "K2")}}
    for paper in papers:
        for kind in ("segmentation", "citations"):
            summary[kind].update(paper[kind])
    for kind, cases in (("K1", [case for paper in papers for case in paper["field_outcomes"]]), ("K2", k2_cases)):
        for case in cases:
            for field, result in case["fields"].items():
                summary["fields"][kind][field]["known" if result["known"] else "excluded"] += 1
                summary["fields"][kind][field]["correct"] += int(result["correct"])
    return summary


def samples(summary):
    segmentation, citations = summary["segmentation"], summary["citations"]
    tp, fp, fn = (segmentation[key] for key in ("true_positive", "false_positive", "false_negative"))
    output = {"O8": ({"method": "f1", "true_positive": tp, "false_positive": fp, "false_negative": fn}, tp + fp + fn)}
    for metric, denominator in (("O10.precision", citations["true_positive"] + citations["false_positive"]), ("O10.recall", citations["true_positive"] + citations["false_negative"])):
        if denominator:
            output[metric] = ({"method": "ratio", "numerator": citations["true_positive"], "denominator": denominator}, denominator)
    for field in FIELDS:
        known = sum(summary["fields"][kind][field]["known"] for kind in ("K1", "K2"))
        correct = sum(summary["fields"][kind][field]["correct"] for kind in ("K1", "K2"))
        if all(summary["fields"][kind][field]["known"] > 0 for kind in ("K1", "K2")):
            output["O9." + field] = ({"method": "ratio", "numerator": correct, "denominator": known}, known)
    return output


class Backend:
    def __init__(self):
        environment = dict(os.environ, CARGO_BUILD_JOBS="1", CARGO_PROFILE_DEV_DEBUG="0", CARGO_PROFILE_TEST_DEBUG="0", CARGO_INCREMENTAL="0")
        subprocess.run(["cargo", "build", "--quiet", "--offline", "--example", "objects_fixture"], cwd=ROOT, env=environment, check=True)
        self.binary = ROOT / "target/debug/examples/objects_fixture"

    def run(self, request):
        output = subprocess.run([str(self.binary)], input=json.dumps(request), capture_output=True, text=True, check=True, timeout=60)
        return json.loads(output.stdout)

    def titles(self, titles):
        return self.run({"normalize_titles": titles})


def score_k2(truth, backend):
    cases = truth.get("cases")
    if not isinstance(cases, list) or not cases:
        raise ValueError("K2 must have real deposited cases")
    eligible = [case for case in cases if case.get("eligible_bibliography") is True and case.get("input", {}).get("kind") == "deposited_unstructured"]
    output = []
    seen = set()
    for case in eligible:
        identity = case.get("case_id")
        if not isinstance(identity, str) or not identity.strip() or identity in seen:
            raise ValueError("K2 eligible case IDs must be nonempty and unique")
        seen.add(identity)
        text = case["input"].get("text")
        if not isinstance(text, str) or not text.strip() or not case.get("snapshot_id") or not case.get("json_pointer"):
            raise ValueError("K2 eligible cases require deposited text and source provenance")
        expected = labels(case)
        fields = predicted_fields(backend.run({"parse_entries": [text]})[0])
        titles = sorted({value for value in (expected.get("title"), fields.get("title")) if value is not None})
        keys = dict(zip(titles, backend.titles(titles), strict=True))
        output.append({"case_id": identity, "snapshot_id": case["snapshot_id"], "json_pointer": case["json_pointer"],
                       "input_sha256": digest(text.encode()), "fields": field_outcomes(expected, fields, keys)})
    return output, {"eligible": len(eligible), "excluded_inputs": len(cases) - len(eligible)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--k1", required=True, help="repository-relative independent K1 bibliography labels")
    parser.add_argument("--k2", help="repository-relative deposited K2 reference labels")
    parser.add_argument("--index-root", required=True, type=Path, help="external corpus/cache root containing reading-index caches")
    args = parser.parse_args()
    started = time.monotonic()
    root = args.index_root.resolve(strict=True)
    permitted = (Path.home() / "Corpora", Path.home() / ".cache/lysilogy")
    if not any(root.is_relative_to(parent.resolve()) for parent in permitted):
        raise ValueError("index root must remain under ~/Corpora or ~/.cache/lysilogy")
    k1, k1_evidence = read_truth(args.k1, "K1")
    if k1.get("origin") != "arxiv-latex" or not k1.get("papers"):
        raise ValueError("real nonempty independent arXiv LaTeX truth is required")
    threshold = k1.get("alignment_threshold")
    if not isinstance(threshold, (int, float)) or not 0 < threshold <= 1:
        raise ValueError("invalid independently chosen alignment threshold")
    implementation = [evidence(path, VERSION) for path in SOURCES]
    truths = {"K1": k1_evidence}
    k2 = None
    if args.k2:
        k2, truths["K2"] = read_truth(args.k2, "K2")
    backend = Backend()
    observations, seen = [], set()
    for paper in k1["papers"]:
        identity = paper["arxiv_id"]
        if identity in seen or not re.fullmatch(r"(?:\d{4}\.\d{4,5}|[a-z-]+(?:\.[A-Z]{2})?/\d{7})(?:v\d+)?", identity):
            raise ValueError("K1 arXiv IDs must be valid and unique")
        seen.add(identity)
        if not re.fullmatch(r"[a-f\d]{16}", paper["paper_id"]):
            raise ValueError("invalid paper ID")
        alignment = paper["alignment"]
        if not isinstance(alignment.get("quality"), (int, float)) or not threshold <= alignment["quality"] <= 1 or not alignment.get("method"):
            raise ValueError("K1 paper does not meet its independent alignment policy")
        for field in ("pdf_sha256", "source_sha256"):
            if not re.fullmatch(r"[a-f\d]{64}", paper[field]):
                raise ValueError("K1 requires source PDF/archive hashes")
        reference = paper["index"]
        path = safe_file(root, reference["path"])
        data = path.read_bytes()
        if digest(data) != reference["sha256"]:
            raise ValueError("K1 reading-index fingerprint changed")
        index = json.loads(data)["index"]
        generation = '"' + digest(data) + '"'
        artifact = backend.run({"index": index, "paper_id": paper["paper_id"], "generation": generation})
        if artifact["reading_index_generation"] != generation or artifact["paper_id"] != paper["paper_id"] or digest(path.read_bytes()) != reference["sha256"]:
            raise ValueError("index generation changed during measurement")
        measured = score_paper(index, paper, artifact, backend.titles)
        measured.update({key: paper[key] for key in ("arxiv_id", "paper_id", "pdf_sha256", "source_sha256", "alignment", "index")})
        observations.append(measured)
    k2_cases, k2_coverage = score_k2(k2, backend) if k2 else ([], {"eligible": 0, "excluded_inputs": 0, "unavailable": True})
    summary = summarize(observations, k2_cases)
    if implementation != [evidence(path, VERSION) for path in SOURCES] or any(evidence(item["path"], item["version"]) != item for item in truths.values()):
        raise ValueError("implementation or truth changed during measurement")
    observed_path = "eval/inputs/evidence/bibliography-observations.json"
    atomic_json(ROOT / observed_path, {"schema_version": 1, "collector": VERSION, "papers": observations,
                "k2_cases": k2_cases, "k2_coverage": k2_coverage, "summary": summary, "model_calls": 0})
    observed = evidence(observed_path, VERSION)
    metrics = {metric: {"sample": sample, "cases": cases, "evidence": [observed]} for metric, (sample, cases) in samples(summary).items()}
    atomic_json(ROOT / "eval/inputs/bibliography/entries.json", {"schema_version": 1, "suite": "bibliography", "collector": VERSION,
                "implementation": implementation, "truth_sets": truths, "metrics": metrics, "cost_usd": 0, "wall_seconds": time.monotonic() - started})
    print(json.dumps({"summary": summary, "k2_coverage": k2_coverage}, sort_keys=True))


if __name__ == "__main__":
    main()
