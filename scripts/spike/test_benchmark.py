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

    def test_should_fold_dotless_i_and_ligature_letters(self):
        self.assertEqual(b.name_key("Jan Kr\u0131\u0301z\u030c"), b.name_key("Jan K\u0159\u00ed\u017e"))
        self.assertEqual(b.fold("Tworzyd\u0142o"), "tworzydlo")

    def test_should_match_a_small_caps_surname_with_a_dropped_capital(self):
        oracle = {**ORACLE, "authors": ["Martin Brossard", "Silv\u00e8re Bonnabel"]}
        identity = {"title": ORACLE["title"], "authors": [{"name": "Martin B ROSSARD"}, {"name": "Silv\u00e8re B ONNABEL"}]}
        self.assertTrue(b.score_identity(artifacts(identity=identity), oracle)["authors_ordered_exact"])

    def test_should_treat_author_order_as_a_set_and_allow_a_near_name(self):
        swapped = artifacts(identity={"title": ORACLE["title"], "authors": [{"name": "Xiangyu Zhang"}, {"name": "Kaiming He"}, {"name": "Shaoqing Ren"}]})
        score = b.score_identity(swapped, ORACLE)
        self.assertFalse(score["authors_ordered_exact"])
        self.assertTrue(score["authors_set_exact"])
        self.assertTrue(score["authors_matchable"])
        typo = artifacts(identity={"title": ORACLE["title"], "authors": [{"name": "Kaiming He"}, {"name": "Xiangyu Zbang"}, {"name": "Shaoqing Ren"}]})
        score = b.score_identity(typo, ORACLE)
        self.assertFalse(score["authors_set_exact"])
        self.assertTrue(score["authors_matchable"])

    def test_should_separate_exact_from_perfect_enough_identity(self):
        oracle = {"title": ORACLE["title"], "authors": None, "abstract": None}
        near = artifacts(identity={"title": "Deep Residual Learning for Image Recognitionn", "authors": []}, abstract={"text": None})
        score = b.score_identity(near, oracle)
        self.assertFalse(score["title_exact"])
        self.assertTrue(score["title_matchable"])
        self.assertEqual(b.identity_resolvable(score), (False, True))

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


class ReasoningTests(unittest.TestCase):
    def test_should_sum_reasoning_tokens_per_stage(self):
        record = {
            "calls": [
                {"stage": "extract.pass1", "reasoning_tokens": 100, "completion_tokens": 200},
                {"stage": "extract.pass1", "reasoning_tokens": 50, "completion_tokens": 100},
                {"stage": "extract.pass2", "reasoning_tokens": 30, "completion_tokens": 60},
            ]
        }
        profile = b.reasoning_profile(record)
        self.assertEqual(profile["tokens"], 180)
        self.assertEqual(profile["by_stage"], {"extract.pass1": 150, "extract.pass2": 30})
        self.assertEqual(profile["share"], 0.5)

    def test_should_flag_a_stage_that_spends_far_more_reasoning_than_the_cohort(self):
        def row(stem, tokens):
            return {"stem": stem, "identity": {}, "reasoning_profile": {"tokens": tokens, "by_stage": {"extract.pass1": tokens}}}

        rows = [row("a", 10_000), row("b", 11_000), row("c", 40_000)]
        medians = b.mark_confused(rows)
        self.assertEqual(medians["extract.pass1"], 11_000)
        self.assertTrue(rows[2]["confused"])
        self.assertFalse(rows[0]["confused"])

    def test_should_not_flag_a_stage_below_the_floor_even_when_it_dominates(self):
        rows = [{"stem": "a", "identity": {}, "reasoning_profile": {"tokens": 100, "by_stage": {"extract.repair": 100}}}]
        b.mark_confused(rows)
        self.assertFalse(rows[0]["confused"])

    def test_should_leave_a_failed_paper_without_a_confusion_judgment(self):
        rows = [{"stem": "y", "succeeded": False, "failures": []}]
        b.mark_confused(rows)
        self.assertIsNone(rows[0]["confused"])


class SectionTests(unittest.TestCase):
    def test_should_measure_coverage_and_flag_malformed_units(self):
        artifacts = {
            "sections": [
                {"title": "A", "pages": {"start": 1, "end": 3}},
                {"title": "B", "page": 4},
            ]
        }
        score = b.score_sections(artifacts, 10)
        self.assertEqual(score["coverage"], 0.3)
        self.assertEqual(score["malformed"], 1)
        self.assertEqual(score["count"], 2)

    def test_should_report_zero_coverage_when_there_are_no_sections(self):
        self.assertEqual(b.score_sections({"sections": []}, 5)["coverage"], 0.0)

    def test_should_align_units_to_the_audited_spans(self):
        oracle = {"sections": [{"pages": {"start": 1, "end": 2}}, {"pages": {"start": 3, "end": 4}}]}
        exact = {"sections": [{"pages": {"start": 1, "end": 2}}, {"pages": {"start": 3, "end": 4}}]}
        score = b.score_sections_oracle(exact, oracle)
        self.assertEqual(score["unit_iou"], 1.0)
        self.assertEqual(score["boundary_exact"], 1.0)
        self.assertEqual(score["unit_count_error"], 0)
        shifted = {"sections": [{"pages": {"start": 2, "end": 3}}]}
        score = b.score_sections_oracle(shifted, oracle)
        self.assertLess(score["unit_iou"], 1.0)
        self.assertEqual(score["boundary_exact"], 0.0)
        self.assertEqual(score["unit_count_error"], -1)


class IntervalTests(unittest.TestCase):
    def test_should_bound_a_perfect_rate_below_one(self):
        interval = b.wilson(20, 20)
        self.assertEqual(interval["rate"], 1.0)
        self.assertLess(interval["lower"], 1.0)

    def test_should_return_none_for_no_papers(self):
        self.assertIsNone(b.wilson(0, 0))


if __name__ == "__main__":
    unittest.main()
