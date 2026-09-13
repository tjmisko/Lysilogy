"""Synthetic arithmetic/provenance regressions; never written as K1/K2 truth."""
import copy
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import bibliography as collector


def utf16(value):
    return len(value.encode("utf-16-le")) // 2


def fixture():
    body = "😀 See [1] and [2]."
    first, second = "[1] Smith. First study. 2020.", "[2] Jones. Second study. 2021."
    text = body + "\n\n" + first + "\n\n" + second
    one = {"start": utf16(body) + 2, "end": utf16(body) + 2 + len(first)}
    two = {"start": one["end"] + 2, "end": utf16(text)}
    marker = lambda label: {"start": utf16(body[:body.index(label)]), "end": utf16(body[:body.index(label)]) + len(label)}
    mentions = [{**marker("[1]"), "target": "independent-a"}, {**marker("[2]"), "target": "independent-b"}]
    truth = {"entries": [
        {"id": "independent-a", "spans": [one], "field_labels": {"title": "First study", "first_author": "Smith", "year": "2020"}},
        {"id": "independent-b", "spans": [two], "field_labels": {"title": "Second study", "first_author": None, "year": "2021"}},
    ], "mentions": mentions}
    objects = []
    for number, span, name, title, year in [(1, one, "Smith", "First study", "2020"), (2, two, "Jones", "Second study", "2021")]:
        span = copy.deepcopy(span)
        objects.append({"kind": "bib_entry", "id": f"bib-{number}", "anchor": span, "member_anchors": [span],
                        "bibliography": {"title": {"value": title}, "authors": {"value": [name]}, "year": {"value": year}},
                        "mentions": [{"anchor": {**marker(f"[{number}]"), "page": 1}}]})
    return {"text": text}, truth, {"objects": objects}


def score(index, truth, artifact):
    # Expected/actual titles in these arithmetic fixtures are already identical
    # presentation strings; normalization itself is covered by Rust title tests.
    return collector.score_paper(index, truth, artifact, lambda titles: titles)


class BibliographyCollectorTests(unittest.TestCase):
    def test_should_execute_cargos_reported_artifact_when_a_custom_target_has_a_stale_default_binary(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            stale = root / "target/debug/examples/objects_fixture"
            stale.parent.mkdir(parents=True)
            stale.write_bytes(b"stale default executable")
            executable = root / "custom-target/debug/examples/objects_fixture"
            executable.parent.mkdir(parents=True)
            executable.write_bytes(b"current Cargo executable")
            artifact = {"reason": "compiler-artifact", "executable": str(executable),
                        "target": {"name": "objects_fixture", "kind": ["example"],
                                   "src_path": str(root / "examples/objects_fixture.rs")}}

            def run(command, **kwargs):
                if command[0] == "cargo":
                    self.assertIn("--message-format=json", command)
                    self.assertEqual(kwargs["env"]["CARGO_TARGET_DIR"], str(root / "custom-target"))
                    return subprocess.CompletedProcess(command, 0, stdout=json.dumps(artifact))
                self.assertEqual(command, [str(executable)])
                return subprocess.CompletedProcess(command, 0, stdout="[]")

            with patch.object(collector, "ROOT", root), patch.object(collector.subprocess, "run", side_effect=run), patch.dict(collector.os.environ, CARGO_TARGET_DIR=str(root / "custom-target")):
                backend = collector.Backend()
                self.assertEqual(backend.run({"normalize_titles": []}), [])
                self.assertEqual(backend.verify()["executable_sha256"], collector.digest(executable.read_bytes()))
                self.assertNotEqual(backend.executable_sha256, collector.digest(stale.read_bytes()))
                executable.write_bytes(b"replaced after measurement")
                with self.assertRaisesRegex(ValueError, "executable changed"):
                    backend.verify()

    def test_should_penalize_known_fields_when_segmentation_misses_a_truth_entry(self):
        index, truth, artifact = fixture()
        artifact["objects"].pop()
        measured = score(index, truth, artifact)
        self.assertEqual(measured["segmentation"], {"true_positive": 1, "false_positive": 0, "false_negative": 1})
        summary = collector.summarize([measured], [])
        self.assertEqual(summary["fields"]["K1"]["title"], {"known": 2, "excluded": 0, "correct": 1})
        self.assertEqual(summary["fields"]["K1"]["first_author"], {"known": 1, "excluded": 1, "correct": 1})
        self.assertNotIn("O9.title", collector.samples(summary))  # K2 is still unavailable.
        self.assertEqual(collector.samples(summary)["O10.recall"][0]["denominator"], 2)

    def test_should_count_wrong_and_duplicate_targets_when_occurrences_are_resolved_incorrectly(self):
        index, truth, artifact = fixture()
        second = artifact["objects"][1]["mentions"].pop()
        artifact["objects"][0]["mentions"].append(second)
        artifact["objects"][0]["mentions"].append(copy.deepcopy(artifact["objects"][0]["mentions"][0]))
        measured = score(index, truth, artifact)
        self.assertEqual(measured["citations"], {"true_positive": 1, "false_positive": 2, "false_negative": 1})
        self.assertEqual(sum(pair["count"] for pair in measured["extra_pairs"]), 2)
        self.assertEqual(measured["missed_pairs"][0]["target"], "independent-b")

    def test_should_match_one_to_one_when_duplicate_predictions_cover_the_same_source(self):
        index, truth, artifact = fixture()
        duplicate = copy.deepcopy(artifact["objects"][0])
        duplicate["id"] = "bib-duplicate"
        artifact["objects"].append(duplicate)
        measured = score(index, truth, artifact)
        self.assertEqual(measured["segmentation"], {"true_positive": 2, "false_positive": 1, "false_negative": 0})
        self.assertEqual(measured["citations"]["false_positive"], 1)
        self.assertEqual(measured["entry_decisions"][2]["truth_id"], None)

    def test_should_keep_id_namespaces_distinct_when_an_unmatched_prediction_uses_a_truth_id(self):
        index, truth, artifact = fixture()
        artifact["objects"][0]["id"] = "independent-a"
        artifact["objects"][0]["member_anchors"][0]["end"] -= 1
        measured = score(index, truth, artifact)
        self.assertEqual(measured["citations"], {"true_positive": 1, "false_positive": 1, "false_negative": 1})
        self.assertEqual(measured["extra_pairs"][0]["target"], None)
        self.assertEqual(measured["extra_pairs"][0]["unmatched_prediction_id"], "independent-a")

    def test_should_ignore_only_whitespace_when_comparing_entry_membership(self):
        index, truth, artifact = fixture()
        artifact["objects"][0]["member_anchors"][0]["start"] -= 1
        self.assertEqual(score(index, truth, artifact)["segmentation"]["true_positive"], 2)
        artifact["objects"][0]["member_anchors"][0]["end"] -= 1  # The final period is source content.
        self.assertEqual(score(index, truth, artifact)["segmentation"], {"true_positive": 1, "false_positive": 1, "false_negative": 1})
        with self.assertRaisesRegex(ValueError, "UTF-16"):
            collector.Membership("😀 citation").signature([{"start": 1, "end": 4}])

    def test_should_collapse_page_fragments_when_one_occurrence_crosses_pages(self):
        index, truth, artifact = fixture()
        fragment = copy.deepcopy(artifact["objects"][0]["mentions"][0])
        fragment["anchor"]["page"] = 2
        artifact["objects"][0]["mentions"].append(fragment)
        self.assertEqual(score(index, truth, artifact)["citations"], {"true_positive": 2, "false_positive": 0, "false_negative": 0})

    def test_should_report_unavailable_precision_when_all_true_mentions_are_unresolved(self):
        index, truth, artifact = fixture()
        for entry in artifact["objects"]:
            entry["mentions"] = []
        summary = collector.summarize([score(index, truth, artifact)], [])
        metrics = collector.samples(summary)
        self.assertNotIn("O10.precision", metrics)
        self.assertEqual(metrics["O10.recall"][0], {"method": "ratio", "numerator": 0, "denominator": 2})

    def test_should_exclude_answer_derived_inputs_when_k2_has_structured_only_references(self):
        truth = {"cases": [{"case_id": "c1", "eligible_bibliography": True, "input": {"kind": "deposited_structured", "text": "Rendered from the labels."}}]}
        cases, coverage = collector.score_k2(truth, None)
        self.assertEqual(cases, [])
        self.assertEqual(coverage, {"eligible": 0, "excluded_inputs": 1})

    def test_should_retain_deposited_identity_when_k2_cases_supply_independent_labels(self):
        class Backend:
            def run(self, request):
                self.request = request
                return [{"title": {"value": "A study"}, "authors": {"value": ["Smith"]}, "year": {"value": "2020a"}}]

            def titles(self, values):
                return values

        case = {"case_id": "deposited-reference-12", "eligible_bibliography": True,
                "input": {"kind": "deposited_unstructured", "text": "Smith. A study. 2020a."},
                "snapshot_id": "crossref-snapshot", "json_pointer": "/message/reference/12",
                "field_labels": {"title": "A study", "first_author": "Smith", "year": "2020a"}}
        backend = Backend()
        cases, coverage = collector.score_k2({"cases": [case]}, backend)
        self.assertEqual(cases[0]["case_id"], case["case_id"])
        self.assertEqual(cases[0]["snapshot_id"], case["snapshot_id"])
        self.assertEqual(cases[0]["json_pointer"], case["json_pointer"])
        self.assertTrue(all(field["correct"] for field in cases[0]["fields"].values()))
        self.assertEqual(backend.request, {"parse_entries": [case["input"]["text"]]})
        self.assertEqual(coverage, {"eligible": 1, "excluded_inputs": 0})
        for invalid in (None, "", " "):
            with self.assertRaisesRegex(ValueError, "case IDs"):
                collector.score_k2({"cases": [{**case, "case_id": invalid}]}, backend)
        with self.assertRaisesRegex(ValueError, "case IDs"):
            collector.score_k2({"cases": [case, copy.deepcopy(case)]}, backend)

    def test_should_require_known_labels_from_both_truth_sets_when_publishing_field_accuracy(self):
        index, truth, artifact = fixture()
        artifact["objects"].pop()
        outcome = {"fields": {"title": {"known": True, "correct": True}, "year": {"known": True, "correct": False}, "first_author": {"known": False, "correct": False}}}
        metrics = collector.samples(collector.summarize([score(index, truth, artifact)], [outcome]))
        self.assertEqual(metrics["O9.title"][0], {"method": "ratio", "numerator": 2, "denominator": 3})
        self.assertEqual(metrics["O9.year"][0], {"method": "ratio", "numerator": 1, "denominator": 3})
        self.assertNotIn("O9.first_author", metrics)

    def test_should_not_credit_empty_normalizations_when_fields_have_only_punctuation(self):
        fields = {"title": "---", "first_author": "...", "year": None}
        outcomes = collector.field_outcomes(fields, fields, {"---": ""})
        self.assertTrue(outcomes["title"]["known"])
        self.assertFalse(any(field["correct"] for field in outcomes.values()))

    def test_should_reject_bad_truth_when_ids_targets_or_field_labels_are_invalid(self):
        for change in (lambda truth: truth["entries"][1].update(id="independent-a"),
                       lambda truth: truth["mentions"][0].update(target="not-an-entry"),
                       lambda truth: truth["entries"][0]["field_labels"].update(title=""),
                       lambda truth: truth["mentions"].append(copy.deepcopy(truth["mentions"][0]))):
            index, truth, artifact = fixture()
            change(truth)
            with self.assertRaises(ValueError):
                score(index, truth, artifact)

    def test_should_reject_escape_and_protected_paths_when_reading_external_index_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            root = parent / "allowed"
            root.mkdir()
            (parent / "outside.json").write_text("{}")
            (root / "link.json").symlink_to(parent / "outside.json")
            for relative in ("../outside.json", "link.json", ".env", ".secrets/value.json", "/absolute.json"):
                with self.assertRaises(ValueError):
                    collector.safe_file(root, relative)


if __name__ == "__main__":
    unittest.main()
