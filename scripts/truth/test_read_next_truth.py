"""Generated toy graphs verify withholding mechanics, never read-next recall."""
import copy
import unittest

import read_next_truth as holdout
from reference_truth import TruthError


def fixture():
    def edge(source, target, record):
        return {"source": source, "target": target, "source_record": record}
    return {"nodes": [{"id": "A", "aliases": ["doi:10.1234/a", "arxiv:2001.00001v1"], "cohorts": ["scale"]},
                       {"id": "B", "aliases": ["doi:10.1234/b"], "cohorts": ["scale"]},
                       {"id": "C", "aliases": ["doi:10.1234/c"], "cohorts": ["local"]}],
            "views": [{"name": "bibliography", "edges": [edge("A", "B", "A/ref1"), edge("B", "C", "B/ref1")]},
                      {"name": "openalex", "edges": [edge("doi:10.1234/A", "B", "provider/A"), edge("C", "A", "provider/C")]},
                      {"name": "incoming-provider", "edges": [edge("arxiv:2001.00001v3", "B", "provider/B/citations/A")]}]}


class ReadNextTruthTests(unittest.TestCase):
    def should_exclude_held_out_own_edges_when_building_every_k7_feature_view(self):
        graph = holdout.normalize_graph(fixture())
        before = copy.deepcopy(graph)
        features = holdout.fold_features(graph, "https://doi.org/10.1234/a")
        self.assertEqual([], features["outgoing"]["A"])
        self.assertTrue(all(edge["source"] != "A" for edges in features["views"].values() for edge in edges))
        self.assertEqual([], features["incoming"]["B"])
        self.assertEqual({"out": 0, "in": 1}, features["degrees"]["A"])
        self.assertEqual(["C"], features["incoming"]["A"])
        self.assertEqual(["C"], features["outgoing"]["B"])
        self.assertEqual(before, graph)

    def should_preserve_labels_separately_when_fold_features_hide_positive_edges(self):
        graph = holdout.normalize_graph(fixture())
        plan = holdout.fold_plan(graph)
        fold = next(fold for fold in plan["folds"] if fold["query"] == "A")
        self.assertEqual(["B"], fold["expected_targets"])
        features = holdout.features_for_fold(graph, fold)
        self.assertNotIn("expected_targets", features)
        self.assertNotIn("source_record", str(features))
        self.assertEqual([], features["outgoing"]["A"])
        self.assertEqual(2, plan["coverage"]["eligible_folds"])

    def should_reject_cached_feature_inputs_when_observations_include_unfiltered_derived_values(self):
        observed = fixture()
        observed["nodes"][0]["outdegree"] = 1
        with self.assertRaisesRegex(TruthError, "precomputed features"):
            holdout.normalize_graph(observed)
        observed = fixture()
        observed["raw_openalex_payload"] = {"referenced_works": ["B"]}
        with self.assertRaisesRegex(TruthError, "only nodes"):
            holdout.normalize_graph(observed)

    def should_keep_fold_order_deterministic_when_input_node_and_view_orders_change(self):
        first = fixture()
        second = copy.deepcopy(first)
        second["nodes"].reverse()
        second["views"].reverse()
        for view in second["views"]:
            view["edges"].reverse()
        self.assertEqual(holdout.normalize_graph(first), holdout.normalize_graph(second))
        self.assertEqual(holdout.fold_plan(holdout.normalize_graph(first)), holdout.fold_plan(holdout.normalize_graph(second)))

    def should_report_missing_candidate_coverage_when_a_bibliography_target_is_not_independently_mapped(self):
        observed = fixture()
        observed["views"][0]["edges"].append({"source": "A", "target": "doi:10.1234/unknown", "source_record": "A/ref2"})
        graph = holdout.normalize_graph(observed)
        self.assertEqual(1, graph["coverage"]["bibliography"]["outside_candidate_universe"])
        self.assertEqual(3, len(graph["nodes"]))
        self.assertNotIn("doi:10.1234/unknown", str(holdout.fold_features(graph, "A")))

    def should_refuse_ambiguous_aliases_when_two_candidate_nodes_claim_the_same_paper(self):
        observed = fixture()
        observed["nodes"][1]["aliases"].append("arxiv:2001.00001v2")
        with self.assertRaisesRegex(TruthError, "Ambiguous graph alias"):
            holdout.normalize_graph(observed)

    def should_withhold_legacy_arxiv_versions_when_local_papers_predate_modern_ids(self):
        observed = fixture()
        observed["nodes"][0]["aliases"].append("arxiv:math.GT/0309136v1")
        observed["views"][1]["edges"].append({"source": "arxiv:math.GT/0309136v2", "target": "C", "source_record": "legacy/query"})
        features = holdout.fold_features(holdout.normalize_graph(observed), "arxiv:math.GT/0309136")
        self.assertEqual([], features["outgoing"]["A"])
        for invalid in ["arxiv:math.GT/0309136v0", "arxiv:2001.00001v0", "opaque\x00name"]:
            with self.subTest(invalid=invalid), self.assertRaises(TruthError):
                holdout.alias_key(invalid)

    def should_refuse_changed_graphs_when_a_fold_uses_a_different_frozen_input(self):
        graph = holdout.normalize_graph(fixture())
        fold = holdout.fold_plan(graph)["folds"][0]
        graph["views"]["openalex"].clear()
        with self.assertRaisesRegex(TruthError, "fingerprint"):
            holdout.features_for_fold(graph, fold)
