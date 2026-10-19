"""Pin the spike scorer, so a scorer change cannot silently move the numbers.

Run from the repository root: python3 -B -m unittest scripts/spike/test_benchmark.py
"""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import benchmark as b  # noqa: E402

ORACLE = {
    "title": "Deep Résidual Learning for Image Recognition",
    "authors": ["Kaiming He", "Xiangyu Zhang", "Shaoqing Ren"],
    "abstract": "We present a residual learning framework. It eases training of deep networks.",
}

INVENTORY = {
    "figures": [{"number": "1", "page": 2}, {"number": "2", "page": 4}],
    "tables": [{"number": "I", "page": 5}],
    "algorithms": [],
    "equations": [{"number": "1", "page": 3}, {"number": "2", "page": 3}, {"number": "A.1", "page": 9}],
}


def artifacts(**overrides):
    value = {
        "identity": {
            "title": "Deep Residual Learning for Image Recognition",
            "authors": [{"name": "K. He"}, {"name": "Xiangyu Zhang"}, {"name": "Shaoqing Ren"}],
        },
        "abstract": {"text": "We present a residual learning framework. It eases training of deep networks."},
        "objects": [
            {"kind": "figure", "label": "Figure 1", "number": "1", "page": 2},
            {"kind": "figure", "label": "Figure 2", "number": "2", "page": 5},
            {"kind": "table", "label": "Table I", "number": None, "page": 5},
            {"kind": "equation", "label": "(1)", "number": "1", "page": 3},
            {"kind": "equation", "label": "(A.1)", "number": "A.1", "page": 9},
            {"kind": "equation", "label": "(7)", "number": "7", "page": 3},
            {"kind": "equation", "label": None, "number": None, "page": 4},
        ],
    }
    value.update(overrides)
    return value


class IdentityTests(unittest.TestCase):
    def test_should_match_title_ignoring_accents_case_and_punctuation(self):
        self.assertTrue(b.score_identity(artifacts(), ORACLE)["title_exact"])

    def test_should_accept_an_initial_for_a_given_name_but_require_order(self):
        self.assertTrue(b.score_identity(artifacts(), ORACLE)["authors_ordered_exact"])
        swapped = artifacts(identity={"title": ORACLE["title"], "authors": [{"name": "Xiangyu Zhang"}, {"name": "Kaiming He"}, {"name": "Shaoqing Ren"}]})
        score = b.score_identity(swapped, ORACLE)
        self.assertFalse(score["authors_ordered_exact"])
        self.assertEqual(score["authors_f1"], 1.0)

    def test_should_score_a_missing_abstract_as_zero_not_an_error(self):
        score = b.score_identity(artifacts(abstract={"text": None}), ORACLE)
        self.assertEqual(score["abstract_f1"], 0.0)
        self.assertFalse(score["abstract_match"])

    def test_should_score_a_failed_paper_with_no_artifacts(self):
        score = b.score_identity(None, ORACLE)
        self.assertFalse(score["title_exact"])
        self.assertEqual(score["authors_count_error"], -3)

    def test_should_skip_fields_the_oracle_withholds(self):
        self.assertEqual(b.score_identity(artifacts(), {"title": None, "authors": None, "abstract": None}), {})


class ObjectTests(unittest.TestCase):
    def setUp(self):
        self.score = b.score_objects(artifacts(), INVENTORY)

    def test_should_read_the_number_from_the_label_when_number_is_missing(self):
        table = self.score["table"]
        self.assertEqual((table["matched"], table["recall"]), (1, 1.0))

    def test_should_count_page_errors_only_among_matched_objects(self):
        figure = self.score["figure"]
        self.assertEqual((figure["matched"], figure["page_correct"]), (2, 1))

    def test_should_ignore_unnumbered_equations_and_count_extras(self):
        equation = self.score["equation"]
        self.assertEqual(equation["predicted"], 4)
        self.assertEqual(equation["predicted_numbered"], 3)
        self.assertEqual(equation["matched"], 2)
        self.assertEqual(equation["missing"], ["2"])
        self.assertEqual(equation["extra"], ["7"])
        self.assertEqual(equation["count_error"], 0)

    def test_should_give_full_marks_for_an_empty_kind_with_no_predictions(self):
        algorithm = self.score["algorithm"]
        self.assertEqual((algorithm["precision"], algorithm["recall"]), (1.0, 1.0))


class AggregateTests(unittest.TestCase):
    def test_should_report_rates_and_totals_over_papers(self):
        record = {"artifacts": artifacts(), "pass1": artifacts(), "pages": 9, "totals": {"cost_usd": 0.05, "wall_s": 100.0},
                  "calls": [{"stage": "extract.pass1", "prompt_tokens": 1, "completion_tokens": 2, "reasoning_tokens": 0,
                             "cost_usd": 0.05, "latency_s": 9.0, "finish_reason": "stop", "cost_source": "reported"}]}
        paper = {"stem": "x"}
        rows = [b.score_paper(paper, record, ORACLE, INVENTORY, 10), {"stem": "y", "succeeded": False, "failures": [{"stage": "run", "error": "no result file"}]}]
        summary = b.aggregate(rows)
        self.assertEqual(summary["papers"], 2)
        self.assertEqual(summary["succeeded"], 1)
        self.assertEqual(summary["identity_final"]["title_exact"], "1/1")
        self.assertEqual(summary["objects_final"]["equation"]["matched"], 2)
        self.assertEqual(summary["cost_usd"]["total"], 0.05)
        self.assertEqual(summary["failures"], [("y", "run", "no result file")])


if __name__ == "__main__":
    unittest.main()
